//! D17 / D20 / D21 / D22 / D23 / D24: evaluation and drop order for every
//! source position, on success, on a panicking written field, and on a
//! failing source.
#![allow(dead_code)]
use std::cell::RefCell;
use refield::{Refield, refield};

thread_local! {
    static LOG: RefCell<Vec<&'static str>> = RefCell::new(Vec::new());
}

fn note(event: &'static str) {
    LOG.with(|log| log.borrow_mut().push(event));
}

fn take_log() -> Vec<&'static str> {
    LOG.with(|log| log.borrow_mut().drain(..).collect())
}

struct D(&'static str);

impl Drop for D {
    fn drop(&mut self) {
        note(self.0);
    }
}

struct Raw {
    a: D,
    b: D,
    spare: D,
}

fn raw() -> Raw {
    note("source");
    Raw { a: D("drop a"), b: D("drop b"), spare: D("drop spare") }
}

fn fail_raw() -> Result<Raw, ()> {
    note("source fails");
    Err(())
}

#[derive(Refield)]
struct Target {
    #[refield(with = |a| {
        note("auto a");
        a
    })]
    a: D,
    #[refield(with = |b| {
        note("auto b");
        b
    })]
    b: D,
    x: D,
    y: D,
}

impl Drop for Target {
    fn drop(&mut self) {
        note("drop Target");
    }
}

fn written(event: &'static str, drop: &'static str) -> D {
    note(event);
    D(drop)
}

fn boom() -> D {
    note("written y panics");
    panic!("written field panics")
}

fn early(fail: bool) -> Result<D, ()> {
    note("written y returns early");
    if fail { Err(()) } else { Ok(D("drop y")) }
}

fn front_value_question_mark() -> Result<Target, ()> {
    Ok(refield!(Target { raw().., x: written("written x", "drop x"), y: early(true)? }))
}

fn failing_source() -> Result<Target, ()> {
    Ok(refield!(Target { fail_raw()?.., x: written("written x", "drop x"), y: written("written y", "drop y") }))
}

fn back_place_panics(place: Raw) -> Target {
    refield!(Target { x: written("written x", "drop x"), y: boom(), ..place })
}

fn main() {
    std::panic::set_hook(Box::new(|_| note("<panic>")));

    // D17 / D20: a value written first runs first; the remainder drops after
    // the construction, at the end of the expansion.
    let t = refield!(Target { raw().., x: written("written x", "drop x"), y: written("written y", "drop y") });
    note("after");
    std::mem::forget(t);
    assert_eq!(
        take_log(),
        ["source", "auto a", "auto b", "written x", "written y", "drop spare", "after"]
    );

    // A place written first is read before the written fields...
    let place = raw();
    let t = refield!(Target { place.., x: written("written x", "drop x"), y: written("written y", "drop y") });
    std::mem::forget(t);
    assert_eq!(take_log(), ["source", "auto a", "auto b", "written x", "written y"]);
    drop(place.spare);
    assert_eq!(take_log(), ["drop spare"]);

    // ...and a place written last after them, like native update syntax.
    let place = raw();
    let t = refield!(Target { x: written("written x", "drop x"), y: written("written y", "drop y"), ..place });
    std::mem::forget(t);
    assert_eq!(take_log(), ["source", "written x", "written y", "auto a", "auto b"]);
    drop(place.spare);
    take_log();

    // D21 / D22: a panicking written field drops everything evaluated so far
    // in reverse order, then the source remainder; no `Target` is built.
    let result = std::panic::catch_unwind(|| {
        refield!(Target { raw().., x: written("written x", "drop x"), y: boom() })
    });
    assert!(result.is_err());
    assert_eq!(
        take_log(),
        [
            "source", "auto a", "auto b", "written x", "written y panics", "<panic>",
            "drop x", "drop b", "drop a", "drop spare",
        ]
    );

    // The same with `?` in a written field.
    assert!(front_value_question_mark().is_err());
    assert_eq!(
        take_log(),
        [
            "source", "auto a", "auto b", "written x", "written y returns early",
            "drop x", "drop b", "drop a", "drop spare",
        ]
    );

    // D23 / D24: a failing source evaluates nothing else and builds nothing.
    assert!(failing_source().is_err());
    assert_eq!(take_log(), ["source fails"]);

    // A place written last with a panicking written field: the written field
    // ran first, so nothing was moved out of the place yet.
    assert!(std::panic::catch_unwind(|| back_place_panics(raw())).is_err());
    assert_eq!(
        take_log(),
        ["source", "written x", "written y panics", "<panic>", "drop x", "drop a", "drop b", "drop spare"]
    );
    println!("d17_d24 ok");
}
