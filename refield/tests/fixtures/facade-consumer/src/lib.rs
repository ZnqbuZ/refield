//! Exercises `refield` reached only through a facade re-export.
//!
//! This crate has no direct `refield` dependency entry, so `refield!` cannot
//! resolve a call-site runtime path. The companion macro must fall back to the
//! definition-site `crate` configuration.

use facade::{Refield, refield};

pub struct Raw {
    pub value: u8,
}

#[derive(Refield, Debug, PartialEq, Eq)]
#[refield(crate = facade)]
pub struct Target {
    pub value: u8,
}

pub fn build(raw: Raw) -> Target {
    refield!(Target { ..raw })
}

#[derive(Refield, Debug, PartialEq, Eq)]
#[refield(crate = facade, with = |v| facade::support::bump(v))]
pub struct ConvertedTarget {
    pub value: u8,
}

pub fn build_converted(raw: Raw) -> ConvertedTarget {
    refield!(ConvertedTarget { ..raw })
}

// A carrier routes the written field through `#[derive(Prepare)]`, which must
// be invoked through the same definition-site `crate = facade` path.
pub fn build_with_attribute(raw: Raw) -> Target {
    refield!(Target {
        #[cfg(any())]
        value: missing_name,
        ..raw
    })
}
