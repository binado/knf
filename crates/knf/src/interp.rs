//! The process environment, as `knf-interp` wants to see it.
//!
//! The only place `std::env` appears. `knf-interp` takes its environment through
//! a trait so that it stays deterministic and testable without touching process
//! state; this is the implementation that does not.

use knf_interp::{Env, EnvValue};

use crate::value;

/// Reads `${env:NAME}` from the real environment.
pub struct ProcessEnv;

impl Env for ProcessEnv {
    fn lookup(&self, name: &str) -> Option<EnvValue> {
        // `var` also fails on a non-UTF-8 value, which reads here as unset. That
        // is the only outcome available: bytes that are not UTF-8 cannot be
        // spliced into a JSON or TOML document either way.
        let raw = std::env::var(name).ok()?;
        // The same typing rule as `--set`'s RHS, from the same function — which
        // is the whole consistency argument for typing environment values at
        // all. Going through JSON also means this can never fabricate a
        // `Datetime`: JSON has no such type.
        let typed = value::from_json(knf_dotted::json_or_string(raw.clone()));
        Some(EnvValue { raw, typed })
    }
}
