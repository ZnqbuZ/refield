#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use refield::{Refield, refieldable};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRecord {
    pub origin: Option<String>,
    pub ttl: u32,
    pub records: Vec<String>,
    pub extra_unused: u64,
}

#[derive(Refield, Debug, Clone, PartialEq, Eq)]
pub struct RootModel {
    pub origin: String,
    #[refield(default = 60)]
    pub ttl: u32,
    #[refield(default)]
    pub records: Vec<String>,
}

pub mod nested {
    use alloc::string::String;
    use refield::{Refield, refieldable};

    // Intentionally shares the name `RootModel` in a submodule to verify that
    // `#[macro_export]` companion macros never collide at the crate root.
    #[derive(Refield, Debug, Clone, PartialEq, Eq)]
    pub struct RootModel {
        pub a: i32,
        pub b: String,
    }

    #[refieldable]
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct GenericModel<T, U> {
        pub first: T,
        #[refield(name = "raw_{}", bound(U: From<&'static str>), with = |s: &'static str| s.into())]
        pub second: U,
        #[refield(skip, default = 99)]
        pub tag: u32,
    }

    #[derive(Refield, Debug, Clone, PartialEq, Eq)]
    pub struct TupleModel(
        pub i32,
        #[refield(with = |s: &'static str| s.into())] pub String,
    );
}

mod private_inner {
    use refield::Refield;

    #[derive(Refield, Debug, Clone, PartialEq, Eq)]
    pub struct HiddenExport {
        pub x: i32,
        pub y: i32,
    }
}

pub use private_inner::HiddenExport;
pub use private_inner::HiddenExport as RenamedExport;

pub mod glob_source {
    use refield::Refield;

    #[derive(Refield, Debug, Clone, PartialEq, Eq)]
    pub struct GlobExport {
        pub left: i32,
        pub right: i32,
    }
}

pub mod glob_reexport {
    pub use crate::glob_source::*;
}

#[refieldable]
pub type SpecializedModel<T> = nested::GenericModel<T, String>;

#[refieldable]
pub type ChainedAlias = SpecializedModel<i64>;

// Cross-crate definition-site helpers: both functions below are private to
// this crate, so the generated helper methods must resolve them here rather
// than at the `refield!` call site in the integration test crate.
fn normalize_host(host: &str) -> String {
    host.trim().to_ascii_lowercase()
}

fn default_retries() -> u8 {
    3
}

#[derive(Refield, Debug, Clone, PartialEq, Eq)]
pub struct CrossCrateModel<T: Default> {
    #[refield(with = |host: &str| normalize_host(host))]
    pub host: String,
    #[refield(default = default_retries())]
    pub retries: u8,
    #[refield(default = T::default())]
    pub payload: T,
}

// A cross-crate alias to a tuple struct: the companion macro must preserve
// the target's tuple shape when constructing through the alias.
#[refieldable]
pub type TupleModelAlias = nested::TupleModel;

/// A `#![no_std]` construction crossing the prepared-carrier path.
pub struct PreparedRaw {
    pub value: u8,
    pub other: u8,
}

#[derive(Refield, Debug, Clone, PartialEq, Eq)]
pub struct PreparedModel {
    #[refield(with = |value: u8| value + 1)]
    pub value: u8,
    pub other: u8,
}

pub fn build_prepared(raw: PreparedRaw) -> PreparedModel {
    refield::refield!(PreparedModel {
        #[cfg(any())]
        value: missing_name,
        ..raw
    })
}

/// `#[non_exhaustive]` forbids struct literals outside this crate, with or
/// without `refield!` (see the compile-fail doctests in `refield/src/lib.rs`).
#[derive(Refield, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct NonExhaustiveModel {
    pub port: u16,
}
