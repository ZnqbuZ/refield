//! A value written last is accepted only without written fields.
#![allow(dead_code)]
use refield::{Refield, refield};

struct Raw {
    a: u32,
    b: u32,
}

fn raw() -> Raw {
    Raw { a: 1, b: 2 }
}

#[derive(Refield, Debug, PartialEq)]
struct Pair {
    #[refield(on(swap), name = "b")]
    a: u32,
    b: u32,
}

fn main() {
    assert_eq!(refield!(Pair { ..raw() }), Pair { a: 1, b: 2 });
    assert_eq!(Pair! { @swap ..raw() }, Pair { a: 2, b: 2 });
    assert_eq!(refield!(Pair { ..Raw { a: 3, b: 4 } }), Pair { a: 3, b: 4 });
    #[cfg(fields)]
    let _ = refield!(Pair { a: 1, ..raw() });
    #[cfg(misplaced)]
    let _ = refield!(Pair { a: 1, raw().. });
    println!("post_value ok");
}
