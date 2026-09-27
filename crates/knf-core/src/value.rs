//! Native value adapters for the shared merge and interpolation algorithms.

/// An ordered object whose children have the same native value type.
pub trait ConfigObject<V>: Sized + IntoIterator<Item = (String, V)> {
    /// Construct an empty object.
    fn new() -> Self;
    /// Iterate in input/insertion order.
    fn iter<'a>(&'a self) -> impl Iterator<Item = (&'a String, &'a V)>
    where
        V: 'a;
    /// Look up a child by key.
    fn get(&self, key: &str) -> Option<&V>;
    /// Look up a child for replacement or recursion.
    fn get_mut(&mut self, key: &str) -> Option<&mut V>;
    /// Insert a child, preserving an existing key's position.
    fn insert(&mut self, key: String, value: V);
}

/// Structural operations on a native configuration value.
///
/// Scalars stay opaque to merge and traversal. Implementations classify
/// integers and floats as `number`, and render scalars without quoting strings.
pub trait ConfigValue: Clone {
    /// The native object type.
    type Object: ConfigObject<Self>;
    /// A diagnostic kind: object, array, string, number, bool, null or datetime.
    fn kind(&self) -> &'static str;
    /// Borrow an object, if this is one.
    fn as_object(&self) -> Option<&Self::Object>;
    /// Mutably borrow an object, if this is one.
    fn as_object_mut(&mut self) -> Option<&mut Self::Object>;
    /// Consume an object; return the original value for any other kind.
    fn into_object(self) -> Result<Self::Object, Self>;
    /// Borrow an array, if this is one.
    fn as_array(&self) -> Option<&[Self]>;
    /// Borrow a string, if this is one.
    fn as_str(&self) -> Option<&str>;
    /// Wrap a native object.
    fn object(object: Self::Object) -> Self;
    /// Construct an array.
    fn array(items: Vec<Self>) -> Self;
    /// Construct a string.
    fn string(text: String) -> Self;
    /// Render a scalar for an embedded reference. Containers and null reject.
    /// Finite floats keep Rust float spelling; serializers may use another spelling.
    fn stringify(&self) -> Option<String>;
}

impl ConfigObject<serde_json::Value> for serde_json::Map<String, serde_json::Value> {
    fn new() -> Self {
        Self::new()
    }
    fn iter<'a>(&'a self) -> impl Iterator<Item = (&'a String, &'a serde_json::Value)>
    where
        serde_json::Value: 'a,
    {
        self.iter()
    }
    fn get(&self, key: &str) -> Option<&serde_json::Value> {
        self.get(key)
    }
    fn get_mut(&mut self, key: &str) -> Option<&mut serde_json::Value> {
        self.get_mut(key)
    }
    fn insert(&mut self, key: String, value: serde_json::Value) {
        self.insert(key, value);
    }
}

impl ConfigObject<toml::Value> for toml::Table {
    fn new() -> Self {
        Self::new()
    }
    fn iter<'a>(&'a self) -> impl Iterator<Item = (&'a String, &'a toml::Value)>
    where
        toml::Value: 'a,
    {
        self.iter()
    }
    fn get(&self, key: &str) -> Option<&toml::Value> {
        self.get(key)
    }
    fn get_mut(&mut self, key: &str) -> Option<&mut toml::Value> {
        self.get_mut(key)
    }
    fn insert(&mut self, key: String, value: toml::Value) {
        self.insert(key, value);
    }
}

impl ConfigValue for serde_json::Value {
    type Object = serde_json::Map<String, Self>;
    fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }
    fn as_object(&self) -> Option<&<Self as ConfigValue>::Object> {
        self.as_object()
    }
    fn as_object_mut(&mut self) -> Option<&mut <Self as ConfigValue>::Object> {
        self.as_object_mut()
    }
    fn into_object(self) -> Result<<Self as ConfigValue>::Object, Self> {
        match self {
            Self::Object(map) => Ok(map),
            other => Err(other),
        }
    }
    fn as_array(&self) -> Option<&[Self]> {
        self.as_array().map(Vec::as_slice)
    }
    fn as_str(&self) -> Option<&str> {
        self.as_str()
    }
    fn object(object: <Self as ConfigValue>::Object) -> Self {
        Self::Object(object)
    }
    fn array(items: Vec<Self>) -> Self {
        Self::Array(items)
    }
    fn string(text: String) -> Self {
        Self::String(text)
    }
    fn stringify(&self) -> Option<String> {
        match self {
            Self::Bool(b) => Some(b.to_string()),
            Self::Number(n) => Some(if n.is_i64() || n.is_u64() {
                n.to_string()
            } else {
                n.as_f64().map(float).unwrap_or_else(|| n.to_string())
            }),
            Self::String(s) => Some(s.clone()),
            Self::Null | Self::Array(_) | Self::Object(_) => None,
        }
    }
}

impl ConfigValue for toml::Value {
    type Object = toml::Table;
    fn kind(&self) -> &'static str {
        match self {
            Self::String(_) => "string",
            Self::Integer(_) | Self::Float(_) => "number",
            Self::Boolean(_) => "bool",
            Self::Datetime(_) => "datetime",
            Self::Array(_) => "array",
            Self::Table(_) => "object",
        }
    }
    fn as_object(&self) -> Option<&Self::Object> {
        self.as_table()
    }
    fn as_object_mut(&mut self) -> Option<&mut Self::Object> {
        self.as_table_mut()
    }
    fn into_object(self) -> Result<Self::Object, Self> {
        match self {
            Self::Table(map) => Ok(map),
            other => Err(other),
        }
    }
    fn as_array(&self) -> Option<&[Self]> {
        self.as_array().map(Vec::as_slice)
    }
    fn as_str(&self) -> Option<&str> {
        self.as_str()
    }
    fn object(object: Self::Object) -> Self {
        Self::Table(object)
    }
    fn array(items: Vec<Self>) -> Self {
        Self::Array(items)
    }
    fn string(text: String) -> Self {
        Self::String(text)
    }
    fn stringify(&self) -> Option<String> {
        match self {
            Self::String(s) => Some(s.clone()),
            Self::Integer(i) => Some(i.to_string()),
            Self::Float(f) => Some(float(*f)),
            Self::Boolean(b) => Some(b.to_string()),
            Self::Datetime(dt) => Some(dt.to_string()),
            Self::Array(_) | Self::Table(_) => None,
        }
    }
}

fn float(f: f64) -> String {
    if f.is_finite() {
        format!("{f:?}")
    } else if f.is_nan() {
        "nan".into()
    } else if f.is_sign_negative() {
        "-inf".into()
    } else {
        "inf".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConfigFormat;

    fn rendering<V: ConfigFormat>() {
        for (source, expected) in [
            ("true", "true"),
            ("false", "false"),
            ("1", "1"),
            ("1.0", "1.0"),
            ("-0.0", "-0.0"),
            ("1e300", "1e300"),
            ("1e-7", "1e-7"),
        ] {
            assert_eq!(
                V::parse_inline(source.into()).stringify().as_deref(),
                Some(expected)
            );
        }
        assert_eq!(
            V::string("plain".into()).stringify().as_deref(),
            Some("plain")
        );
        assert_eq!(V::array(Vec::new()).stringify(), None);
        assert_eq!(V::object(V::Object::new()).stringify(), None);
    }

    #[test]
    fn embedded_float_spelling_is_independent_of_serialization() {
        let json = serde_json::json!({"v": 1e20});
        let toml = toml::Value::parse_document("v = 1e20").unwrap();
        assert_eq!(json["v"].stringify().as_deref(), Some("1e20"));
        assert_eq!(toml["v"].stringify().as_deref(), Some("1e20"));
        assert_eq!(crate::format::emit(json, false).unwrap(), "{\"v\":1e+20}\n");
        assert_eq!(
            crate::format::emit(toml, false).unwrap(),
            "v = 100000000000000000000.0\n"
        );
    }

    #[test]
    fn native_scalar_rendering_preserves_existing_spellings() {
        rendering::<serde_json::Value>();
        rendering::<toml::Value>();
        assert_eq!(serde_json::Value::Null.stringify(), None);
        assert_eq!(
            serde_json::Value::from(u64::MAX).stringify().as_deref(),
            Some("18446744073709551615")
        );
        for spelling in [
            "inf",
            "-inf",
            "nan",
            "1979-05-27T07:32:00Z",
            "1979-05-27",
            "07:32:00",
        ] {
            assert_eq!(
                toml::Value::parse_inline(spelling.into())
                    .stringify()
                    .as_deref(),
                Some(spelling)
            );
        }
    }
}
