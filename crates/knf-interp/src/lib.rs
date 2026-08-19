//! `${key.path}` and `${env:VAR}` resolution over the [`knf_core`] merge IR.
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
//! **No `std::env` here.** The environment is injected through [`Env`], so the
//! crate is deterministic and testable without touching process state — the same
//! property that makes `cargo test -p knf-core` a fast inner loop. It is also
//! what keeps the JSON-or-string typing rule out of this crate: the caller
//! parses and hands over a [`Value`].
//!
//! `knf-core`, `knf-dotted` (with `default-features = false`, so `serde_json`
//! stays out — the path vocabulary is unconditional there) and `thiserror`.
//! No format crate, no I/O.
//! `cargo tree -p knf-interp --depth 1` is the enforcement.

mod error;
mod path;
mod render;
mod scan;

use std::collections::HashMap;

use knf_core::{Map, Value};
use knf_dotted::{RefError, RefPath};

pub use error::{Cycle, InterpError, Problem};
pub use knf_dotted::{Seg, render_path};
pub use scan::Syntax;

use path::lookup;
use render::stringify;
use scan::{Piece, Spelled, scan};

/// The one namespace. Matched as a literal **prefix**, not by splitting on the
/// first `:`, so `${a:b}` is the ordinary key `a:b` and `${db.host:port}` does
/// not produce a baffling "unknown namespace `db.host`". The only unaddressable
/// keys are those literally beginning `env:` — the same class of limitation the
/// dotted-path flags already carry.
const ENV: &str = "env:";

/// An environment variable in both forms interpolation needs.
///
/// Two fields rather than one because the two positions want different things.
/// Embedded, the variable is spliced as **raw text**: parsing it and rendering
/// it back could only ever corrupt it. Whole-string, it is typed, by whatever
/// rule the caller uses for its own inline values — which is the entire
/// consistency argument for typing environment values at all.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvValue {
    /// Spliced verbatim into surrounding text.
    pub raw: String,
    /// Substituted whole, with its type, when the reference is the whole string.
    pub typed: Value,
}

/// Where `${env:NAME}` reads from.
///
/// A trait rather than a direct `std::env::var` call so that this crate never
/// touches process state; the binary supplies the one implementation that does.
pub trait Env {
    /// The variable, or `None` if it is unset.
    fn lookup(&self, name: &str) -> Option<EnvValue>;
}

/// Resolves every reference in `doc`.
///
/// By value because resolution builds a new tree rather than editing in place —
/// a referent must be read in its pre-substitution form no matter which order
/// the document is walked in.
///
/// Every unresolved reference and every malformed one is collected, so a run
/// reports all of them. A cycle is the exception and returns alone: there is
/// nothing meaningful to continue past.
pub fn interpolate(doc: Value, env: &dyn Env) -> Result<Value, InterpError> {
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
struct Resolver<'a> {
    doc: &'a Value,
    env: &'a dyn Env,
    memo: HashMap<Vec<Seg>, Value>,
    /// The paths currently being resolved, innermost last. Doubles as the cycle
    /// chain: the slice from a repeated path to the top *is* the loop.
    visiting: Vec<Vec<Seg>>,
    problems: Vec<Problem>,
}

impl<'a> Resolver<'a> {
    /// Resolves the node at `path`, which the caller has established exists.
    fn resolve(&mut self, path: &[Seg]) -> Result<Value, Cycle> {
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

    fn resolve_value(&mut self, raw: &'a Value, path: &[Seg]) -> Result<Value, Cycle> {
        match raw {
            Value::String(text) => self.resolve_string(text, path),
            // Children resolve under their own paths and memoize there, which
            // is what makes a whole-string container reference return a fully
            // resolved subtree — the cost of allowing aliasing at all.
            Value::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for index in 0..items.len() {
                    out.push(self.resolve(&child(path, Seg::Index(index)))?);
                }
                Ok(Value::Array(out))
            }
            Value::Object(map) => {
                let mut out = Map::with_capacity(map.len());
                for key in map.keys() {
                    let value = self.resolve(&child(path, Seg::Key(key.clone())))?;
                    out.insert(key.clone(), value);
                }
                Ok(Value::Object(out))
            }
            scalar => Ok(scalar.clone()),
        }
    }

    fn resolve_string(&mut self, text: &str, path: &[Seg]) -> Result<Value, Cycle> {
        let pieces = match scan(text) {
            Ok(pieces) => pieces,
            Err(error) => {
                self.problems.push(Problem::Syntax {
                    path: path.to_vec(),
                    error,
                });
                return Ok(Value::String(text.to_string()));
            }
        };

        match pieces.as_slice() {
            // No `$` anywhere — the common case, and the reason `scan` reports
            // it as emptiness rather than a list of one literal.
            [] => Ok(Value::String(text.to_string())),
            [Piece::Ref(body)] => self.substitute(body, path),
            embedded => {
                let mut out = String::new();
                for piece in embedded {
                    match piece {
                        Piece::Literal(literal) => out.push_str(literal),
                        Piece::Ref(body) => out.push_str(&self.splice(body, path)?),
                    }
                }
                Ok(Value::String(out))
            }
        }
    }

    /// Whole-string position: the reference *is* the value, so it takes the
    /// referent's type. Containers are allowed here.
    fn substitute(&mut self, body: &str, path: &[Seg]) -> Result<Value, Cycle> {
        if let Some(name) = body.strip_prefix(ENV) {
            return Ok(match self.env_value(name, body, path) {
                // Environment values are terminal: never re-scanned, so a
                // variable holding `${x}` cannot reach back into the document.
                Some(found) => found.typed,
                None => Value::String(Spelled(body).to_string()),
            });
        }
        match self.target(body, path) {
            Some(target) => self.resolve(&target),
            None => Ok(Value::String(Spelled(body).to_string())),
        }
    }

    /// Embedded position: the reference joins surrounding text, so it renders.
    fn splice(&mut self, body: &str, path: &[Seg]) -> Result<String, Cycle> {
        if let Some(name) = body.strip_prefix(ENV) {
            return Ok(match self.env_value(name, body, path) {
                // Raw, not re-rendered: a variable is text already, and parsing
                // it only to print it again could only lose something.
                Some(found) => found.raw,
                None => Spelled(body).to_string(),
            });
        }
        let Some(target) = self.target(body, path) else {
            return Ok(Spelled(body).to_string());
        };
        let value = self.resolve(&target)?;
        Ok(match stringify(&value) {
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
    fn env_value(&mut self, name: &str, body: &str, path: &[Seg]) -> Option<EnvValue> {
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
    /// `RefPath`, where the merge-side grammars stay keys-only — and memoization,
    /// cycle detection and the whole-string/embedded split all run on `Vec<Seg>`
    /// already, so nothing downstream of this parse changes.
    fn target(&mut self, body: &str, path: &[Seg]) -> Option<Vec<Seg>> {
        let target: Vec<Seg> = match body.parse::<RefPath>() {
            Ok(parsed) => parsed.into_segs(),
            Err(RefError::BadIndex { .. }) => {
                self.problems.push(Problem::Syntax {
                    path: path.to_vec(),
                    error: Syntax::BadIndex {
                        body: body.to_string(),
                    },
                });
                return None;
            }
            // `EmptyKey` is unreachable: the scanner rejects `${}` first.
            Err(RefError::EmptySegment { .. } | RefError::EmptyKey) => {
                self.problems.push(Problem::Syntax {
                    path: path.to_vec(),
                    error: Syntax::EmptySegment {
                        body: body.to_string(),
                    },
                });
                return None;
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
