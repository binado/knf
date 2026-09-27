//! `${key.path}` and `${env:VAR}` resolution over the merged native values.
//!
//! A whole-string reference (`"${p}"`) takes the referent's value and type; an
//! embedded one (`"x/${p}"`) stringifies, and containers are an error there.
//! `$$` is a literal `$`. The environment is injected through [`Env`].

mod error;
mod scan;

use std::collections::HashMap;

use crate::path::lookup;
use crate::{ConfigFormat, ConfigObject, PathError, RefPath, Seg};

pub use error::{Cycle, InterpError, Problem};
pub use scan::Syntax;

use scan::{Piece, Spelled, scan};

/// The environment namespace, matched as a literal prefix so `${a:b}` is the
/// ordinary key `a:b`.
const ENV: &str = "env:";

/// Where `${env:NAME}` reads from. [`ProcessEnv`](crate::ProcessEnv) reads the
/// real environment.
pub trait Env {
    /// The variable, or `None` if it is unset.
    fn lookup(&self, name: &str) -> Option<String>;
}

/// Resolves every reference in `doc`. Call once, on the merged document.
///
/// Reports all unresolved and malformed references together; a cycle is
/// reported alone.
pub fn interpolate<V: ConfigFormat>(doc: V, env: &dyn Env) -> Result<V, InterpError> {
    let mut resolver = Resolver {
        doc: &doc,
        env,
        memo: HashMap::new(),
        visiting: Vec::new(),
        problems: Vec::new(),
    };
    let resolved = resolver.resolve(&[]).map_err(InterpError::Cycle)?;
    if resolver.problems.is_empty() {
        Ok(resolved)
    } else {
        Err(InterpError::Problems(resolver.problems))
    }
}

/// Memoized depth-first resolution, keyed on path, so each problem is reported
/// once.
struct Resolver<'a, V: ConfigFormat> {
    doc: &'a V,
    env: &'a dyn Env,
    memo: HashMap<Vec<Seg>, V>,
    /// Paths being resolved, innermost last; also the cycle chain.
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

        let raw = lookup(self.doc, path).expect("resolve is only called on paths that exist");

        self.visiting.push(path.to_vec());
        let resolved = self.resolve_value(raw, path)?;
        self.visiting.pop();

        // No reference can name the root, so don't cache it.
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

        // No `$` anywhere.
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

    /// Whole-string position: takes the referent's value and type.
    fn substitute(&mut self, body: &str, path: &[Seg]) -> Result<V, Cycle> {
        if let Some(name) = body.strip_prefix(ENV) {
            return Ok(match self.env_value(name, body, path) {
                // Environment values are terminal: never re-scanned.
                Some(found) => V::parse_inline(found),
                None => V::string(Spelled(body).to_string()),
            });
        }
        match self.target(body, path) {
            Some(target) => self.resolve(&target),
            None => Ok(V::string(Spelled(body).to_string())),
        }
    }

    /// Embedded position: the referent is rendered as text.
    fn splice(&mut self, body: &str, path: &[Seg]) -> Result<String, Cycle> {
        if let Some(name) = body.strip_prefix(ENV) {
            return Ok(match self.env_value(name, body, path) {
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
