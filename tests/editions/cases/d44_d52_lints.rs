//! D44-D48 / D51 / D52: lint scopes. Each `--cfg` below must fail to compile
//! with the source's own diagnostic; without any `--cfg` the file compiles
//! cleanly under `deny`.
#![allow(dead_code)]
#![deny(deprecated, unused_variables, unfulfilled_lint_expectations)]
use refield::{Refield, refield};

struct Raw {
    a: u32,
    b: u32,
}

#[deprecated]
fn old_raw() -> Raw {
    Raw { a: 1, b: 2 }
}

#[deprecated]
fn old_value() -> u32 {
    3
}

fn raw() -> Raw {
    Raw { a: 1, b: 2 }
}

#[derive(Refield)]
struct Plain {
    a: u32,
    b: u32,
}

// The first automatic field's target is deprecated.
#[derive(Refield)]
struct OldFirst {
    #[deprecated]
    a: u32,
    b: u32,
}

#[allow(deprecated)]
struct OldSourceField {
    #[deprecated]
    a: u32,
    b: u32,
}

#[forbid(deprecated)]
fn clean_under_forbid() -> Plain {
    // D46: a clean schema injects no `allow`, so `forbid` is fine.
    refield!(Plain { raw().., b: 5 })
}

fn main() {
    // D47: a deprecated target field read automatically is allowed only there.
    let t = refield!(OldFirst { raw().., b: 5 });
    let _ = t.b;
    // D45: `expect` on a written field is fulfilled by that field alone.
    let t = refield!(Plain { raw().., #[expect(deprecated)] b: old_value() });
    let _ = (t.a, t.b);
    let _ = clean_under_forbid();

    // D44: the written field's `allow` must not cover the source.
    #[cfg(d44)]
    let _ = refield!(Plain { { let unused = 1; raw() }.., #[allow(unused_variables)] b: { let x = 1; 5 } });
    // D45: the written field's `expect` must not be fulfilled by the source.
    #[cfg(d45)]
    let _ = refield!(Plain { old_raw().., #[expect(deprecated)] b: 5 });
    // D48: a deprecated source field is reported (the target is not deprecated).
    #[cfg(d48)]
    let _ = refield!(Plain { OldSourceField { a: 1, b: 2 }.. });
    // D51: every field written; the last field's `allow` must not cover the source.
    #[cfg(d51)]
    let _ = refield!(Plain { old_raw().., a: 1, #[allow(deprecated)] b: old_value() });
    // D52: the first automatic field's `allow` must not cover the source.
    #[cfg(d52)]
    let _ = refield!(OldFirst { old_raw().., b: 5 });
    println!("d44_d52 ok");
}
