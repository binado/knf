//! `${key.path}` and `${env:VAR}` resolution over native layers.
//!
//! A whole-string reference (`"${p}"`) takes the referent's value and type; an
//! embedded one (`"x/${p}"`) stringifies, and containers are an error there.
//! `$$` is a literal `$`. The environment is injected through [`Env`].
//!
//! A reference body is itself interpolated first, so `${a.${b}}` reads the
//! path `a` followed by whatever `b` holds; inner values must be scalars.
//!
//! Layers fold as expressions, so a whole-string reference merges exactly as
//! the value it names would. References bind to the final document.

mod error;
mod graph;
mod scan;

use std::collections::HashMap;

use crate::glob::KeyGlobPattern;
use crate::{ConfigFormat, ConfigObject, PathError, RefPath, Seg};

pub use error::{Cycle, InterpError, Problem};
pub use scan::Syntax;

use graph::{Expr, Id, Node, Shape};
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

/// Options for reference-aware merging and opt-in object inheritance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InterpOptions {
    /// Literal key whose whole-string reference supplies an object's defaults.
    /// `None` leaves every key as ordinary data.
    pub merge_key: Option<String>,
    /// Destination key paths to replace wholesale, in layer and inheritance
    /// merges. Matching ancestors stop merging; paths with indices never match.
    pub shallow: Option<KeyGlobPattern>,
}

/// Left-folds `layers` like [`merge`](crate::merge), then resolves references
/// against the result, looking up each complete path in it first, then in
/// `context`. Only the merged document is returned.
///
/// A whole-string reference merges as the value it names: an object merges
/// with an object on the other side; anything else replaces. `shallow` paths
/// replace wholesale. An operand that is replaced is never resolved, but a
/// reference whose kind decides a merge must resolve.
///
/// With `merge_key`, an object's marker must hold one whole-string reference
/// to an object, which supplies defaults beneath the object's own fields. The
/// marker is removed from the result.
///
/// Context values are resolved only when referenced. Environment values are
/// terminal. Invalid directives and reference problems are reported together;
/// cycles are reported alone.
pub fn merge_interpolate<V: ConfigFormat>(
    layers: impl IntoIterator<Item = V>,
    context: Option<&V>,
    env: &dyn Env,
    options: &InterpOptions,
) -> Result<V, InterpError> {
    let mut resolver = Resolver {
        env,
        merge_key: options.merge_key.as_deref(),
        shallow: options.shallow.as_ref(),
        nodes: Vec::new(),
        doc: 0,
        context: None,
        merges: HashMap::new(),
        shapes: HashMap::new(),
        raws: HashMap::new(),
        targets: HashMap::new(),
        shaping: Vec::new(),
        values: HashMap::new(),
        visiting: Vec::new(),
        problems: Vec::new(),
    };
    let mut doc = resolver.add(Expr::Object(Vec::new()), Vec::new());
    for layer in layers {
        let layer = resolver.build(layer, Vec::new());
        doc = resolver.combine(doc, layer, Vec::new());
    }
    resolver.doc = doc;
    resolver.context = context.map(|context| resolver.build(context.clone(), Vec::new()));
    let resolved = resolver.eval(doc).map_err(InterpError::Cycle)?;
    if resolver.problems.is_empty() {
        Ok(resolved)
    } else {
        Err(InterpError::Problems(resolver.problems))
    }
}

/// Resolves every reference in `doc`.
///
/// Reports all unresolved and malformed references together; a cycle is
/// reported alone.
pub fn interpolate<V: ConfigFormat>(doc: V, env: &dyn Env) -> Result<V, InterpError> {
    merge_interpolate([doc], None, env, &InterpOptions::default())
}

/// Resolves references in `doc`, looking up each complete path in `doc` first,
/// then in `context`. Context values are resolved only when referenced, using
/// the same lookup order for their dependencies. Only `doc` is returned.
///
/// Existing values, including null, take precedence. Selected containers keep
/// their own children; they are never combined with the other document.
pub fn interpolate_with_context<V: ConfigFormat>(
    doc: V,
    context: &V,
    env: &dyn Env,
) -> Result<V, InterpError> {
    merge_interpolate([doc], Some(context), env, &InterpOptions::default())
}

/// The expression graph and every cache over it. Node ids are stable, so
/// each cache is keyed by id alone.
struct Resolver<'a, V: ConfigFormat> {
    env: &'a dyn Env,
    merge_key: Option<&'a str>,
    shallow: Option<&'a KeyGlobPattern>,
    nodes: Vec<Node<V>>,
    doc: Id,
    context: Option<Id>,
    merges: HashMap<(Id, Id, Vec<Seg>), Id>,
    shapes: HashMap<Id, Shape>,
    raws: HashMap<Id, Shape>,
    targets: HashMap<Id, Option<Id>>,
    /// Nodes whose structure or target is being decided, innermost last.
    shaping: Vec<Id>,
    values: HashMap<Id, V>,
    /// Nodes being evaluated, innermost last; also the cycle chain.
    visiting: Vec<Id>,
    problems: Vec<Problem>,
}

impl<V: ConfigFormat> Resolver<'_, V> {
    fn eval(&mut self, id: Id) -> Result<V, Cycle> {
        if let Some(done) = self.values.get(&id) {
            return Ok(done.clone());
        }
        if let Some(start) = self.visiting.iter().position(|&seen| seen == id) {
            let mut chain: Vec<_> = self.visiting[start..]
                .iter()
                .map(|&seen| self.nodes[seen].path.clone())
                .collect();
            chain.push(self.nodes[id].path.clone());
            return Err(Cycle::new(chain));
        }
        self.visiting.push(id);
        let value = match &self.nodes[id].expr {
            Expr::Terminal(value) => value.clone(),
            Expr::Value(value) => match value.as_str() {
                Some(text) => {
                    let text = text.to_owned();
                    self.resolve_string(id, &text)?
                }
                None => value.clone(),
            },
            Expr::Array(items) => {
                let items = items.clone();
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item)?);
                }
                V::array(out)
            }
            Expr::Object(_) | Expr::Merge { .. } => match self.shape(id)? {
                Shape::Object(fields) => {
                    let mut out = V::Object::new();
                    for (key, field) in fields.iter() {
                        out.insert(key.clone(), self.eval(*field)?);
                    }
                    V::object(out)
                }
                // Only a merge resolves to a non-object: its right operand.
                _ => {
                    let Expr::Merge { over, .. } = self.nodes[id].expr else {
                        unreachable!("an object node always shapes as an object")
                    };
                    self.visiting.pop();
                    let value = self.eval(over)?;
                    self.values.insert(id, value.clone());
                    return Ok(value);
                }
            },
        };
        self.visiting.pop();
        self.values.insert(id, value.clone());
        Ok(value)
    }

    fn resolve_string(&mut self, id: Id, text: &str) -> Result<V, Cycle> {
        let pieces = scan(text);

        // No `$` anywhere.
        if pieces.is_empty() {
            return Ok(V::string(text.to_string()));
        }
        if let [Piece::Ref(body)] = pieces.as_slice() {
            return Ok(match self.resolve_ref(id)? {
                Some(target) => self.eval(target)?,
                None => V::string(Spelled(body).to_string()),
            });
        }

        let path = self.nodes[id].path.clone();
        let mut out = String::new();
        for piece in pieces {
            match piece {
                Piece::Literal(literal) => out.push_str(literal),
                Piece::Ref(body) => out.push_str(&self.splice(body, &path)?),
                Piece::Malformed { spelling, error } => {
                    self.problems.push(Problem::Syntax {
                        path: path.clone(),
                        error,
                    });
                    out.push_str(spelling);
                }
            }
        }
        Ok(V::string(out))
    }

    /// Embedded position: the referent is rendered as text.
    fn splice(&mut self, body: &str, path: &[Seg]) -> Result<String, Cycle> {
        let Some(expanded) = self.expand(body, path)? else {
            return Ok(Spelled(body).to_string());
        };
        let body = expanded.as_str();
        if let Some(name) = body.strip_prefix(ENV) {
            return Ok(match self.env_value(name, body, path) {
                Some(found) => found,
                None => Spelled(body).to_string(),
            });
        }
        let Some(target) = self.target(body, path)? else {
            return Ok(Spelled(body).to_string());
        };
        let value = self.eval(target)?;
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

    /// A reference body with its inner references spliced in as text, or
    /// `None` if that recorded a problem. The result is not scanned again.
    pub(super) fn expand(&mut self, body: &str, path: &[Seg]) -> Result<Option<String>, Cycle> {
        let pieces = scan(body);
        if pieces.is_empty() {
            return Ok(Some(body.to_string()));
        }
        let before = self.problems.len();
        let mut out = String::new();
        for piece in pieces {
            match piece {
                Piece::Literal(literal) => out.push_str(literal),
                Piece::Ref(inner) => out.push_str(&self.splice(inner, path)?),
                Piece::Malformed { error, .. } => self.problems.push(Problem::Syntax {
                    path: path.to_vec(),
                    error,
                }),
            }
        }
        Ok((self.problems.len() == before).then_some(out))
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

    /// The node a reference names, recording a problem and returning `None`
    /// if it is malformed or names nothing.
    fn target(&mut self, body: &str, path: &[Seg]) -> Result<Option<Id>, Cycle> {
        let Some(target) = parse_target(body, path, &mut self.problems) else {
            return Ok(None);
        };
        let found = self.lookup(&target)?;
        if found.is_none() {
            self.problems.push(Problem::Unresolved {
                path: path.to_vec(),
                reference: body.to_string(),
            });
        }
        Ok(found)
    }
}

fn parse_target(body: &str, path: &[Seg], problems: &mut Vec<Problem>) -> Option<Vec<Seg>> {
    Some(match body.parse::<RefPath>() {
        Ok(parsed) => parsed.into_segs(),
        Err(PathError::BadIndex { .. }) => {
            problems.push(Problem::Syntax {
                path: path.to_vec(),
                error: Syntax::BadIndex {
                    body: body.to_string(),
                },
            });
            return None;
        }
        // `EmptyPath` is unreachable: the scanner rejects `${}` first.
        Err(PathError::EmptySegment { .. } | PathError::EmptyPath) => {
            problems.push(Problem::Syntax {
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
    })
}

fn child(path: &[Seg], seg: Seg) -> Vec<Seg> {
    let mut out = Vec::with_capacity(path.len() + 1);
    out.extend_from_slice(path);
    out.push(seg);
    out
}

#[cfg(test)]
mod tests;
