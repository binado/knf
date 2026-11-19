//! Process environment adapter.

use crate::Env;

/// [`Env`] implementation backed by `std::env::var`.
pub struct ProcessEnv;

impl Env for ProcessEnv {
    fn lookup(&self, name: &str) -> Option<String> {
        // Non-UTF-8 environment values are treated as unset.
        std::env::var(name).ok()
    }
}
