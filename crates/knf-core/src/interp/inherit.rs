//! Lazy object structure, with source paths and terminal environment values.

use std::collections::HashMap;

use crate::glob::KeyGlobPattern;
use crate::{ConfigFormat, ConfigObject, Seg};

use super::{Cycle, ENV, Env, Piece, Problem, Syntax, child, parse_target, scan};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Source {
    context: bool,
    pub path: Vec<Seg>,
}

#[derive(Clone)]
pub(super) struct Node<V> {
    pub raw: V,
    pub source: Source,
    pub terminal: bool,
    pub fields: Option<Vec<(String, Node<V>)>>,
}

impl<V: ConfigFormat> Node<V> {
    fn root(raw: V, context: bool) -> Self {
        Self {
            raw,
            source: Source {
                context,
                path: Vec::new(),
            },
            terminal: false,
            fields: None,
        }
    }

    pub fn descendant(&self, raw: V, seg: Seg) -> Self {
        Self {
            raw,
            source: Source {
                context: self.source.context,
                path: child(&self.source.path, seg),
            },
            terminal: self.terminal,
            fields: None,
        }
    }

    fn children(&self) -> Vec<(String, Self)> {
        self.fields.clone().unwrap_or_else(|| {
            self.raw
                .as_object()
                .expect("object node")
                .iter()
                .map(|(key, value)| {
                    (
                        key.clone(),
                        self.descendant(value.clone(), Seg::Key(key.clone())),
                    )
                })
                .collect()
        })
    }

    fn get(&self, seg: &Seg) -> Option<Self> {
        match seg {
            Seg::Key(key) => {
                if let Some(fields) = &self.fields {
                    ConfigObject::get(fields, key).cloned()
                } else {
                    self.raw
                        .as_object()?
                        .get(key)
                        .map(|value| self.descendant(value.clone(), seg.clone()))
                }
            }
            Seg::Index(index) => self
                .raw
                .as_array()?
                .get(*index)
                .map(|value| self.descendant(value.clone(), seg.clone())),
        }
    }
}

impl<V: ConfigFormat> ConfigObject<Node<V>> for Vec<(String, Node<V>)> {
    fn new() -> Self {
        Vec::new()
    }
    fn iter<'b>(&'b self) -> impl Iterator<Item = (&'b String, &'b Node<V>)>
    where
        Node<V>: 'b,
    {
        self.as_slice().iter().map(|(key, node)| (key, node))
    }
    fn get(&self, key: &str) -> Option<&Node<V>> {
        self.as_slice()
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, node)| node)
    }
    fn get_mut(&mut self, key: &str) -> Option<&mut Node<V>> {
        self.as_mut_slice()
            .iter_mut()
            .find(|(name, _)| name == key)
            .map(|(_, node)| node)
    }
    fn insert(&mut self, key: String, node: Node<V>) {
        if let Some(slot) = ConfigObject::get_mut(self, &key) {
            *slot = node;
        } else {
            self.push((key, node));
        }
    }
}

type ShapeKey = (Source, Vec<Seg>);

pub(super) struct Projection<'a, V: ConfigFormat> {
    doc: &'a V,
    context: Option<&'a V>,
    env: &'a dyn Env,
    key: &'a str,
    shallow: Option<&'a KeyGlobPattern>,
    shapes: HashMap<ShapeKey, Node<V>>,
    expanding: Vec<ShapeKey>,
    pub problems: Vec<Problem>,
}

impl<'a, V: ConfigFormat> Projection<'a, V> {
    pub fn new(
        doc: &'a V,
        context: Option<&'a V>,
        env: &'a dyn Env,
        key: &'a str,
        shallow: Option<&'a KeyGlobPattern>,
    ) -> Self {
        Self {
            doc,
            context,
            env,
            key,
            shallow,
            shapes: HashMap::new(),
            expanding: Vec::new(),
            problems: Vec::new(),
        }
    }

    pub fn lookup(&mut self, path: &[Seg]) -> Result<Option<Node<V>>, Cycle> {
        if let Some(node) = self.lookup_in(self.doc, false, path)? {
            return Ok(Some(node));
        }
        match self.context {
            Some(context) => self.lookup_in(context, true, path),
            None => Ok(None),
        }
    }

    fn lookup_in(
        &mut self,
        root: &V,
        context: bool,
        path: &[Seg],
    ) -> Result<Option<Node<V>>, Cycle> {
        let mut node = Node::root(root.clone(), context);
        for (index, seg) in path.iter().enumerate() {
            let parent = &path[..index];
            let key = (node.source.clone(), parent.to_vec());
            if self.expanding.contains(&key) {
                // Explicit children are available while their parent's base
                // is being selected; inherited children depend on that base.
                if let Some(explicit) = node.get(seg) {
                    node = explicit;
                    continue;
                }
                self.enter(key)?;
                unreachable!("enter detects the active structural dependency");
            }
            node = self.expand(node, parent)?;
            let Some(next) = node.get(seg) else {
                return Ok(None);
            };
            node = next;
        }
        Ok(Some(node))
    }

    fn enter(&mut self, key: ShapeKey) -> Result<(), Cycle> {
        if let Some(start) = self.expanding.iter().position(|seen| seen == &key) {
            let mut chain: Vec<_> = self.expanding[start..]
                .iter()
                .map(|(source, _)| source.path.clone())
                .collect();
            chain.push(key.0.path.clone());
            return Err(Cycle::new(chain));
        }
        self.expanding.push(key);
        Ok(())
    }

    pub fn expand(&mut self, mut node: Node<V>, path: &[Seg]) -> Result<Node<V>, Cycle> {
        if node.fields.is_some() || node.raw.as_object().is_none() {
            return Ok(node);
        }
        if node.terminal {
            node.fields = Some(node.children());
            return Ok(node);
        }
        let cache_key = (node.source.clone(), path.to_vec());
        if let Some(done) = self.shapes.get(&cache_key) {
            return Ok(done.clone());
        }
        self.enter(cache_key.clone())?;
        let mut fields = node.children();
        let directive = fields
            .as_slice()
            .iter()
            .position(|(name, _)| name == self.key)
            .map(|index| fields.remove(index).1);
        node.fields = Some(fields);
        if let Some(directive) = directive
            && let Some(base) = self.directive(directive)?
        {
            node = self.overlay(base, node, path)?;
        }
        self.expanding.pop();
        self.shapes.insert(cache_key, node.clone());
        Ok(node)
    }

    fn directive(&mut self, node: Node<V>) -> Result<Option<Node<V>>, Cycle> {
        let Some(text) = node.raw.as_str() else {
            self.problems.push(Problem::MergeReference {
                path: node.source.path,
            });
            return Ok(None);
        };
        let pieces = scan(text);
        let [Piece::Ref(body)] = pieces.as_slice() else {
            let mut malformed = false;
            for piece in pieces {
                if let Piece::Malformed { error, .. } = piece {
                    malformed = true;
                    self.problems.push(Problem::Syntax {
                        path: node.source.path.clone(),
                        error,
                    });
                }
            }
            if !malformed {
                self.problems.push(Problem::MergeReference {
                    path: node.source.path,
                });
            }
            return Ok(None);
        };
        let path = node.source.path.clone();
        let Some(base) = self.reference(body, &path)? else {
            return Ok(None);
        };
        let Some(base) = self.base(base)? else {
            return Ok(None);
        };
        if base.raw.as_object().is_none() {
            self.problems.push(Problem::MergeObject {
                path,
                kind: base.raw.kind(),
            });
            return Ok(None);
        }
        Ok(Some(base))
    }

    /// Follow whole-string aliases without resolving the base's children.
    fn base(&mut self, node: Node<V>) -> Result<Option<Node<V>>, Cycle> {
        if !node.terminal
            && let Some(text) = node.raw.as_str()
            && let [Piece::Ref(body)] = scan(text).as_slice()
        {
            let key = (node.source.clone(), node.source.path.clone());
            self.enter(key)?;
            let next = self.reference(body, &node.source.path)?;
            let base = match next {
                Some(next) => self.base(next)?,
                None => None,
            };
            self.expanding.pop();
            return Ok(base);
        }
        let path = node.source.path.clone();
        self.expand(node, &path).map(Some)
    }

    fn reference(&mut self, body: &str, path: &[Seg]) -> Result<Option<Node<V>>, Cycle> {
        if let Some(name) = body.strip_prefix(ENV) {
            if name.is_empty() {
                self.problems.push(Problem::Syntax {
                    path: path.to_vec(),
                    error: Syntax::EmptyEnvName,
                });
                return Ok(None);
            }
            let Some(text) = self.env.lookup(name) else {
                self.problems.push(Problem::Unresolved {
                    path: path.to_vec(),
                    reference: body.to_owned(),
                });
                return Ok(None);
            };
            return Ok(Some(Node {
                raw: V::parse_inline(text),
                source: Source {
                    context: false,
                    path: path.to_vec(),
                },
                terminal: true,
                fields: None,
            }));
        }
        let Some(target) = parse_target(body, path, &mut self.problems) else {
            return Ok(None);
        };
        let node = self.lookup(&target)?;
        if node.is_none() {
            self.problems.push(Problem::Unresolved {
                path: path.to_vec(),
                reference: body.to_owned(),
            });
        }
        Ok(node)
    }

    fn overlay(&mut self, base: Node<V>, over: Node<V>, path: &[Seg]) -> Result<Node<V>, Cycle> {
        if crate::merge::shallow_at(self.shallow, path) {
            return Ok(over);
        }
        if base.raw.as_object().is_none() || over.raw.as_object().is_none() {
            return Ok(over);
        }
        let base = self.expand(base, path)?;
        let over = self.expand(over, path)?;
        let mut fields = base.children();
        crate::merge::merge_fields(&mut fields, over.children(), |slot, value, key| {
            *slot = self.overlay(slot.clone(), value, &child(path, Seg::Key(key)))?;
            Ok(())
        })?;
        Ok(Node {
            fields: Some(fields),
            terminal: false,
            ..over
        })
    }
}
