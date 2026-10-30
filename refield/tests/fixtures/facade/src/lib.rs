//! A re-export facade for `refield`.
//!
//! Downstream fixtures depend on this crate only, so the `refield` crate is
//! not a direct dependency of theirs and `proc-macro-crate` cannot name it
//! while expanding the call site.

pub use refield::*;

#[doc(hidden)]
pub mod support {
    /// A definition-site conversion helper used by the consumer fixture.
    pub fn bump(value: u8) -> u8 {
        value + 1
    }
}
