//! Configs as an expression graph. A merge whose operands may be references is
//! decided on demand, against the final document.

use std::rc::Rc;

use indexmap::IndexMap;

use crate::merge::shallow_at;
use crate::{Seg, Value};

use super::report::{Outcome, Reported};
use super::scan::{Piece, ScanError, Scanned, scan};
use super::{Cycle, ENV, Origin, Resolver, child};
use scalar::{Container, Scalar};

pub(super) type Id = usize;

/// An object's fields in order.
pub(super) type Object = IndexMap<String, Id>;

/// An object's fields, shared between caches.
pub(super) type Fields = Rc<Object>;

mod scalar {
    use indexmap::IndexMap;

    use crate::Value;

    /// A value with no children.
    pub(in super::super) struct Scalar(Value);

    /// The contents of a value that is not a [`Scalar`].
    pub(in super::super) enum Container {
        Array(Vec<Value>),
        Object(IndexMap<String, Value>),
    }

    impl Scalar {
        pub(in super::super) fn value(&self) -> &Value {
            &self.0
        }
    }

    impl From<String> for Scalar {
        fn from(text: String) -> Self {
            Self(Value::String(text))
        }
    }

    impl TryFrom<Value> for Scalar {
        type Error = Container;

        fn try_from(value: Value) -> Result<Self, Self::Error> {
            match value {
                Value::Array(items) => Err(Container::Array(items)),
                Value::Object(fields) => Err(Container::Object(fields)),
                value @ (Value::Null
                | Value::Bool(_)
                | Value::Integer(_)
                | Value::Unsigned(_)
                | Value::Float(_)
                | Value::String(_)
                | Value::Datetime(_)) => Ok(Self(value)),
            }
        }
    }
}

/// One span of a parsed string.
pub(super) enum Part {
    Literal(String),
    Ref(Body),
    Malformed(ScanError),
}

/// A reference body, with its inner references already parsed.
#[derive(Clone)]
pub(super) struct Body(pub Rc<[Part]>);

impl Body {
    fn parse(text: &str) -> Self {
        Self(match scan(text) {
            Scanned::Plain(text) => [Part::Literal(text.into_owned())].into(),
            Scanned::Whole(body) => [Part::Ref(Self::parse(body))].into(),
            Scanned::Template(pieces) => parts(pieces),
        })
    }
}

fn parts(pieces: Vec<Piece>) -> Rc<[Part]> {
    pieces
        .into_iter()
        .map(|piece| match piece {
            Piece::Literal(text) => Part::Literal(text.to_owned()),
            Piece::Ref(body) => Part::Ref(Body::parse(body)),
            Piece::Malformed(error) => Part::Malformed(error),
        })
        .collect()
}

fn string_expr(text: &str) -> Expr {
    match scan(text) {
        Scanned::Plain(text) => Expr::Scalar(text.into_owned().into()),
        Scanned::Whole(body) => Expr::WholeRef(Body::parse(body)),
        Scanned::Template(pieces) => Expr::Template(parts(pieces)),
    }
}

pub(super) enum Expr {
    /// A config value with no references and no children.
    Scalar(Scalar),
    /// A string that is one reference: it takes the referent's value.
    WholeRef(Body),
    /// A string with references among text.
    Template(Rc<[Part]>),
    /// An environment value: never scanned, and its children stay terminal.
    Terminal(Value),
    Object(Object),
    Array(Vec<Id>),
    /// `over` merged onto `base` at this node's path.
    Merge {
        base: Id,
        over: Id,
    },
}

pub(super) struct Node {
    pub expr: Expr,
    /// The tree it was written in; derived nodes inherit their creator's.
    pub origin: Origin,
    /// Where the expression was written, or where a merge lands.
    pub path: Vec<Seg>,
}

/// Which in-progress chain a node joins; re-entering one is a cycle.
#[derive(Clone, Copy)]
pub(super) enum Stack {
    /// Nodes being evaluated.
    Visiting,
    /// Nodes whose structure or target is being decided.
    Shaping,
}

/// A node's structure after following references and merges.
#[derive(Clone)]
pub(super) enum Shape {
    Object(Fields),
    Array(Rc<[Id]>),
    Scalar,
    /// A reference that failed to resolve.
    Missing(Reported),
}

/// What a node is before anything is resolved.
#[derive(PartialEq)]
enum Hint {
    Object,
    Other,
    Unknown,
}

impl Resolver<'_> {
    pub(super) fn add(&mut self, expr: Expr, origin: Origin, path: Vec<Seg>) -> Id {
        self.nodes.push(Node { expr, origin, path });
        self.nodes.len() - 1
    }

    /// Where a node derived from `id` is reported to be.
    fn site(&self, id: Id) -> (Origin, Vec<Seg>) {
        let node = &self.nodes[id];
        (node.origin, node.path.clone())
    }

    /// Converts a config value into nodes, parsing each string once.
    pub(super) fn build(&mut self, value: Value, origin: Origin, path: Vec<Seg>) -> Id {
        let expr = match value {
            Value::String(text) => string_expr(&text),
            value => match Scalar::try_from(value) {
                Ok(scalar) => Expr::Scalar(scalar),
                Err(Container::Array(items)) => Expr::Array(
                    items
                        .into_iter()
                        .enumerate()
                        .map(|(index, item)| {
                            self.build(item, origin, child(&path, Seg::Index(index)))
                        })
                        .collect(),
                ),
                Err(Container::Object(fields)) => Expr::Object(
                    fields
                        .into_iter()
                        .map(|(key, value)| {
                            let id = self.build(value, origin, child(&path, Seg::Key(key.clone())));
                            (key, id)
                        })
                        .collect(),
                ),
            },
        };
        self.add(expr, origin, path)
    }

    fn hint(&self, id: Id) -> Hint {
        match &self.nodes[id].expr {
            Expr::Object(_) => Hint::Object,
            Expr::Terminal(value) if value.as_object().is_some() => Hint::Object,
            Expr::Array(_) | Expr::Scalar(_) | Expr::Template(_) | Expr::Terminal(_) => Hint::Other,
            Expr::WholeRef(_) => Hint::Unknown,
            Expr::Merge { over, .. } => match self.hint(*over) {
                Hint::Object => Hint::Object,
                _ => Hint::Unknown,
            },
        }
    }

    /// Merges `over` onto `base` at `dest`, deciding now whenever a kind is
    /// known. A replaced operand is never resolved.
    pub(super) fn combine(&mut self, base: Id, over: Id, origin: Origin, dest: Vec<Seg>) -> Id {
        if shallow_at(self.shallow, &dest)
            || self.hint(over) == Hint::Other
            || self.hint(base) == Hint::Other
        {
            return over;
        }
        let key = (base, over, dest);
        if let Some(&id) = self.merges.get(&key) {
            return id;
        }
        let id = self.add(Expr::Merge { base, over }, origin, key.2.clone());
        self.merges.insert(key, id);
        id
    }

    /// Runs `f` with `id` on `stack`, or reports the cycle if it is already there.
    pub(super) fn within<T>(
        &mut self,
        stack: Stack,
        id: Id,
        f: impl FnOnce(&mut Self) -> Result<T, Cycle>,
    ) -> Result<T, Cycle> {
        if let Some(start) = self.stack(stack).iter().position(|&seen| seen == id) {
            let hops = self.stack(stack)[start..].to_vec();
            let chain = hops
                .into_iter()
                .chain([id])
                .map(|seen| self.nodes[seen].path.clone())
                .collect();
            return Err(Cycle::new(chain));
        }
        self.stack(stack).push(id);
        let result = f(self);
        self.stack(stack).pop();
        result
    }

    fn stack(&mut self, stack: Stack) -> &mut Vec<Id> {
        match stack {
            Stack::Visiting => &mut self.visiting,
            Stack::Shaping => &mut self.shaping,
        }
    }

    /// The node a whole-string reference at `id` names. Environment values
    /// become terminal nodes.
    pub(super) fn resolve_ref(&mut self, id: Id, body: &Body) -> Result<Outcome<Id>, Cycle> {
        if let Some(&target) = self.targets.get(&id) {
            return Ok(target);
        }
        let target = self.within(Stack::Shaping, id, |resolver| {
            let body = match resolver.expand(&body.0, id)? {
                Ok(body) => body,
                Err(reported) => return Ok(Err(reported)),
            };
            let Some(name) = body.strip_prefix(ENV) else {
                return resolver.target(&body, id);
            };
            Ok(resolver.env_value(name, &body, id).map(|found| {
                let value = (resolver.parse_inline)(found);
                let (origin, path) = resolver.site(id);
                resolver.add(Expr::Terminal(value), origin, path)
            }))
        })?;
        self.targets.insert(id, target);
        Ok(target)
    }

    pub(super) fn shape(&mut self, id: Id) -> Result<Shape, Cycle> {
        if let Some(shape) = self.shapes.get(&id) {
            return Ok(shape.clone());
        }
        let shape = match &self.nodes[id].expr {
            Expr::Scalar(_) | Expr::Template(_) => Shape::Scalar,
            Expr::WholeRef(body) => {
                let body = body.clone();
                match self.resolve_ref(id, &body)? {
                    // Aliases can chain back to themselves.
                    Ok(target) => {
                        self.within(Stack::Shaping, id, |resolver| resolver.shape(target))?
                    }
                    Err(reported) => Shape::Missing(reported),
                }
            }
            Expr::Terminal(value) => {
                let value = value.clone();
                self.terminal_shape(id, value)
            }
            Expr::Array(items) => Shape::Array(items.as_slice().into()),
            Expr::Object(fields) => Shape::Object(Rc::new(fields.clone())),
            &Expr::Merge { base, over } => self.within(Stack::Shaping, id, |resolver| {
                resolver.merge_shape(id, base, over)
            })?,
        };
        self.shapes.insert(id, shape.clone());
        Ok(shape)
    }

    fn terminal_shape(&mut self, id: Id, value: Value) -> Shape {
        let (origin, path) = self.site(id);
        let mut terminal = |item, seg| self.add(Expr::Terminal(item), origin, child(&path, seg));
        match value {
            Value::Array(items) => Shape::Array(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(index, item)| terminal(item, Seg::Index(index)))
                    .collect(),
            ),
            Value::Object(map) => Shape::Object(Rc::new(
                map.into_iter()
                    .map(|(key, item)| {
                        let id = terminal(item, Seg::Key(key.clone()));
                        (key, id)
                    })
                    .collect(),
            )),
            _ => Shape::Scalar,
        }
    }

    /// Right operand first: only an object there needs the left one.
    fn merge_shape(&mut self, id: Id, base: Id, over: Id) -> Result<Shape, Cycle> {
        let over_shape = self.shape(over)?;
        let Shape::Object(over_fields) = &over_shape else {
            return Ok(over_shape);
        };
        let Shape::Object(base_fields) = self.shape(base)? else {
            return Ok(over_shape);
        };
        let (origin, path) = self.site(id);
        let over_fields = Object::clone(over_fields);
        let fields = self.overlay(Rc::unwrap_or_clone(base_fields), over_fields, origin, &path);
        Ok(Shape::Object(Rc::new(fields)))
    }

    fn overlay(&mut self, mut base: Object, over: Object, origin: Origin, dest: &[Seg]) -> Object {
        for (key, value) in over {
            if let Some(slot) = base.get_mut(&key) {
                *slot = self.combine(*slot, value, origin, child(dest, Seg::Key(key)));
            } else {
                base.insert(key, value);
            }
        }
        base
    }

    /// Output first by complete path, then context.
    pub(super) fn lookup(&mut self, path: &[Seg]) -> Result<Option<Id>, Cycle> {
        let doc = self.doc;
        if let Some(found) = self.walk(doc, path)? {
            return Ok(Some(found));
        }
        match self.context {
            Some(context) => self.walk(context, path),
            None => Ok(None),
        }
    }

    fn walk(&mut self, mut id: Id, path: &[Seg]) -> Result<Option<Id>, Cycle> {
        for seg in path {
            match self.child(id, seg)? {
                Some(next) => id = next,
                None => return Ok(None),
            }
        }
        Ok(Some(id))
    }

    fn child(&mut self, id: Id, seg: &Seg) -> Result<Option<Id>, Cycle> {
        if self.shaping.contains(&id) {
            return Ok(self.explicit(id, seg));
        }
        Ok(match (self.shape(id)?, seg) {
            (Shape::Object(fields), Seg::Key(key)) => fields.get(key).copied(),
            (Shape::Array(items), Seg::Index(index)) => items.get(*index).copied(),
            _ => None,
        })
    }

    /// While its structure is being decided, a node exposes only the children
    /// written locally; anything else is absent from the output.
    fn explicit(&self, id: Id, seg: &Seg) -> Option<Id> {
        match (&self.nodes[id].expr, seg) {
            (Expr::Object(fields), Seg::Key(key)) => fields.get(key).copied(),
            (&Expr::Merge { base, over }, _) => self.explicit(over, seg).or_else(|| {
                matches!(self.nodes[over].expr, Expr::Object(_))
                    .then(|| self.explicit(base, seg))
                    .flatten()
            }),
            _ => None,
        }
    }
}
