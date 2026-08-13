//! Type-conflict detection for `--strict`.
//!
//! Strict mode catches the class of mistake where a leaf accidentally shadows a
//! subtree. Pleasant side effect: it rejects exactly the type changes that break
//! associativity, so under strict mode the merge *is* associative.

use crate::MergeError;

/// Errors if `over` would change the kind of the existing value `base`.
pub(crate) fn check(
    expected: &'static str,
    found: &'static str,
    path: &[String],
) -> Result<(), MergeError> {
    if expected == found {
        return Ok(());
    }
    Err(MergeError::TypeConflict {
        path: path.to_vec(),
        expected,
        found,
    })
}
