//! D25-D30: inference and coercion of written fields; D31/D33/D34: lifetimes,
//! partial moves, boxes, and references, for every source position.
#![allow(dead_code)]
use std::fmt::Display;
use refield::{Refield, refield};

struct Raw {
    n: u8,
    label: String,
    spare: String,
}

fn raw() -> Raw {
    Raw { n: 1, label: "label".into(), spare: "spare".into() }
}

#[derive(Refield)]
struct Coerced<'a> {
    n: u8,
    f: fn(&str) -> usize,
    text: &'a str,
    slice: &'a [u8],
    shown: Box<dyn Display>,
    label: String,
}

fn len(s: &str) -> usize {
    s.len()
}

#[derive(Refield)]
struct Generic<T, U = String> {
    n: u8,
    value: T,
    other: U,
}

fn main() {
    let owned = String::from("owned");
    let bytes = [1u8, 2, 3];

    // D25 / D26: a closure, a deref coercion, an unsizing coercion, a fn item
    // to fn pointer coercion, and a trait object, in every source position.
    let place = raw();
    let a = refield!(Coerced { place.., f: |s| s.len(), text: &owned, slice: &bytes, shown: Box::new(1) });
    let place = raw();
    let b = refield!(Coerced { f: len, text: &owned, slice: &bytes, shown: Box::new("x"), ..place });
    let c = refield!(Coerced { raw().., f: |s| s.len() + 1, text: &owned, slice: &bytes, shown: Box::new(2.5) });
    assert_eq!(((a.f)("ab"), (b.f)("ab"), (c.f)("ab")), (2, 2, 3));
    assert_eq!((a.text, a.slice.len(), a.shown.to_string(), a.label.as_str()), ("owned", 3, "1".into(), "label"));

    // D27 / D29 / D30: generic arguments decided by written fields, overriding
    // the declared default, without any annotation.
    let g = refield!(Generic { raw().., value: 7u64, other: 'c' });
    let _: Generic<u64, char> = g;
    let place = raw();
    let g = refield!(Generic { value: "v", other: 1i8, ..place });
    let _: Generic<&str, i8> = g;

    // D33: a front place moves the automatic fields first; written fields may
    // still move the remaining ones, and unread fields stay in the place.
    let place = raw();
    let t = refield!(Coerced { place.., f: len, text: "t", slice: &[], shown: Box::new(place.spare) });
    assert_eq!((t.n, t.label.as_str(), t.shown.to_string()), (1, "label", "spare".to_string()));

    // A back place lets written fields borrow the whole source first.
    let place = raw();
    let described = |raw: &Raw| format!("{}/{}", raw.label, raw.spare);
    let t = refield!(Coerced { f: len, text: "t", slice: &[], shown: Box::new(described(&place)), ..place });
    assert_eq!(t.shown.to_string(), "label/spare");
    assert_eq!(place.spare, "spare");

    // D34: a boxed place moves out of the box; a reference source copies.
    let boxed = Box::new(raw());
    let t = refield!(Coerced { *boxed.., f: len, text: "t", slice: &[], shown: Box::new(0) });
    assert_eq!(t.label, "label");
    #[derive(Refield)]
    struct Copied {
        n: u8,
        extra: u8,
    }
    let shared = raw();
    let r = &shared;
    let t = refield!(Copied { r.., extra: r.n + 1 });
    assert_eq!((t.n, t.extra), (1, 2));
    let t = refield!(Copied { &shared as &Raw.., extra: 5 });
    assert_eq!((t.n, t.extra), (1, 5));
    println!("d25_d34 ok");
}
