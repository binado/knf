//! Indexed paths into a document.
//!
//! A near-copy of the `Seg`/`render_path` pair in `crates/knf/src/value.rs`,
//! duplicated on purpose rather than promoted: `knf-core`'s own paths are
//! `Vec<String>` because merge only ever descends through object keys, and
//! moving an indexed path type there would make the core carry something it
//! never uses. The same trade `crates/knf-core/tests/common/mod.rs` already
//! makes.
//!
//! Indices exist because references may *live* inside an array even though they
//! can never *point* into one — `KeyPath` has no bracket syntax, so there is no
//! `${servers[0]}`.

use knf_core::Value;

/// One step of a path into a value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// Renders a path for display: `servers.primary.host`, `tags[0]`.
pub fn render_path(path: &[Seg]) -> String {
    let mut out = String::new();
    for seg in path {
        match seg {
            Seg::Key(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(k);
            }
            Seg::Index(i) => out.push_str(&format!("[{i}]")),
        }
    }
    out
}

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
    fn rendering_mixes_keys_and_indices() {
        assert_eq!(render_path(&[]), "");
        assert_eq!(
            render_path(&[Seg::Key("a".into()), Seg::Key("b".into())]),
            "a.b"
        );
        assert_eq!(
            render_path(&[Seg::Key("tags".into()), Seg::Index(0)]),
            "tags[0]"
        );
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
