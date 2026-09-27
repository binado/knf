//! `${key.path}` and `${env:VAR}` resolution over the merged native values.
//!
//! One pass over a merged document, replacing references in string values. Keys
//! are never interpolated; values only.
//!
//! Two positions, and the distinction is the whole design:
//!
//! - **whole string** — `port = "${p}"` takes the referent's value *and type*,
//!   so the output is a number. A reference to a container is allowed here, and
//!   aliases the (fully resolved) subtree.
//! - **embedded** — `url = "http://${host}:${p}/"` stringifies. A container has
//!   no format-independent spelling there, so it is an error in v1.
//!
//! `$$` is a literal `$`. A `$` followed by anything but `$` or `{` is ordinary
//! text.
//!
//! **No `std::env` here.** The environment is injected through [`Env`], so this
//! module is deterministic and testable without touching process state. It is
//! also what keeps format-specific typing out of resolution: the native adapter
//! parses whole-string environment values. [`ProcessEnv`](crate::ProcessEnv) is the
//! one implementation that reads the real environment.

mod error;
mod scan;

use std::collections::HashMap;

use crate::path::lookup;
use crate::{ConfigFormat, ConfigObject, PathError, RefPath, Seg};

pub use error::{Cycle, InterpError, Problem};
pub use scan::Syntax;

use scan::{Piece, Spelled, scan};

/// The one namespace. Matched as a literal **prefix**, not by splitting on the
/// first `:`, so `${a:b}` is the ordinary key `a:b` and `${db.host:port}` does
/// not produce a baffling "unknown namespace `db.host`". The only unaddressable
/// keys are those literally beginning `env:` — the same class of limitation the
/// dotted-path flags already carry.
const ENV: &str = "env:";

/// Where `${env:NAME}` reads from.
///
/// A trait rather than a direct `std::env::var` call so that resolution never
/// touches process state; [`ProcessEnv`](crate::ProcessEnv) is the one
/// implementation that does.
pub trait Env {
    /// The variable, or `None` if it is unset.
    fn lookup(&self, name: &str) -> Option<String>;
}

/// Resolves every reference in `doc`.
///
/// By value because resolution builds a new tree rather than editing in place —
/// a referent must be read in its pre-substitution form no matter which order
/// the document is walked in.
///
/// Call it once, on the merged document, never per layer: a reference reads the
/// document the caller is actually going to get. Overlays therefore interpolate
/// like any other layer, and strict mode has already run — it compares the
/// types values had when they were *written*, so a `"${port}"` was a string when
/// it looked.
///
/// Every unresolved reference and every malformed one is collected, so a run
/// reports all of them. A cycle is the exception and returns alone: there is
/// nothing meaningful to continue past.
pub fn interpolate<V: ConfigFormat>(doc: V, env: &dyn Env) -> Result<V, InterpError> {
    let mut resolver = Resolver {
        doc: &doc,
        env,
        memo: HashMap::new(),
        visiting: Vec::new(),
        problems: Vec::new(),
    };
    // Resolving the root path resolves the document: the recursion is the same
    // one references use, so transitivity and cycle detection come for free.
    let resolved = resolver.resolve(&[]).map_err(InterpError::Cycle)?;
    if resolver.problems.is_empty() {
        Ok(resolved)
    } else {
        Err(InterpError::Problems(resolver.problems))
    }
}

/// Memoized depth-first resolution, keyed on path.
///
/// One table buys three things at once: transitivity (a referent is resolved
/// before it is spliced), order-independence (which key is reached first decides
/// who does the work, never what the answer is), and — since `resolve_value`
/// runs at most once per path — reporting each problem exactly once however many
/// references point at it.
struct Resolver<'a, V: ConfigFormat> {
    doc: &'a V,
    env: &'a dyn Env,
    memo: HashMap<Vec<Seg>, V>,
    /// The paths currently being resolved, innermost last. Doubles as the cycle
    /// chain: the slice from a repeated path to the top *is* the loop.
    visiting: Vec<Vec<Seg>>,
    problems: Vec<Problem>,
}

impl<'a, V: ConfigFormat> Resolver<'a, V> {
    /// Resolves the node at `path`, which the caller has established exists.
    fn resolve(&mut self, path: &[Seg]) -> Result<V, Cycle> {
        if let Some(done) = self.memo.get(path) {
            return Ok(done.clone());
        }
        if let Some(start) = self
            .visiting
            .iter()
            .position(|seen| seen.as_slice() == path)
        {
            let mut chain = self.visiting[start..].to_vec();
            chain.push(path.to_vec());
            return Err(Cycle::new(chain));
        }

        // Copied out of `self` so the raw tree stays readable while `self` is
        // borrowed mutably below.
        let raw = lookup(self.doc, path).expect("resolve is only called on paths that exist");

        self.visiting.push(path.to_vec());
        let resolved = self.resolve_value(raw, path)?;
        self.visiting.pop();

        // The root is skipped: no reference can name it (a `RefPath` is never
        // empty) and it is resolved exactly once, so caching it would only
        // clone the whole document for nobody.
        if !path.is_empty() {
            self.memo.insert(path.to_vec(), resolved.clone());
        }
        Ok(resolved)
    }

    fn resolve_value(&mut self, raw: &'a V, path: &[Seg]) -> Result<V, Cycle> {
        if let Some(text) = raw.as_str() {
            return self.resolve_string(text, path);
        }
        if let Some(items) = raw.as_array() {
            let mut out = Vec::with_capacity(items.len());
            for index in 0..items.len() {
                out.push(self.resolve(&child(path, Seg::Index(index)))?);
            }
            return Ok(V::array(out));
        }
        if let Some(map) = raw.as_object() {
            let mut out = V::Object::new();
            for (key, _) in map.iter() {
                let value = self.resolve(&child(path, Seg::Key(key.clone())))?;
                out.insert(key.clone(), value);
            }
            return Ok(V::object(out));
        }
        Ok(raw.clone())
    }

    fn resolve_string(&mut self, text: &str, path: &[Seg]) -> Result<V, Cycle> {
        let pieces = scan(text);

        // No `$` anywhere — the common case, and the reason `scan` reports it
        // as emptiness rather than a list of one literal.
        if pieces.is_empty() {
            return Ok(V::string(text.to_string()));
        }
        if let [Piece::Ref(body)] = pieces.as_slice() {
            return self.substitute(body, path);
        }

        let mut out = String::new();
        for piece in pieces {
            match piece {
                Piece::Literal(literal) => out.push_str(literal),
                Piece::Ref(body) => out.push_str(&self.splice(body, path)?),
                Piece::Malformed { spelling, error } => {
                    self.problems.push(Problem::Syntax {
                        path: path.to_vec(),
                        error,
                    });
                    out.push_str(spelling);
                }
            }
        }
        Ok(V::string(out))
    }

    /// Whole-string position: the reference *is* the value, so it takes the
    /// referent's type. Containers are allowed here.
    fn substitute(&mut self, body: &str, path: &[Seg]) -> Result<V, Cycle> {
        if let Some(name) = body.strip_prefix(ENV) {
            return Ok(match self.env_value(name, body, path) {
                // Environment values are terminal: never re-scanned, so a
                // variable holding `${x}` cannot reach back into the document.
                Some(found) => V::parse_inline(found),
                None => V::string(Spelled(body).to_string()),
            });
        }
        match self.target(body, path) {
            Some(target) => self.resolve(&target),
            None => Ok(V::string(Spelled(body).to_string())),
        }
    }

    /// Embedded position: the reference joins surrounding text, so it renders.
    fn splice(&mut self, body: &str, path: &[Seg]) -> Result<String, Cycle> {
        if let Some(name) = body.strip_prefix(ENV) {
            return Ok(match self.env_value(name, body, path) {
                // Raw, not re-rendered: a variable is text already, and parsing
                // it only to print it again could only lose something.
                Some(found) => found,
                None => Spelled(body).to_string(),
            });
        }
        let Some(target) = self.target(body, path) else {
            return Ok(Spelled(body).to_string());
        };
        let value = self.resolve(&target)?;
        Ok(match value.stringify() {
            Some(text) => text,
            None => {
                self.problems.push(Problem::NotStringifiable {
                    path: path.to_vec(),
                    reference: body.to_string(),
                    kind: value.kind(),
                });
                Spelled(body).to_string()
            }
        })
    }

    /// The variable, recording a problem and returning `None` if the name is
    /// empty or the variable is unset.
    fn env_value(&mut self, name: &str, body: &str, path: &[Seg]) -> Option<String> {
        if name.is_empty() {
            self.problems.push(Problem::Syntax {
                path: path.to_vec(),
                error: Syntax::EmptyEnvName,
            });
            return None;
        }
        let found = self.env.lookup(name);
        if found.is_none() {
            self.problems.push(Problem::Unresolved {
                path: path.to_vec(),
                reference: body.to_string(),
            });
        }
        found
    }

    /// The document path a reference names, recording a problem and returning
    /// `None` if it is malformed or names nothing.
    ///
    /// A reference may *read* an array element — `${servers[0]}` parses through
    /// the one `RefPath` spelling, where write-side callers run
    /// `try_into_keys` to reject indices instead — and memoization,
    /// cycle detection and the whole-string/embedded split all run on `Vec<Seg>`
    /// already, so nothing downstream of this parse changes.
    fn target(&mut self, body: &str, path: &[Seg]) -> Option<Vec<Seg>> {
        let target: Vec<Seg> = match body.parse::<RefPath>() {
            Ok(parsed) => parsed.into_segs(),
            Err(PathError::BadIndex { .. }) => {
                self.problems.push(Problem::Syntax {
                    path: path.to_vec(),
                    error: Syntax::BadIndex {
                        body: body.to_string(),
                    },
                });
                return None;
            }
            // `EmptyPath` is unreachable: the scanner rejects `${}` first.
            Err(PathError::EmptySegment { .. } | PathError::EmptyPath) => {
                self.problems.push(Problem::Syntax {
                    path: path.to_vec(),
                    error: Syntax::EmptySegment {
                        body: body.to_string(),
                    },
                });
                return None;
            }
            // Neither can come out of `FromStr`: a reference body has no `=`
            // to miss, and index rejection lives in `try_into_keys`, which
            // only write-side callers run.
            Err(PathError::MissingEquals | PathError::IndexInKeyPath { .. }) => {
                unreachable!("parsing a reference body never reports these")
            }
        };
        if lookup(self.doc, &target).is_none() {
            self.problems.push(Problem::Unresolved {
                path: path.to_vec(),
                reference: body.to_string(),
            });
            return None;
        }
        Some(target)
    }
}

fn child(path: &[Seg], seg: Seg) -> Vec<Seg> {
    let mut out = Vec::with_capacity(path.len() + 1);
    out.extend_from_slice(path);
    out.push(seg);
    out
}

#[cfg(test)]
mod tests;
