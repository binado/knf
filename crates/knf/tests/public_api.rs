//! Public API composition for file and in-memory callers.
use knf::{
    Cycle, InterpError, InterpolationOptions, MergeOptions, PathError, PathLeaf, Problem,
    ProcessEnv, RefPath, Seg, Syntax, Value, render_path,
};
use knf_convert::{InputFormat, LoadError, OutputFormat, Policy};

#[allow(dead_code)]
struct PublicErrors {
    load: LoadError,
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

#[test]
fn both_formats_use_one_pipeline() {
    for format in [InputFormat::Json, InputFormat::Toml] {
        let layer = |expr: &str| -> Value {
            expr.parse::<PathLeaf<String>>()
                .unwrap()
                .map_leaf(|raw| format.parse_inline(raw))
                .try_into()
                .unwrap()
        };
        let opts = MergeOptions::default();
        let context = layer("defaults.port=5432");
        let interp = InterpolationOptions {
            context: Some(&context),
            env: &ProcessEnv,
            parse_inline: &Value::String,
        };
        let mut out = knf::merge_with_interpolation(
            [layer("service=${defaults}"), layer("service.host=x")],
            &opts,
            &interp,
        )
        .unwrap();
        knf::merge_into(&mut out, layer("port=${service.port}"), &opts);
        let out = knf::interpolate(out, &interp).unwrap();
        assert_eq!(out.as_object().unwrap()["port"], Value::Integer(5432));
        let emitted = OutputFormat::from(format)
            .emit(&out, Policy::Strict, true)
            .unwrap();
        assert_eq!(format.parse_document(&emitted.value).unwrap(), out);
    }
    let path: RefPath = "servers[0].host".parse().unwrap();
    assert_eq!(render_path(path.segs()), "servers[0].host");
    let _: &[Seg] = path.segs();
    assert!(matches!(
        path.try_into_keys(),
        Err(PathError::IndexInKeyPath { .. })
    ));
    let target = knf::fs::AccumulateTarget::try_from(std::path::PathBuf::from("a.json")).unwrap();
    let _: knf::fs::GlobPattern = "*.json".parse().unwrap();
    let glob: knf::glob::GlobPattern = "*.json".parse().unwrap();
    assert!(glob.matches("config.json"));
    let selector: knf::glob::KeyGlobPattern = "db.*".parse().unwrap();
    assert!(selector.matches_keys(&["db".into(), "pool".into()]));
    let _ = target;
}
