//! What a downstream crate can name and compose without an IR.
use knf::{
    ConfigFormat, ConfigObject, ConfigValue, Cycle, Env, Format, InterpError, InterpOptions,
    Layers, LoadError, MergeError, MergeOptions, PathError, PathLeaf, Problem, ProcessEnv, RefPath,
    STDIN, Seg, Syntax, interpolate, interpolate_with_context, json_or_string, load_layers, merge,
    merge_interpolate, merge_into, render_path, resolve_format, toml_or_string,
};

#[allow(dead_code)]
struct PublicErrors {
    load: LoadError,
    merge: MergeError,
    path: PathError,
    interpolation: InterpError,
    cycle: Cycle,
    problem: Problem,
    syntax: Syntax,
    accumulate: knf::fs::AccumulateError,
    target: knf::fs::AccumulateTargetError,
    glob: knf::glob::GlobError,
    key_glob: knf::glob::KeyGlobError,
}

fn native_pipeline<V: ConfigFormat + std::fmt::Debug>(env: &dyn Env) {
    let _: Format = V::FORMAT;
    let mut object = V::Object::new();
    object.insert("port".into(), V::parse_inline("8080".into()));
    assert!(object.get("port").is_some());
    assert!(object.get_mut("port").is_some());
    assert_eq!(object.iter().count(), 1);
    let layer = V::object(object);
    let mut merged = merge([layer], &MergeOptions::default()).unwrap();
    let overlay = "copy=${port}"
        .parse::<PathLeaf<String>>()
        .unwrap()
        .into_layer::<V>()
        .unwrap();
    merge_into(&mut merged, overlay, &MergeOptions::default()).unwrap();
    let merged = interpolate(merged, env).unwrap();
    assert_eq!(
        kind(merged.as_object().unwrap().get("copy").unwrap()),
        "number"
    );
    let emitted = knf::format::emit(merged, true).unwrap();
    let _: V = knf::format::parse(&emitted, &knf::format::SourceName::Stdin).unwrap();
    let doc = "copy=${port}"
        .parse::<PathLeaf<String>>()
        .unwrap()
        .into_layer::<V>()
        .unwrap();
    let context = "port=5432"
        .parse::<PathLeaf<String>>()
        .unwrap()
        .into_layer::<V>()
        .unwrap();
    let out = interpolate_with_context(doc, &context, env).unwrap();
    assert_eq!(
        out.as_object()
            .unwrap()
            .get("copy")
            .unwrap()
            .stringify()
            .as_deref(),
        Some("5432")
    );
    assert!(out.as_object().unwrap().get("port").is_none());
    let doc = "service.extends=${defaults}"
        .parse::<PathLeaf<String>>()
        .unwrap()
        .into_layer::<V>()
        .unwrap();
    let context = "defaults.port=5432"
        .parse::<PathLeaf<String>>()
        .unwrap()
        .into_layer::<V>()
        .unwrap();
    let out = merge_interpolate(
        [doc],
        Some(&context),
        env,
        &InterpOptions {
            merge_key: Some("extends".into()),
            shallow: None,
        },
    )
    .unwrap();
    let service = out
        .as_object()
        .unwrap()
        .get("service")
        .unwrap()
        .as_object()
        .unwrap();
    assert!(service.get("extends").is_none());
    assert_eq!(
        service.get("port").unwrap().stringify().as_deref(),
        Some("5432")
    );
}

#[test]
fn public_interfaces_support_both_native_types() {
    struct Empty;
    impl Env for Empty {
        fn lookup(&self, _: &str) -> Option<String> {
            None
        }
    }
    native_pipeline::<serde_json::Value>(&Empty);
    native_pipeline::<toml::Value>(&Empty);
    let _: &dyn Env = &ProcessEnv;
    assert_eq!(json_or_string("null".into()), serde_json::Value::Null);
    assert_eq!(
        toml_or_string("null".into()),
        toml::Value::String("null".into())
    );
    let path: RefPath = "servers[0].host".parse().unwrap();
    assert_eq!(render_path(path.segs()), "servers[0].host");
    let _: &[Seg] = path.segs();
    assert!(matches!(
        path.try_into_keys(),
        Err(PathError::IndexInKeyPath { .. })
    ));
    assert_eq!(STDIN, "-");
    assert_eq!(resolve_format::<&str>(&[], None).unwrap(), Format::Json);
    let Layers::Json(layers) = load_layers::<&str>(&[], None).unwrap() else {
        panic!("default")
    };
    assert!(layers.is_empty());
    let target = knf::fs::AccumulateTarget::try_from(std::path::PathBuf::from("a.json")).unwrap();
    let _: knf::fs::GlobPattern = "*.json".parse().unwrap();
    let glob: knf::glob::GlobPattern = "*.json".parse().unwrap();
    assert!(glob.matches("config.json"));
    let selector: knf::glob::KeyGlobPattern = "db.*".parse().unwrap();
    assert!(selector.matches_keys(&["db".into(), "pool".into()]));
    let _ = target;
}

fn kind<V: ConfigValue>(value: &V) -> &'static str {
    value.kind()
}
