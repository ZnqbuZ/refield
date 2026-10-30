//! Errors in reading the source are reported at the source the user wrote,
//! and lints in user-written code are not silenced by the expansion. Each
//! `--cfg` switches on one faulty case.
#![allow(dead_code)]
use refield::{Refield, refield};

struct Raw {
    a: u8,
    b: String,
}

struct Partial {
    a: u8,
}

struct Wrong {
    a: String,
    b: String,
}

#[derive(Refield)]
struct Target {
    a: u8,
    b: String,
}

struct Small {
    a: u8,
}

#[deprecated]
static OLD: Small = Small { a: 1 };

#[derive(Refield)]
struct Legacy {
    #[deprecated]
    a: u8,
}

// The deprecated target field is filled under `#[allow(deprecated)]`; the
// deprecated static the user wrote is still reported.
#[cfg(deprecated_place)]
#[deny(deprecated)]
fn legacy() -> Legacy {
    refield!(Legacy { ..OLD })
}

// A `with` closure keeps the lints of the code the user wrote.
#[cfg(unused_closure)]
#[deny(unused_variables)]
mod closure {
    #[derive(refield::Refield)]
    struct Closure {
        #[refield(with = |x: u8| 5)]
        a: u8,
    }
}

// The helpers' generated names are not reported as user code.
#[deny(non_snake_case, dead_code, unused_variables)]
mod clean {
    #[derive(refield::Refield)]
    pub struct Helped {
        #[refield(with = |a: u8| a + 1)]
        pub a: u8,
        #[refield(skip, default = 7)]
        pub c: u8,
    }
}

fn main() {
    let raw = Raw { a: 1, b: String::new() };
    #[cfg(missing)]
    let _ = refield!(Target { Partial { a: 1 }.. });
    #[cfg(mismatch)]
    let _ = refield!(Target { Wrong { a: String::new(), b: String::new() }.. });
    #[cfg(behind)]
    let _ = {
        let borrowed = &raw;
        refield!(Target { ..*borrowed })
    };
    #[cfg(primitive)]
    let _ = refield!(Target { 5u8.. });
    let small = Small { a: 1 };
    let helped = refield!(clean::Helped { ..small });
    assert_eq!((helped.a, helped.c), (2, 7));
    let target = refield!(Target { ..raw });
    assert_eq!((target.a, target.b.len()), (1, 0));
    println!("source diagnostics ok");
}
