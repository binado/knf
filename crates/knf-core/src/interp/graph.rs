//! Configs as an expression graph. A merge whose operands may be references is
//! decided on demand, against the final document.

use std::rc::Rc;

use indexmap::IndexMap;

use crate::merge::shallow_at;
use crate::{Seg, Value};

use super::scan::{Piece, scan};
use super::{Cycle, ENV, Resolver, child};

pub(super) type Id = usize;

/// An object's fields in order.
pub(super) type Object = IndexMap<String, Id>;

/// An object's fields, shared between caches.
pub(super) type Fields = Rc<Object>;

pub(super) enum Expr {
    /// A non-container config value; strings are scanned on demand.
    Value(Value),
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
    /// Where the expression was written, or where a merge lands.
    pub path: Vec<Seg>,
}

/// A node's structure after following references and merges.
#[derive(Clone)]
pub(super) enum Shape {
    Object(Fields),
    Array(Rc<[Id]>),
    Scalar,
    /// A reference that names nothing; the problem is already recorded.
    Missing,
}

/// What a node is before anything is resolved.
#[derive(PartialEq)]
enum Hint {
    Object,
    Other,
    Unknown,
}

/// The body of a whole-string reference.
pub(super) fn whole_ref(text: &str) -> Option<&str> {
    match scan(text).as_slice() {
        [Piece::Ref(body)] => Some(body),
        _ => None,
    }
}

impl Resolver<'_> {
    pub(super) fn add(&mut self, expr: Expr, path: Vec<Seg>) -> Id {
        self.nodes.push(Node { expr, path });
        self.nodes.len() - 1
    }

    /// Converts a config value into nodes.
    pub(super) fn build(&mut self, value: Value, path: Vec<Seg>) -> Id {
        let expr = match value {
            Value::Array(items) => Expr::Array(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(index, item)| self.build(item, child(&path, Seg::Index(index))))
                    .collect(),
            ),
            Value::Object(map) => Expr::Object(
                map.into_iter()
                    .map(|(key, value)| {
                        let id = self.build(value, child(&path, Seg::Key(key.clone())));
                        (key, id)
                    })
                    .collect(),
            ),
            value => Expr::Value(value),
        };
        self.add(expr, path)
    }

    fn hint(&self, id: Id) -> Hint {
        match &self.nodes[id].expr {
            Expr::Object(_) => Hint::Object,
            Expr::Array(_) => Hint::Other,
            Expr::Terminal(value) if value.as_object().is_some() => Hint::Object,
            Expr::Terminal(_) => Hint::Other,
            Expr::Value(value) if value.as_str().and_then(whole_ref).is_some() => Hint::Unknown,
            Expr::Value(_) => Hint::Other,
            Expr::Merge { over, .. } => match self.hint(*over) {
                Hint::Object => Hint::Object,
                _ => Hint::Unknown,
            },
        }
    }

    /// Merges `over` onto `base` at `dest`, deciding now whenever a kind is
    /// known. A replaced operand is never resolved.
    pub(super) fn combine(&mut self, base: Id, over: Id, dest: Vec<Seg>) -> Id {
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
        let id = self.add(Expr::Merge { base, over }, key.2.clone());
        self.merges.insert(key, id);
        id
    }

    /// Marks `id` as being decided, or reports the structural cycle.
    fn enter(&mut self, id: Id) -> Result<(), Cycle> {
        if let Some(start) = self.shaping.iter().position(|&seen| seen == id) {
            let mut chain: Vec<_> = self.shaping[start..]
                .iter()
                .map(|&seen| self.nodes[seen].path.clone())
                .collect();
            chain.push(self.nodes[id].path.clone());
            return Err(Cycle::new(chain));
        }
        self.shaping.push(id);
        Ok(())
    }

    /// The node a whole-string reference names, or `None` once the problem
    /// is recorded. Environment values become terminal nodes.
    pub(super) fn resolve_ref(&mut self, id: Id) -> Result<Option<Id>, Cycle> {
        if let Some(&target) = self.targets.get(&id) {
            return Ok(target);
        }
        let Expr::Value(value) = &self.nodes[id].expr else {
            unreachable!("only strings hold references")
        };
        let body = value
            .as_str()
            .and_then(whole_ref)
            .expect("a whole-string reference")
            .to_owned();
        let path = self.nodes[id].path.clone();
        self.enter(id)?;
        let target = match self.expand(&body, &path)? {
            None => None,
            Some(body) => {
                if let Some(name) = body.strip_prefix(ENV) {
                    self.env_value(name, &body, &path).map(|found| {
                        self.add(Expr::Terminal((self.parse_inline)(found)), path.clone())
                    })
                } else {
                    self.target(&body, &path)?
                }
            }
        };
        self.shaping.pop();
        self.targets.insert(id, target);
        Ok(target)
    }

    pub(super) fn shape(&mut self, id: Id) -> Result<Shape, Cycle> {
        if let Some(shape) = self.shapes.get(&id) {
            return Ok(shape.clone());
        }
        let shape = match &self.nodes[id].expr {
            Expr::Value(value) if value.as_str().and_then(whole_ref).is_none() => Shape::Scalar,
            // Aliases can chain back to themselves.
            Expr::Value(_) => match self.resolve_ref(id)? {
                Some(target) => {
                    self.enter(id)?;
                    let shape = self.shape(target)?;
                    self.shaping.pop();
                    shape
                }
                None => Shape::Missing,
            },
            Expr::Terminal(value) => {
                let value = value.clone();
                self.terminal_shape(id, value)
            }
            Expr::Array(items) => Shape::Array(items.as_slice().into()),
            Expr::Object(fields) => Shape::Object(Rc::new(fields.clone())),
            &Expr::Merge { base, over } => {
                self.enter(id)?;
                let shape = self.merge_shape(id, base, over)?;
                self.shaping.pop();
                shape
            }
        };
        self.shapes.insert(id, shape.clone());
        Ok(shape)
    }

    fn terminal_shape(&mut self, id: Id, value: Value) -> Shape {
        let path = self.nodes[id].path.clone();
        match value {
            Value::Array(items) => Shape::Array(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(index, item)| {
                        self.add(Expr::Terminal(item), child(&path, Seg::Index(index)))
                    })
                    .collect(),
            ),
            Value::Object(map) => Shape::Object(Rc::new(
                map.into_iter()
                    .map(|(key, item)| {
                        let id =
                            self.add(Expr::Terminal(item), child(&path, Seg::Key(key.clone())));
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
        let path = self.nodes[id].path.clone();
        let over_fields = Object::clone(over_fields);
        let fields = self.overlay(Rc::unwrap_or_clone(base_fields), over_fields, &path);
        Ok(Shape::Object(Rc::new(fields)))
    }

    fn overlay(&mut self, mut base: Object, over: Object, dest: &[Seg]) -> Object {
        for (key, value) in over {
            if let Some(slot) = base.get_mut(&key) {
                *slot = self.combine(*slot, value, child(dest, Seg::Key(key)));
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
