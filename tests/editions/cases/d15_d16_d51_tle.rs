//! D15 / D16 / D51: temporaries borrowed by written fields live exactly as in
//! a native struct literal, for every source position, also when every field
//! is written and the source is a value.
#![allow(dead_code)]
use refield::{Refield, refield};

#[derive(Clone, Copy)]
struct Raw {
    id: u32,
    n: u32,
    text: &'static str,
}

#[derive(Refield)]
struct View<'a> {
    id: u32,
    text: &'a str,
    n: u32,
}

fn raw() -> Raw {
    Raw { id: 1, n: 2, text: "raw" }
}

fn consume(view: View<'_>) -> usize {
    view.text.len()
}

fn main() {
    // D16: held by `let` across statements.
    let place = raw();
    let a = refield!(View { place.., text: &String::from("front place") });
    let b = refield!(View { text: &String::from("back place"), ..place });
    let c = refield!(View { raw().., text: &String::from("front value") });
    // D51: every field written; the last one still borrows a temporary.
    let d = refield!(View { id: 9, n: 9, text: &String::from("all, place"), ..place });
    let e = refield!(View { raw().., id: 9, n: 9, text: &String::from("all, value") });
    assert_eq!(
        (a.text, b.text, c.text, d.text, e.text),
        ("front place", "back place", "front value", "all, place", "all, value")
    );

    // D15: consumed in the same statement.
    assert_eq!(consume(refield!(View { place.., text: &String::from("x") })), 1);
    assert_eq!(refield!(View { text: &String::from("xy"), ..place }).text.len(), 2);
    assert_eq!(refield!(View { raw().., text: &String::from("xyz") }).text.len(), 3);
    assert_eq!(
        consume(refield!(View { raw().., id: 0, n: 0, text: &String::from("last") })),
        4
    );
    println!("d15_d16_d51 ok");
}
