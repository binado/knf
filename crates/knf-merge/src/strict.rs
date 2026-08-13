//! Type-conflict detection for `--strict`.
//!
//! Strict mode catches the class of mistake where a leaf accidentally shadows a
//! subtree. Pleasant side effect: it rejects exactly the type changes that break
//! associativity, so under strict mode the merge *is* associative.

use crate::{MergeError, MergeValue};

/// Errors if `over` would change the kind of the existing value `base`.
pub fn check<V: MergeValue>(base: &V, over: &V, path: &[String]) -> Result<(), MergeError> {
    let (expected, found) = (base.kind(), over.kind());
    if expected == found {
        return Ok(());
    }
    Err(MergeError::TypeConflict {
        path: path.to_vec(),
        expected,
        found,
    })
}
