//! The clap adapter for shared file discovery.

use clap::builder::{OsStringValueParser, TypedValueParser, ValueParser};
use knf::fs::AccumulateTarget;

/// Validate native target paths before any filesystem access.
pub fn parser() -> ValueParser {
    OsStringValueParser::new()
        .try_map(|raw| {
            AccumulateTarget::try_from(std::path::PathBuf::from(raw))
                .map_err(super::explain::explain_accumulate_target)
        })
        .into()
}
