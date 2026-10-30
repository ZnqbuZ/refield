//! D35 / D36 / D37: control flow, `await`, and `const fn` with sources written
//! first or last.
#![allow(dead_code)]
use refield::{Refield, refield};

struct Raw {
    a: u32,
    b: u32,
}

#[derive(Refield, Debug, PartialEq)]
struct Pair {
    a: u32,
    b: u32,
}

fn parse(a: &str, b: &str) -> Result<Pair, std::num::ParseIntError> {
    Ok(refield!(Pair { Raw { a: a.parse()?, b: 0 }.., b: b.parse()? }))
}

fn early(raw: Raw, stop: bool) -> Pair {
    refield!(Pair { raw.., b: if stop { return Pair { a: 0, b: 0 } } else { 5 } })
}

fn first_over(limit: u32) -> Pair {
    let mut n = 0;
    loop {
        n += 1;
        let pair = refield!(Pair { Raw { a: n, b: n * 2 }.., a: if n > limit { break Pair { a: n, b: 0 } } else { n } });
        let _ = pair;
    }
}

const fn front_place(raw: Raw) -> Pair {
    refield!(Pair { raw.., a: raw.a + 10 })
}

const fn make_raw() -> Raw {
    Raw { a: 1, b: 2 }
}

const fn front_value() -> Pair {
    refield!(Pair { make_raw().., a: 20 })
}

const fn back_place(raw: Raw) -> Pair {
    refield!(Pair { a: raw.b, ..raw })
}

const PLACE: Pair = front_place(Raw { a: 5, b: 6 });
const VALUE: Pair = front_value();
const BACK: Pair = back_place(Raw { a: 7, b: 8 });

async fn ready<T>(value: T) -> T {
    value
}

async fn build() -> Pair {
    refield!(Pair { ready(Raw { a: 1, b: 2 }).await.., b: ready(30).await })
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    let waker = std::task::Waker::noop();
    let mut context = std::task::Context::from_waker(waker);
    let mut future = std::pin::pin!(future);
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(output) => output,
        std::task::Poll::Pending => unreachable!(),
    }
}

fn assert_send<T: Send>(value: T) -> T {
    value
}

fn main() {
    assert_eq!(parse("1", "2"), Ok(Pair { a: 1, b: 2 }));
    assert!(parse("x", "2").is_err());
    assert!(parse("1", "x").is_err());
    assert_eq!(early(Raw { a: 1, b: 2 }, true), Pair { a: 0, b: 0 });
    assert_eq!(early(Raw { a: 1, b: 2 }, false), Pair { a: 1, b: 5 });
    assert_eq!(first_over(2), Pair { a: 3, b: 0 });
    assert_eq!((PLACE, VALUE, BACK), (Pair { a: 15, b: 6 }, Pair { a: 20, b: 2 }, Pair { a: 8, b: 8 }));
    assert_eq!(block_on(assert_send(build())), Pair { a: 1, b: 30 });
    println!("d35_d37 ok");
}
