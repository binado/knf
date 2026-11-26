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
mod report;
mod scan;

use std::collections::HashMap;
use std::rc::Rc;

use indexmap::IndexMap;

use crate::glob::KeyGlobPattern;
use crate::{MergeOptions, RefPath, Seg, Value};

pub use error::{Cycle, InterpError, Origin, Problem, ProblemKind, Problems, Syntax};
pub use scan::ScanError;

use graph::{Body, Expr, Id, Node, Object, Part, Shape, Stack};
use report::{Outcome, Reported, Sink};

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
        visiting: Vec::new(),
        sink: Sink::default(),
    };
    let output = Origin::Output;
    let mut doc = resolver.add(Expr::Object(Object::new()), output, Vec::new());
    for config in configs {
        let config = resolver.build(config, output, Vec::new());
        doc = resolver.combine(doc, config, output, Vec::new());
    }
    resolver.doc = doc;
    resolver.context = options
        .context
        .map(|context| resolver.build(context.clone(), Origin::Context, Vec::new()));
    let resolved = resolver.eval(doc).map_err(InterpError::Cycle)?;
    resolver
        .sink
        .finish(resolved)
        .map_err(InterpError::Problems)
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
    targets: HashMap<Id, Outcome<Id>>,
    /// [`Stack::Shaping`], innermost last.
    shaping: Vec<Id>,
    values: HashMap<Id, Outcome<Value>>,
    /// [`Stack::Visiting`], innermost last.
    visiting: Vec<Id>,
    sink: Sink,
}

/// How a node gets its value.
enum Evaluated {
    Here(Outcome<Value>),
    /// A merge that is its right operand. That node is evaluated once the
    /// merge is off the stack, so the merge is not a hop in a cycle chain.
    Forward(Id),
}

impl Resolver<'_> {
    fn eval(&mut self, id: Id) -> Result<Outcome<Value>, Cycle> {
        if let Some(done) = self.values.get(&id) {
            return Ok(done.clone());
        }
        let value = match self.within(Stack::Visiting, id, |resolver| resolver.eval_uncached(id))? {
            Evaluated::Here(value) => value,
            Evaluated::Forward(over) => self.eval(over)?,
        };
        self.values.insert(id, value.clone());
        Ok(value)
    }

    fn eval_uncached(&mut self, id: Id) -> Result<Evaluated, Cycle> {
        Ok(Evaluated::Here(match &self.nodes[id].expr {
            Expr::Scalar(scalar) => Ok(scalar.value().clone()),
            Expr::Terminal(value) => Ok(value.clone()),
            Expr::WholeRef(body) => {
                let body = body.clone();
                match self.resolve_ref(id, &body)? {
                    Ok(target) => self.eval(target)?,
                    Err(reported) => Err(reported),
                }
            }
            Expr::Template(parts) => {
                let parts = Rc::clone(parts);
                self.expand(&parts, id)?.map(Value::String)
            }
            Expr::Array(items) => {
                let items = items.clone();
                self.eval_array(&items)?
            }
            Expr::Object(fields) => {
                let fields = fields.clone();
                self.eval_object(&fields)?
            }
            &Expr::Merge { over, .. } => match self.shape(id)? {
                Shape::Object(fields) => self.eval_object(&fields)?,
                Shape::Missing(reported) => Err(reported),
                Shape::Array(_) | Shape::Scalar => return Ok(Evaluated::Forward(over)),
            },
        }))
    }

    /// Evaluates every item, so that one failure does not hide another.
    fn eval_array(&mut self, items: &[Id]) -> Result<Outcome<Value>, Cycle> {
        let mut values = Vec::with_capacity(items.len());
        for &item in items {
            values.push(self.eval(item)?);
        }
        let values: Outcome<Vec<_>> = values.into_iter().collect();
        Ok(values.map(Value::Array))
    }

    /// Evaluates every field, so that one failure does not hide another.
    fn eval_object(&mut self, fields: &Object) -> Result<Outcome<Value>, Cycle> {
        let mut entries = Vec::with_capacity(fields.len());
        for (key, &field) in fields {
            entries.push(self.eval(field)?.map(|value| (key.clone(), value)));
        }
        let entries: Outcome<IndexMap<_, _>> = entries.into_iter().collect();
        Ok(entries.map(Value::Object))
    }

    /// The text of the string or reference body written at `at`, with its
    /// references spliced in. The result is not scanned again. Every part is
    /// tried, so that all of their problems are reported.
    fn expand(&mut self, parts: &[Part], at: Id) -> Result<Outcome<String>, Cycle> {
        let mut texts = Vec::with_capacity(parts.len());
        for part in parts {
            texts.push(match part {
                Part::Literal(literal) => Ok(literal.clone()),
                Part::Ref(body) => self.splice(body, at)?,
                Part::Malformed(error) => {
                    Err(self.report(at, ProblemKind::Syntax(error.clone().into())))
                }
            });
        }
        Ok(texts.into_iter().collect())
    }

    /// Embedded position: the referent is rendered as text.
    fn splice(&mut self, body: &Body, at: Id) -> Result<Outcome<String>, Cycle> {
        let reference = match self.expand(&body.0, at)? {
            Ok(reference) => reference,
            Err(reported) => return Ok(Err(reported)),
        };
        if let Some(name) = reference.strip_prefix(ENV) {
            return Ok(self.env_value(name, &reference, at));
        }
        let target = match self.target(&reference, at)? {
            Ok(target) => target,
            Err(reported) => return Ok(Err(reported)),
        };
        Ok(self.eval(target)?.and_then(|value| {
            value.stringify().ok_or_else(|| {
                let kind = value.kind();
                self.report(at, ProblemKind::NotStringifiable { reference, kind })
            })
        }))
    }

    /// The variable `reference` names, which must be set.
    fn env_value(&mut self, name: &str, reference: &str, at: Id) -> Outcome<String> {
        if name.is_empty() {
            return Err(self.report(at, ProblemKind::Syntax(Syntax::EmptyEnvName)));
        }
        self.env.lookup(name).ok_or_else(|| {
            let reference = reference.to_string();
            self.report(at, ProblemKind::Unresolved { reference })
        })
    }

    /// The node a reference names, which must be a path to something.
    fn target(&mut self, reference: &str, at: Id) -> Result<Outcome<Id>, Cycle> {
        let path = match reference.parse::<RefPath>() {
            Ok(path) => path.into_segs(),
            Err(error) => {
                let body = reference.to_string();
                let syntax = Syntax::Path { body, error };
                return Ok(Err(self.report(at, ProblemKind::Syntax(syntax))));
            }
        };
        Ok(self.lookup(&path)?.ok_or_else(|| {
            let reference = reference.to_string();
            self.report(at, ProblemKind::Unresolved { reference })
        }))
    }

    /// Records a problem with the string at node `at`.
    fn report(&mut self, at: Id, kind: ProblemKind) -> Reported {
        let Node { origin, path, .. } = &self.nodes[at];
        self.sink.report(Problem {
            origin: *origin,
            path: path.clone(),
            kind,
        })
    }
}

fn child(path: &[Seg], seg: Seg) -> Vec<Seg> {
    let mut out = Vec::with_capacity(path.len() + 1);
    out.extend_from_slice(path);
    out.push(seg);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(entries: &[(&str, Value)]) -> Value {
        Value::Object(
            entries
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect(),
        )
    }

    fn text(s: &str) -> Value {
        Value::String(s.to_owned())
    }

    fn failure(doc: Value) -> String {
        interpolate(doc, &InterpolationOptions::new(&Value::String))
            .expect_err("should fail")
            .to_string()
    }

    #[test]
    fn a_failure_found_while_shaping_is_not_reported_again() {
        let b = ("b", text("${t.${x}}"));
        let x = ("x", text("${missing}"));
        let a = ("a", text("${x.k}"));
        // Only `a` reads through `x`, which shapes it before it is evaluated.
        assert_eq!(
            failure(object(&[a, b.clone(), x.clone()])),
            "unresolved reference\n  --> x: `missing`\n  --> a: `x.k`"
        );
        assert_eq!(
            failure(object(&[b, x])),
            "unresolved reference\n  --> x: `missing`"
        );
    }

    #[test]
    fn a_container_reports_every_failing_child() {
        let children = || [("a", text("${one}")), ("b", text("x${two}"))];
        let list = Value::Array(children().map(|(_, value)| value).to_vec());
        for container in [object(&children()), list] {
            let report = failure(object(&[("c", container), ("copy", text("n=${c}"))]));
            assert!(report.contains("`one`"), "{report}");
            assert!(report.contains("`two`"), "{report}");
            // Embedding the failed container adds nothing.
            assert_eq!(report.lines().count(), 3, "{report}");
        }
    }
}
