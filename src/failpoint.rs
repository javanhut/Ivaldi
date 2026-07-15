//! Deterministic crash injection for crash-consistency tests.
//!
//! A failpoint marks a mutation boundary (object write, ref update, store
//! transaction, rename, ...). When the binary is built with the `failpoints`
//! feature and `IVALDI_FAILPOINT=<name>` is set, reaching that failpoint
//! aborts the process — no destructors, no cleanup — simulating power loss
//! at exactly that boundary. `tests/crash_matrix.rs` drives one child
//! process per failpoint and asserts the repository reopens old-or-new.
//!
//! Without the feature (all release builds) `fail_point` compiles to an
//! empty inline function and has zero runtime cost.

#[cfg(feature = "failpoints")]
pub fn fail_point(name: &str) {
    if std::env::var("IVALDI_FAILPOINT").as_deref() == Ok(name) {
        eprintln!("failpoint hit: {name}");
        std::process::abort();
    }
}

#[cfg(not(feature = "failpoints"))]
#[inline(always)]
pub fn fail_point(_name: &str) {}
