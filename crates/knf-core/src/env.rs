//! The only process environment reader; interpolation receives raw text.

use crate::Env;

/// Reads raw text for `${env:NAME}`. The selected native adapter types it.
pub struct ProcessEnv;

impl Env for ProcessEnv {
    fn lookup(&self, name: &str) -> Option<String> {
        // Non-UTF-8 environment values are treated as unset.
        std::env::var(name).ok()
    }
}
