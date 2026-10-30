//! D6 / D14: a value source that borrows a temporary, written first. The
//! source is bound by a `let`, so the borrowed temporary is extended to the
//! end of the expansion in every edition.
#![allow(dead_code)]
use refield::{Refield, refield};

struct S {
    a: u32,
    b: u32,
    spare: String,
}

#[derive(Refield)]
struct Target {
    a: u32,
    b: u32,
    spare: String,
}

fn make_s() -> S {
    S { a: 1, b: 2, spare: String::from("src") }
}

macro_rules! borrowed_s {
    () => {
        { &make_s() }
    };
}

fn main() {
    let t = refield!(Target { &make_s().., spare: String::new() });
    assert_eq!((t.a, t.b), (1, 2));
    let t = refield!(Target { { &make_s() }.., spare: String::new() });
    assert_eq!((t.a, t.b), (1, 2));
    let t = refield!(Target { borrowed_s!().., spare: String::new() });
    assert_eq!((t.a, t.b), (1, 2));
    println!("d06_d14 ok");
}
