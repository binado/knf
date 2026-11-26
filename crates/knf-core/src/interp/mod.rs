//! `${key.path}` and `${env:VAR}` resolution over ordered configs.
//!
//! A whole-string reference (`"${p}"`) takes the referent's value and type; an
//! embedded one (`"x/${p}"`) stringifies, and containers are an error there.
//! `$$` is a literal `$`. The environment is injected through [`Env`].
//!
//! A reference body is itself interpolated first, so `${a.${b}}` reads the
//! path `a` followed by whatever `b` holds; inner values must be scalars.
//!
//! Configs fold as expressions, so a whole-string reference merges exactly as
//! the value it names would. References bind to the final document.

mod error;
mod graph;
mod scan;

use std::collections::{HashMap, HashSet};

use crate::glob::KeyGlobPattern;
use crate::{MergeOptions, PathError, RefPath, Seg, Value};

pub use error::{Cycle, InterpError, Problem};
pub use scan::Syntax;

use graph::{Expr, Id, Node, Object, Shape};
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

/// Inputs used to resolve references.
pub struct InterpolationOptions<'a> {
    /// Fallback document for paths absent from the output. A present value, including null, wins.
    pub context: Option<&'a Value>,
    /// Source for environment references. Embedded references keep their raw text.
    pub env: &'a dyn Env,
    /// Parser for whole environment references. Pass `&Value::String` to keep the raw text.
    pub parse_inline: &'a dyn Fn(String) -> Value,
}

impl<'a> InterpolationOptions<'a> {
    /// Read the process environment, with no context.
    pub fn new(parse_inline: &'a dyn Fn(String) -> Value) -> Self {
        Self {
            context: None,
            env: &crate::ProcessEnv,
            parse_inline,
        }
    }

    /// Replace the context document.
    pub fn with_context(self, context: Option<&'a Value>) -> Self {
        Self { context, ..self }
    }
}

/// Left-folds `configs` like [`merge`](crate::merge), then resolves references
/// against the result, looking up each complete path in it first, then in
/// `context`. Only the merged document is returned.
///
/// A whole-string reference merges as the value it names: an object merges
/// with an object on the other side; anything else replaces. Paths matched by
/// `merge_options.shallow` replace wholesale. An operand that is replaced is
/// never resolved, but a reference whose kind decides a merge must resolve.
///
/// Context values are resolved only when referenced. Environment values are
/// terminal. Invalid directives and reference problems are reported together;
/// cycles are reported alone.
pub fn merge_with_interpolation(
    configs: impl IntoIterator<Item = Value>,
    merge_options: &MergeOptions,
    options: &InterpolationOptions,
) -> Result<Value, InterpError> {
    let mut resolver = Resolver {
        env: options.env,
        parse_inline: options.parse_inline,
        shallow: merge_options.shallow.as_ref(),
        nodes: Vec::new(),
        doc: 0,
        context: None,
        merges: HashMap::new(),
        shapes: HashMap::new(),
        targets: HashMap::new(),
        shaping: Vec::new(),
        values: HashMap::new(),
        failed: HashSet::new(),
        visiting: Vec::new(),
        problems: Vec::new(),
    };
    let mut doc = resolver.add(Expr::Object(Object::new()), Vec::new());
    for config in configs {
        let config = resolver.build(config, Vec::new());
        doc = resolver.combine(doc, config, Vec::new());
    }
    resolver.doc = doc;
    resolver.context = options
        .context
        .map(|context| resolver.build(context.clone(), Vec::new()));
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
pub fn interpolate(doc: Value, options: &InterpolationOptions) -> Result<Value, InterpError> {
    merge_with_interpolation([doc], &MergeOptions::default(), options)
}

/// Expression graph and caches. `nodes` is append-only so ids are stable cache keys.
struct Resolver<'a> {
    env: &'a dyn Env,
    parse_inline: &'a dyn Fn(String) -> Value,
    shallow: Option<&'a KeyGlobPattern>,
    nodes: Vec<Node>,
    doc: Id,
    context: Option<Id>,
    merges: HashMap<(Id, Id, Vec<Seg>), Id>,
    shapes: HashMap<Id, Shape>,
    targets: HashMap<Id, Option<Id>>,
    /// Nodes whose structure or target is being decided, innermost last.
    shaping: Vec<Id>,
    values: HashMap<Id, Value>,
    /// Evaluated nodes whose evaluation recorded a problem.
    failed: HashSet<Id>,
    /// Nodes being evaluated, innermost last; also the cycle chain.
    visiting: Vec<Id>,
    problems: Vec<Problem>,
}

impl Resolver<'_> {
    fn eval(&mut self, id: Id) -> Result<Value, Cycle> {
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
        let problems = self.problems.len();
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
                Value::Array(out)
            }
            Expr::Object(_) | Expr::Merge { .. } => match self.shape(id)? {
                Shape::Object(fields) => {
                    let mut out = indexmap::IndexMap::new();
                    for (key, field) in fields.iter() {
                        out.insert(key.clone(), self.eval(*field)?);
                    }
                    Value::Object(out)
                }
                // Only a merge resolves to a non-object: its right operand.
                _ => {
                    let Expr::Merge { over, .. } = self.nodes[id].expr else {
                        unreachable!("an object node always shapes as an object")
                    };
                    self.visiting.pop();
                    let value = self.eval(over)?;
                    if self.failed.contains(&over) {
                        self.failed.insert(id);
                    }
                    self.values.insert(id, value.clone());
                    return Ok(value);
                }
            },
        };
        self.visiting.pop();
        if self.problems.len() > problems {
            self.failed.insert(id);
        }
        self.values.insert(id, value.clone());
        Ok(value)
    }

    fn resolve_string(&mut self, id: Id, text: &str) -> Result<Value, Cycle> {
        let pieces = scan(text);

        // No `$` anywhere.
        if pieces.is_empty() {
            return Ok(Value::String(text.to_string()));
        }
        if let [Piece::Ref(body)] = pieces.as_slice() {
            return Ok(match self.resolve_ref(id)? {
                Some(target) => self.eval(target)?,
                None => Value::String(Spelled(body).to_string()),
            });
        }

        let path = self.nodes[id].path.clone();
        let mut out = String::new();
        for piece in pieces {
            match piece {
                Piece::Literal(literal) => out.push_str(literal),
                Piece::Ref(body) => match self.splice(body, &path)? {
                    Some(text) => out.push_str(&text),
                    None => out.push_str(&Spelled(body).to_string()),
                },
                Piece::Malformed { spelling, error } => {
                    self.problems.push(Problem::Syntax {
                        path: path.clone(),
                        error,
                    });
                    out.push_str(spelling);
                }
            }
        }
        Ok(Value::String(out))
    }

    /// Embedded position: the referent is rendered as text. `None` if a
    /// problem was recorded, now or when the referent was first evaluated.
    fn splice(&mut self, body: &str, path: &[Seg]) -> Result<Option<String>, Cycle> {
        let Some(expanded) = self.expand(body, path)? else {
            return Ok(None);
        };
        let body = expanded.as_str();
        if let Some(name) = body.strip_prefix(ENV) {
            return Ok(self.env_value(name, body, path));
        }
        let Some(target) = self.target(body, path)? else {
            return Ok(None);
        };
        let value = self.eval(target)?;
        if self.failed.contains(&target) {
            return Ok(None);
        }
        Ok(match value.stringify() {
            Some(text) => Some(text),
            None => {
                self.problems.push(Problem::NotStringifiable {
                    path: path.to_vec(),
                    reference: body.to_string(),
                    kind: value.kind(),
                });
                None
            }
        })
    }

    /// A reference body with its inner references spliced in as text, or
    /// `None` if any of them failed. The result is not scanned again.
    pub(super) fn expand(&mut self, body: &str, path: &[Seg]) -> Result<Option<String>, Cycle> {
        let pieces = scan(body);
        if pieces.is_empty() {
            return Ok(Some(body.to_string()));
        }
        let mut ok = true;
        let mut out = String::new();
        for piece in pieces {
            match piece {
                Piece::Literal(literal) => out.push_str(literal),
                Piece::Ref(inner) => match self.splice(inner, path)? {
                    Some(text) => out.push_str(&text),
                    None => ok = false,
                },
                Piece::Malformed { error, .. } => {
                    self.problems.push(Problem::Syntax {
                        path: path.to_vec(),
                        error,
                    });
                    ok = false;
                }
            }
        }
        Ok(ok.then_some(out))
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
        // `EmptyPath` occurs when a nested reference expands to `""`.
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
