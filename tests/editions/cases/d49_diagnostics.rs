//! D49: errors in written fields are reported at the written tokens, as in a
//! native literal. Each `--cfg` switches on one faulty construction.
#![allow(dead_code)]
use refield::{Refield, refield};

#[derive(Clone, Copy)]
struct Raw {
    a: u8,
    b: u8,
}

#[derive(Refield)]
struct Target {
    a: u8,
    b: u8,
}

fn main() {
    let raw = Raw { a: 1, b: 2 };
    #[cfg(d49_unknown)]
    let _ = refield!(Target { raw.., missing: 3 });
    #[cfg(d49_duplicate)]
    let _ = refield!(Target { raw.., a: 3, a: 4 });
    #[cfg(d49_type)]
    let _ = refield!(Target { raw.., a: "three" });
    // A malformed attribute on a written field is reported at its `cfg`,
    // twice, like every malformed attribute: once in the `Prepare` carrier,
    // and once in the final literal, which keeps the attribute that rustc
    // could not apply.
    #[cfg(d49_attr)]
    let _ = refield!(Target {
        raw..,
        #[cfg(a, b)]
        a: 3,
    });
    let target = refield!(Target { raw.., a: 3 });
    assert_eq!((target.a, target.b), (3, 2));
    println!("d49 ok");
}
