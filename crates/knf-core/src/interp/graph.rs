//! Layers as an expression graph. A merge whose operands may be references is
//! decided on demand, against the final document.

use std::rc::Rc;

use crate::merge::{merge_fields, shallow_at};
use crate::{ConfigFormat, ConfigObject, Seg};

use super::scan::{Piece, scan};
use super::{Cycle, ENV, Problem, Resolver, child};

pub(super) type Id = usize;

/// An object's fields in order, shared between caches.
pub(super) type Fields = Rc<[(String, Id)]>;

pub(super) enum Expr<V> {
    /// A non-container layer value; strings are scanned on demand.
    Value(V),
    /// An environment value: never scanned, and its children stay terminal.
    Terminal(V),
    Object(Vec<(String, Id)>),
    Array(Vec<Id>),
    /// `over` merged onto `base` at this node's path.
    Merge {
        base: Id,
        over: Id,
    },
}

pub(super) struct Node<V> {
    pub expr: Expr<V>,
    /// Where the expression was written, or where a merge lands.
    pub path: Vec<Seg>,
}

/// A node's structure after following references, merges and inheritance.
#[derive(Clone)]
pub(super) enum Shape {
    Object(Fields),
    Array(Rc<[Id]>),
    Scalar(&'static str),
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

impl ConfigObject<Id> for Vec<(String, Id)> {
    fn new() -> Self {
        Vec::new()
    }
    fn iter<'b>(&'b self) -> impl Iterator<Item = (&'b String, &'b Id)>
    where
        Id: 'b,
    {
        self.as_slice().iter().map(|(key, id)| (key, id))
    }
    fn get(&self, key: &str) -> Option<&Id> {
        self.as_slice()
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, id)| id)
    }
    fn get_mut(&mut self, key: &str) -> Option<&mut Id> {
        self.as_mut_slice()
            .iter_mut()
            .find(|(name, _)| name == key)
            .map(|(_, id)| id)
    }
    fn insert(&mut self, key: String, id: Id) {
        if let Some(slot) = ConfigObject::get_mut(self, &key) {
            *slot = id;
        } else {
            self.push((key, id));
        }
    }
}

impl<V: ConfigFormat> Resolver<'_, V> {
    pub(super) fn add(&mut self, expr: Expr<V>, path: Vec<Seg>) -> Id {
        self.nodes.push(Node { expr, path });
        self.nodes.len() - 1
    }

    /// Converts a native value into nodes.
    pub(super) fn build(&mut self, value: V, path: Vec<Seg>) -> Id {
        let expr = if let Some(items) = value.as_array() {
            let items = items.to_vec();
            Expr::Array(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(index, item)| self.build(item, child(&path, Seg::Index(index))))
                    .collect(),
            )
        } else {
            match value.into_object() {
                Ok(map) => Expr::Object(
                    map.into_iter()
                        .map(|(key, value)| {
                            let id = self.build(value, child(&path, Seg::Key(key.clone())));
                            (key, id)
                        })
                        .collect(),
                ),
                Err(value) => Expr::Value(value),
            }
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
    /// known. A replaced operand is never resolved. Markers are directives,
    /// not values, so the last one wins.
    pub(super) fn combine(&mut self, base: Id, over: Id, dest: Vec<Seg>) -> Id {
        let marker =
            matches!(dest.last(), Some(Seg::Key(key)) if Some(key.as_str()) == self.merge_key);
        if marker
            || shallow_at(self.shallow, &dest)
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
        let target = if let Some(name) = body.strip_prefix(ENV) {
            self.env_value(name, &body, &path)
                .map(|found| self.add(Expr::Terminal(V::parse_inline(found)), path.clone()))
        } else {
            self.target(&body, &path)?
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
            Expr::Value(value) if value.as_str().and_then(whole_ref).is_none() => {
                Shape::Scalar(value.kind())
            }
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
            // Markers fold like any field, so apply only the surviving one.
            Expr::Object(_) | Expr::Merge { .. } => match self.raw(id)? {
                Shape::Object(fields) => {
                    self.enter(id)?;
                    let shape = self.view(id, &fields)?;
                    self.shaping.pop();
                    shape
                }
                other => other,
            },
        };
        self.shapes.insert(id, shape.clone());
        Ok(shape)
    }

    /// Structure before inheritance: literal operands keep their markers,
    /// while a reference contributes its referent's final shape.
    fn raw(&mut self, id: Id) -> Result<Shape, Cycle> {
        match &self.nodes[id].expr {
            Expr::Object(fields) => Ok(Shape::Object(fields.as_slice().into())),
            &Expr::Merge { base, over } => {
                if let Some(shape) = self.raws.get(&id) {
                    return Ok(shape.clone());
                }
                self.enter(id)?;
                let shape = self.merge_shape(id, base, over)?;
                self.shaping.pop();
                self.raws.insert(id, shape.clone());
                Ok(shape)
            }
            _ => self.shape(id),
        }
    }

    fn terminal_shape(&mut self, id: Id, value: V) -> Shape {
        let path = self.nodes[id].path.clone();
        if let Some(items) = value.as_array() {
            Shape::Array(
                items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        let path = child(&path, Seg::Index(index));
                        self.add(Expr::Terminal(item.clone()), path)
                    })
                    .collect(),
            )
        } else if let Some(map) = value.as_object() {
            Shape::Object(
                map.iter()
                    .map(|(key, item)| {
                        let path = child(&path, Seg::Key(key.clone()));
                        (key.clone(), self.add(Expr::Terminal(item.clone()), path))
                    })
                    .collect(),
            )
        } else {
            Shape::Scalar(value.kind())
        }
    }

    /// An object's fields, under its marker's base when it has one. Keys from
    /// environment values are data, never markers.
    fn view(&mut self, id: Id, fields: &Fields) -> Result<Shape, Cycle> {
        let marker = self.merge_key.and_then(|key| {
            fields.iter().position(|&(ref name, field)| {
                name == key && !matches!(self.nodes[field].expr, Expr::Terminal(_))
            })
        });
        let Some(index) = marker else {
            return Ok(Shape::Object(fields.clone()));
        };
        let mut fields = fields.to_vec();
        let directive = fields.remove(index).1;
        let path = self.nodes[id].path.clone();
        if let Some(base) = self.directive(directive)?
            && !shallow_at(self.shallow, &path)
        {
            fields = self.overlay(&base, fields, &path);
        }
        Ok(Shape::Object(fields.into()))
    }

    /// The base fields a marker names, recording why when it names none.
    fn directive(&mut self, id: Id) -> Result<Option<Fields>, Cycle> {
        let path = self.nodes[id].path.clone();
        let text = match &self.nodes[id].expr {
            Expr::Value(value) => value.as_str().map(str::to_owned),
            _ => None,
        };
        let Some(text) = text else {
            self.problems.push(Problem::MergeReference { path });
            return Ok(None);
        };
        if whole_ref(&text).is_none() {
            let mut malformed = false;
            for piece in scan(&text) {
                if let Piece::Malformed { error, .. } = piece {
                    malformed = true;
                    self.problems.push(Problem::Syntax {
                        path: path.clone(),
                        error,
                    });
                }
            }
            if !malformed {
                self.problems.push(Problem::MergeReference { path });
            }
            return Ok(None);
        }
        let Some(target) = self.resolve_ref(id)? else {
            return Ok(None);
        };
        let kind = match self.shape(target)? {
            Shape::Object(fields) => return Ok(Some(fields)),
            Shape::Missing => return Ok(None),
            Shape::Array(_) => "array",
            Shape::Scalar(kind) => kind,
        };
        self.problems.push(Problem::MergeObject { path, kind });
        Ok(None)
    }

    /// Right operand first: only an object there needs the left one.
    fn merge_shape(&mut self, id: Id, base: Id, over: Id) -> Result<Shape, Cycle> {
        let over_shape = self.raw(over)?;
        let Shape::Object(over_fields) = &over_shape else {
            return Ok(over_shape);
        };
        let Shape::Object(base_fields) = self.raw(base)? else {
            return Ok(over_shape);
        };
        let path = self.nodes[id].path.clone();
        let fields = self.overlay(&base_fields, over_fields.to_vec(), &path);
        Ok(Shape::Object(fields.into()))
    }

    fn overlay(
        &mut self,
        base: &[(String, Id)],
        over: Vec<(String, Id)>,
        dest: &[Seg],
    ) -> Vec<(String, Id)> {
        let mut fields = base.to_vec();
        let Ok(()) = merge_fields(&mut fields, over, |slot: &mut Id, value, key| {
            *slot = self.combine(*slot, value, child(dest, Seg::Key(key)));
            Ok::<(), std::convert::Infallible>(())
        });
        fields
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
            (Shape::Object(fields), Seg::Key(key)) => fields
                .iter()
                .find(|(name, _)| name == key)
                .map(|&(_, id)| id),
            (Shape::Array(items), Seg::Index(index)) => items.get(*index).copied(),
            _ => None,
        })
    }

    /// While its structure is being decided, a node exposes only the children
    /// written locally; anything else is absent from the output.
    fn explicit(&self, id: Id, seg: &Seg) -> Option<Id> {
        match (&self.nodes[id].expr, seg) {
            (Expr::Object(fields), Seg::Key(key)) if self.merge_key != Some(key.as_str()) => {
                ConfigObject::get(fields, key).copied()
            }
            (&Expr::Merge { base, over }, _) => self.explicit(over, seg).or_else(|| {
                matches!(self.nodes[over].expr, Expr::Object(_))
                    .then(|| self.explicit(base, seg))
                    .flatten()
            }),
            _ => None,
        }
    }
}
