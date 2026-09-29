//! `${key.path}` and `${env:VAR}` resolution over the merged native values.
//!
//! A whole-string reference (`"${p}"`) takes the referent's value and type; an
//! embedded one (`"x/${p}"`) stringifies, and containers are an error there.
//! `$$` is a literal `$`. The environment is injected through [`Env`].

mod error;
mod inherit;
mod scan;

use std::collections::HashMap;

use crate::glob::KeyGlobPattern;
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

/// Options for interpolation and opt-in object inheritance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InterpOptions {
    /// Literal key whose whole-string reference supplies an object's defaults.
    /// `None` leaves every key as ordinary data.
    pub merge_key: Option<String>,
    /// Destination key paths to replace wholesale during inheritance.
    /// Matching ancestors stop merging; paths containing indices do not match.
    pub shallow: Option<KeyGlobPattern>,
}

/// Resolve references and optionally merge referenced objects into their parents.
///
/// Inheritance runs after layer merging, before resolving surviving values.
/// Local fields override the base; arrays and null replace wholesale. Context
/// lookup is output-first and lazy. Environment values remain terminal.
///
/// The selected marker must contain one whole-string reference to an object
/// and is removed from the result. References remain absolute, including in
/// inherited fields; destination paths can address those fields. Context
/// expressions discarded by overrides are never resolved. Invalid directives
/// and reference problems are reported together; cycles are reported alone.
pub fn interpolate_with_options<V: ConfigFormat>(
    doc: V,
    context: Option<&V>,
    env: &dyn Env,
    options: &InterpOptions,
) -> Result<V, InterpError> {
    resolve_document(doc, context, env, options)
}

/// Resolves every reference in `doc`. Call once, on the merged document.
///
/// Reports all unresolved and malformed references together; a cycle is
/// reported alone.
pub fn interpolate<V: ConfigFormat>(doc: V, env: &dyn Env) -> Result<V, InterpError> {
    interpolate_with_options(doc, None, env, &InterpOptions::default())
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
    interpolate_with_options(doc, Some(context), env, &InterpOptions::default())
}

fn resolve_document<V: ConfigFormat>(
    doc: V,
    context: Option<&V>,
    env: &dyn Env,
    options: &InterpOptions,
) -> Result<V, InterpError> {
    let mut resolver = Resolver {
        doc: &doc,
        context,
        env,
        memo: HashMap::new(),
        source_memo: HashMap::new(),
        visiting: Vec::new(),
        containers: Vec::new(),
        problems: Vec::new(),
        projection: options
            .merge_key
            .as_deref()
            .map(|key| inherit::Projection::new(&doc, context, env, key, options.shallow.as_ref())),
    };
    let resolved = resolver.resolve(&[]).map_err(InterpError::Cycle)?;
    if let Some(projection) = resolver.projection {
        for problem in projection.problems {
            if !resolver.problems.contains(&problem) {
                resolver.problems.push(problem);
            }
        }
    }
    if resolver.problems.is_empty() {
        Ok(resolved)
    } else {
        Err(InterpError::Problems(resolver.problems))
    }
}

/// Output-first lookup fixes one source per path, so caches need only the path.
struct Resolver<'a, V: ConfigFormat> {
    doc: &'a V,
    context: Option<&'a V>,
    env: &'a dyn Env,
    memo: HashMap<Vec<Seg>, V>,
    // Inherited expressions keep their source and must be evaluated only once.
    source_memo: HashMap<inherit::Source, V>,
    /// Paths being resolved, innermost last; also the cycle chain.
    visiting: Vec<Vec<Seg>>,
    /// Inherited containers can repeat their source at ever-longer destinations.
    containers: Vec<(inherit::Source, Vec<Seg>)>,
    problems: Vec<Problem>,
    projection: Option<inherit::Projection<'a, V>>,
}

impl<'a, V: ConfigFormat> Resolver<'a, V> {
    fn lookup(&self, path: &[Seg]) -> Option<&'a V> {
        lookup(self.doc, path).or_else(|| self.context.and_then(|context| lookup(context, path)))
    }

    /// Resolves the node at `path`, which the caller has established exists.
    fn resolve(&mut self, path: &[Seg]) -> Result<V, Cycle> {
        if let Some(projection) = &mut self.projection {
            let node = projection.lookup(path)?.expect("reference exists");
            return self.resolve_node(node, path);
        }
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

        let raw = self
            .lookup(path)
            .expect("resolve is only called on paths that exist");

        self.visiting.push(path.to_vec());
        let resolved = self.resolve_value(raw, path)?;
        self.visiting.pop();

        // No reference can name the root, so don't cache it.
        if !path.is_empty() {
            self.memo.insert(path.to_vec(), resolved.clone());
        }
        Ok(resolved)
    }

    fn resolve_node(&mut self, node: inherit::Node<V>, path: &[Seg]) -> Result<V, Cycle> {
        if let Some(done) = self.memo.get(path) {
            return Ok(done.clone());
        }
        if let Some(start) = self.visiting.iter().position(|seen| seen == path) {
            let mut chain = self.visiting[start..].to_vec();
            chain.push(path.to_vec());
            return Err(Cycle::new(chain));
        }
        self.visiting.push(path.to_vec());
        let mut node = self
            .projection
            .as_mut()
            .expect("inheritance enabled")
            .expand(node, path)?;
        let container = !node.terminal && (node.fields.is_some() || node.raw.as_array().is_some());
        if container {
            if let Some(start) = self
                .containers
                .iter()
                .position(|(source, _)| source == &node.source)
            {
                let mut chain: Vec<_> = self.containers[start..]
                    .iter()
                    .map(|(_, path)| path.clone())
                    .collect();
                chain.push(path.to_vec());
                return Err(Cycle::new(chain));
            }
            self.containers.push((node.source.clone(), path.to_vec()));
        }
        let resolved = if let Some(fields) = node.fields.take() {
            let mut out = V::Object::new();
            for (key, node) in fields {
                let value = self.resolve_node(node, &child(path, Seg::Key(key.clone())))?;
                out.insert(key, value);
            }
            V::object(out)
        } else if node.terminal {
            node.raw
        } else if let Some(items) = node.raw.as_array() {
            let mut out = Vec::with_capacity(items.len());
            for (index, value) in items.iter().enumerate() {
                let seg = Seg::Index(index);
                let item = node.descendant(value.clone(), seg.clone());
                out.push(self.resolve_node(item, &child(path, seg))?);
            }
            V::array(out)
        } else if let Some(text) = node.raw.as_str() {
            if let Some(done) = self.source_memo.get(&node.source) {
                done.clone()
            } else {
                let value = self.resolve_string(text, &node.source.path)?;
                self.source_memo.insert(node.source, value.clone());
                value
            }
        } else {
            node.raw
        };
        self.visiting.pop();
        if container {
            self.containers.pop();
        }
        if !path.is_empty() {
            self.memo.insert(path.to_vec(), resolved.clone());
        }
        Ok(resolved)
    }

    fn resolve_value(&mut self, raw: &'a V, path: &[Seg]) -> Result<V, Cycle> {
        if let Some(text) = raw.as_str() {
            return self.resolve_string(text, path);
        }
        // A context container is absent from doc, as are all its descendants.
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
        match self.target(body, path)? {
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
        let Some(target) = self.target(body, path)? else {
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

    /// The path a reference names, recording a problem and returning
    /// `None` if it is malformed or names nothing.
    fn target(&mut self, body: &str, path: &[Seg]) -> Result<Option<Vec<Seg>>, Cycle> {
        let Some(target) = parse_target(body, path, &mut self.problems) else {
            return Ok(None);
        };
        let exists = if let Some(projection) = &mut self.projection {
            projection.lookup(&target)?.is_some()
        } else {
            self.lookup(&target).is_some()
        };
        if !exists {
            self.problems.push(Problem::Unresolved {
                path: path.to_vec(),
                reference: body.to_string(),
            });
            return Ok(None);
        }
        Ok(Some(target))
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
