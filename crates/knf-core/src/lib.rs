//! Ordered configuration values, layered merging, and reference resolution.

mod env;
pub mod fs;
pub mod glob;
mod inline;
mod interp;
mod merge;
mod path;
mod value;

pub use env::ProcessEnv;
pub use inline::PathLeaf;
pub use interp::{
    Cycle, Env, InterpError, InterpolationOptions, Problem, Syntax, interpolate,
    merge_with_interpolation,
};
#[allow(deprecated)]
pub use merge::merge_layers;
pub use merge::{MergeOptions, merge, merge_configs, merge_into};
pub use path::{PathError, RefPath, Seg, render_path};
pub use toml_datetime::Datetime;
pub use value::{Unsigned, Value};
