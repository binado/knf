//! Indexed lookups into a document.
//!
//! [`Seg`] itself lives in `knf-dotted` with the rest of the path vocabulary;
//! only [`lookup`] is here, because it is the one piece that needs
//! `knf_core::Value` — and `knf-dotted` must not.
//!
//! Indices serve both directions of a reference: where one *lives* — inside an
//! array, if that is where the string sits — and where one *points*, since a
//! `${servers[0]}` body parses to a path with an [`Seg::Index`] in it. The
//! merge-side grammars stay keys-only; only this crate's readers index.

use knf_core::Value;
use knf_dotted::Seg;

/// The node at `path`, or `None` if nothing lives there.
pub fn lookup<'a>(root: &'a Value, path: &[Seg]) -> Option<&'a Value> {
    let mut node = root;
    for seg in path {
        node = match (seg, node) {
            (Seg::Key(k), Value::Object(map)) => map.get(k)?,
            (Seg::Index(i), Value::Array(items)) => items.get(*i)?,
            _ => return None,
        };
    }
    Some(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use knf_core::{Map, Number};

    fn doc() -> Value {
        let mut inner = Map::new();
        inner.insert("host".into(), Value::String("h".into()));
        let mut root = Map::new();
        root.insert("db".into(), Value::Object(inner));
        root.insert(
            "tags".into(),
            Value::Array(vec![Value::Number(Number::I64(1))]),
        );
        Value::Object(root)
    }

    #[test]
    fn lookup_walks_objects_and_arrays() {
        let doc = doc();
        assert_eq!(lookup(&doc, &[]), Some(&doc));
        assert_eq!(
            lookup(&doc, &[Seg::Key("db".into()), Seg::Key("host".into())]),
            Some(&Value::String("h".into()))
        );
        assert_eq!(
            lookup(&doc, &[Seg::Key("tags".into()), Seg::Index(0)]),
            Some(&Value::Number(Number::I64(1)))
        );
    }

    /// A missing key and a segment of the wrong shape are both simply absent —
    /// the caller reports one unresolved reference either way.
    #[test]
    fn lookup_misses_are_none() {
        let doc = doc();
        assert_eq!(lookup(&doc, &[Seg::Key("nope".into())]), None);
        assert_eq!(lookup(&doc, &[Seg::Key("db".into()), Seg::Index(0)]), None);
        assert_eq!(
            lookup(&doc, &[Seg::Key("tags".into()), Seg::Index(9)]),
            None
        );
    }
}
