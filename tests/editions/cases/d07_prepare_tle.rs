//! D7: written fields carrying attributes go through the `Prepare` carrier;
//! the source and the written fields' temporaries still live as without it.
#![allow(dead_code)]
use refield::{Refield, refield};

#[derive(Clone, Copy)]
struct Raw {
    id: u32,
    text: &'static str,
}

#[derive(Refield)]
struct View<'a> {
    id: u32,
    text: &'a str,
}

fn raw() -> Raw {
    Raw { id: 1, text: "raw" }
}

fn main() {
    // The Round 6 `s1_ref_block_with_prepare` source, written first.
    let t = refield!(View { { &raw() }.., #[cfg(all())] id: 7 });
    assert_eq!((t.id, t.text), (7, "raw"));

    // Written-field temporaries, held by `let` and consumed in place.
    let place = raw();
    let a = refield!(View { place.., #[cfg(all())] text: &String::from("a") });
    let b = refield!(View { #[cfg(all())] text: &String::from("b"), ..place });
    let c = refield!(View { raw().., #[cfg(all())] text: &String::from("c") });
    // A block written by the user inside the field.
    let d = refield!(View { raw().., #[cfg(all())] text: { &String::from("d") } });
    assert_eq!((a.text, b.text, c.text, d.text), ("a", "b", "c", "d"));
    assert_eq!(refield!(View { raw().., #[cfg(all())] text: &String::from("ee") }).text.len(), 2);
    println!("d07 ok");
}
