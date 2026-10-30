# refield

**Cross-type struct update syntax for Rust: spread fields from any source type.**

[![crates.io](https://img.shields.io/crates/v/refield.svg)](https://crates.io/crates/refield)
[![Documentation](https://docs.rs/refield/badge.svg)](https://docs.rs/refield)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/ZnqbuZ/refield/blob/HEAD/LICENSE)

[API documentation](https://docs.rs/refield)

Add `#[derive(Refield)]` to a target struct and construct it with
`refield!(Target { port: raw_port, ..raw })`, where `raw` is a value of a **different**
type. Explicit fields are written as usual; every other field is taken from `raw` by
name, moving it out without `Clone`.

Use it for `TryFrom<Raw>` conversions, mapping protobuf or wire types to domain types,
and any "build this type from that type" code that would otherwise be a wall of
`field: raw.field` lines.

## Why refield?

- **Cross-type spread.** Native `..raw` update syntax requires both sides to be the same
  type. `refield!` lifts that: the source can be any struct with matching field names.
- **Partial moves stay available.** Explicit fields are evaluated *before* `..raw` is
  consumed, so they can move, borrow, or parse individual source fields. The remaining
  fields are still moved automatically.
- **The source is read where it is written.** `..raw` comes last and is read after the
  explicit fields; `make_raw()..` comes first and is evaluated once, before them.
- **No `Clone`, no allocation.** Automatic fields are moved by name. Nothing needs
  `Clone`, `Default`, or a conversion trait unless you ask for one.
- **Fallible conversions fit naturally.** Write `port: raw.port.parse()?` and let the rest
  of the fields flow across with `..raw`, including inside `TryFrom` implementations.
- **Conversions when you want them.** `with`, `name`, `default`, and `skip`
  describe per-field extraction, so the common wire-to-domain mapping is declarative.
  `with` accepts a pseudo-closure shorthand or a function path, so
  `#[refield(name = "off", with = negate)] on: bool` inverts polarity. Typed pseudo-closures
  like `#[refield(with = |s: &str| title_case(s))]` handle
  heterogeneous conversions, `bound(...)` adds per-field `where` predicates, and `const` makes
  the generated helpers `const fn`.
- **Any struct shape.** Named, tuple, and unit structs, and mappings between them;
  generics, type aliases, `const fn`, and `no_std`.

It is a lightweight layer over a struct literal: the macro expands to field accesses and
`let` bindings, so it also works in `const` contexts.

## Quick start

Works on stable Rust, from both edition 2021 and edition 2024 crates.

```toml
[dependencies]
refield = "0.1"
```

```rust
use refield::{Refield, refield};

struct Raw {
    host: &'static str,
    port: u16,
}

#[derive(Refield, Debug, PartialEq)]
struct Config {
    host: String,
    port: u16,
}

fn main() {
    let raw = Raw { host: "localhost", port: 8080 };

    // `host` is converted explicitly; `port` is moved from `raw` by name.
    let config = refield!(Config {
        host: raw.host.to_owned(),
        ..raw
    });

    assert_eq!(config, Config { host: "localhost".into(), port: 8080 });
}
```

The derive also generates a companion macro, so `Config! { ..raw }` works when `Config`
is in scope under its original name.

## What you can do

| Feature | Example | Behavior |
| --- | --- | --- |
| Cross-type update | `refield!(Config { ..raw })` | Automatic fields are moved from `raw` by name. |
| Companion macro | `Config! { ..raw }` | Same body, resolved through the struct's own path. |
| Explicit overrides | `refield!(Config { port: parse(raw.port)?, ..raw })` | Written fields bypass all automatic rules. |
| Source written first | `refield!(Config { make_raw()?.., port: 1 })` | Reads the source before the written fields; any expression may come first. |
| Rename the source | `#[refield(name = "raw_{}")]` | Reads `raw_port` for `port`; `{}` expands to the field name. On a struct, applies to every automatic field. |
| Convert values | `#[refield(with = value)]` | Apply a pseudo-closure helper or function path to extracted values. |
| Heterogeneous conversion | `#[refield(with = \|s: &str\| title_case(s))]` | Names the input type in the pseudo-closure parameter when it differs from the target field type. Also works on the struct. |
| Per-field bounds | `#[refield(bound(T: Trait))]` | Adds `where` predicates to the generated helper, without constraining the struct. Parentheses syntax only. |
| `const` helpers | `#[refield(const)]`, `#[refieldable(const)]` | Generates `const fn` conversion/default helpers, so the call works in `const` contexts. |
| Unwrap or default | `#[refield(default = 60)]` | Uses `Some(value)` from an `Option` source, otherwise the default. |
| Skip + default | `#[refield(skip, default = expr)]` | Never reads the source; initializes from the default. |
| Per-profile rules | `#[refield(on(fast), name = "fast_{}")]` | Declares an alternate rule; `refield!(Config { @fast ..raw })` selects it, and other fields fall back to their default rules. Multiple attributes for the same profile merge. |
| Type aliases | `#[refieldable] type Alias = Target;` | Adds a companion macro under the alias's name. |
| `Self` in impls | `#[refieldable] impl Target { ... }` | Resolves `refield!(Self { ... })` to the impl's type. |
| Associated types | `refield!(<T as Trait>::Assoc as Schema { ..raw })` | Names the schema explicitly; an annotated `impl` resolves `Self::Assoc` for you, and the schema identity check rejects the wrong type. |
| Custom `Deref` wrappers | `refield!(Target { &wrapper as &Raw.., a: 1 })` | Pins the final reference so each `Deref` layer runs once; `&make_wrapper() as &Raw..` also extends the temporary's lifetime. |
| Crate redirect | `#[refield(crate = patches)]` | For renamed or re-exported `refield`. |

## Typical examples

### `TryFrom` with a fallible field

```rust
use core::num::ParseIntError;
use refield::{Refield, refield};

struct Raw {
    port: Option<&'static str>,
    host: &'static str,
}

#[derive(Refield, Debug, PartialEq)]
struct Config {
    port: u16,
    #[refield(with = |host: &'static str| host.into())]
    host: String,
}

impl TryFrom<Raw> for Config {
    type Error = ParseIntError;

    fn try_from(raw: Raw) -> Result<Self, Self::Error> {
        Ok(refield!(Config {
            port: raw.port.unwrap_or("0").parse()?,
            ..raw
        }))
    }
}

fn main() {
    let config = Config::try_from(Raw { port: Some("8080"), host: "localhost" }).unwrap();
    assert_eq!(config, Config { port: 8080, host: "localhost".into() });
}
```

`raw.port` is consumed before `..raw` moves `raw.host`, so the fallible conversion and
the automatic spread coexist. Neither type needs `Clone`.

### Struct-level defaults for `name` and `with`

`name = "..."` and `with` can be declared once on the struct
instead of on every field. A struct-level `name` pattern is expanded against each target field,
so `{}` becomes that field's name:

```rust
use refield::{Refield, refield};

struct Raw { raw_port: u32, raw_name: String }

#[derive(Refield, Debug, PartialEq)]
#[refield(name = "raw_{}")]
struct Config {
    port: u32,
    name: String,
}

fn main() {
    let config = refield!(Config { ..Raw { raw_port: 8080, raw_name: "node".into() } });
    assert_eq!(config, Config { port: 8080, name: "node".into() });
}
```

A struct-level `with` is compiled into one helper per automatic field. An untyped pseudo-closure
sees the target field's type, while a typed one names one shared input type:

```rust
use refield::{Refield, refield};

struct Raw { port: u16, retries: u16 }

#[derive(Refield, Debug, PartialEq)]
#[refield(with = |value: u16| u32::from(value) * 2)]
struct Config {
    port: u32,
    retries: u32,
}

fn main() {
    let config = refield!(Config { ..Raw { port: 4, retries: 5 } });
    assert_eq!(config, Config { port: 8, retries: 10 });
}
```

`with` accepts a pseudo-closure shorthand for a helper method, not full Rust closure support,
or a plain function path. The input may be written with `_` to take the field's type:
`#[refield(with = |value: _| value + 1)]`. Any other shape — a parenthesized value, a call or
block expression, or a closure with an `async`, `move`, `const`, or `for<...>` prefix — is a
syntax error where the value starts. A nested placeholder such as `Vec<_>` reaches the helper
signature and is reported by `rustc`.

Field-level attributes always win over struct-level ones: `name`, `with`, and `skip` override their
struct-level counterpart for that field:

```rust
use refield::{Refield, refield};

struct Raw { raw_port: u32, legacy_name: String }

#[derive(Refield, Debug, PartialEq)]
#[refield(name = "raw_{}")]
struct Config {
    port: u32,
    // Reads `legacy_name` instead of `raw_name`.
    #[refield(name = "legacy_name")]
    name: String,
}

fn main() {
    let config = refield!(Config { ..Raw { raw_port: 1, legacy_name: "old".into() } });
    assert_eq!(config, Config { port: 1, name: "old".into() });
}
```

### Unwrap `Option` sources and fill defaults

```rust
use refield::{Refield, refield};

struct Raw {
    ttl: Option<u32>,
    hosts: Option<Vec<&'static str>>,
}

#[derive(Refield, Debug, PartialEq)]
struct Config {
    #[refield(default = 60)]
    ttl: u32,
    #[refield(with = |hosts: Vec<&str>| hosts.into_iter().map(str::to_owned).collect(), default)]
    hosts: Vec<String>,
    #[refield(skip, default = String::from("generated"))]
    tag: String,
}

fn main() {
    let config = refield!(Config {
        ..Raw { ttl: None, hosts: Some(vec!["a", "b"]) }
    });
    assert_eq!(config, Config {
        ttl: 60,
        hosts: vec!["a".into(), "b".into()],
        tag: "generated".into(),
    });
}
```

### Multiple raw layouts with profiles

One target struct can be built from several raw layouts. Fields declare an extra rule per
**profile** with `on(...)`, and the call selects one with `@name` as the first entry of the body.
Fields without a rule for that profile use their default rule.

```rust
use refield::{Refield, refield};

struct Raw {
    host: String,
    fast_host: String,
    port: u16,
    label: &'static str,
}

#[derive(Refield, Debug, PartialEq)]
struct Config {
    #[refield(on(fast), name = "fast_host")]
    host: String,
    #[refield(on(fast), with = |port: u16| port / 2)]
    #[refield(on(thorough), with = |port: u16| port * 2)]
    port: u16,
    // No `on`: this attribute contributes to the default rule.
    #[refield(with = |label: &'static str| label.to_string())]
    #[refield(on(fast, thorough), with = |label: &str| label.to_uppercase())]
    label: String,
    #[refield(skip, default = 1)]
    version: u8,
}

fn main() {
    let raw = || Raw {
        host: "default.example".into(),
        fast_host: "fast.example".into(),
        port: 100,
        label: "node",
    };

    let plain = refield!(Config { ..raw() });
    assert_eq!(plain.host, "default.example");
    assert_eq!(plain.port, 100);
    assert_eq!(plain.label, "node");

    let fast = Config! { @fast ..raw() };
    assert_eq!(fast.host, "fast.example");
    assert_eq!(fast.port, 50);
    assert_eq!(fast.label, "NODE");
}
```

`on(...)` uses parentheses syntax: `on(fast)` or `on(fast, thorough)`. Multiple `#[refield(on(p), ...)]`
attributes for the same profile merge their configuration keys. Attributes without `on(...)`
contribute to the default rule. Specifying conflicting configuration keys for the same profile, or
selecting a profile the struct never declares, is a compile error.

The call-site selector is `@name`; it must be the first entry of the body. The comma after it is
optional, so `{ @fast ..raw }`, `{ @fast, ..raw }`, `{ @fast port: 1, ..raw }`,
`{ @fast raw().., port: 1 }`, and `{ @fast }` are all valid. Omitting `..raw` keeps the same selection rules: fields with a rule for the
selected profile use it, and the rest fall back to their `default` rules.

### Nested `refield!` and cross-layer partial moves

`refield!` is an ordinary expression and nests. An inner call can partial-move one field of the
outer `raw` while the outer call still moves the rest:

```rust
use refield::{Refield, refield};

struct InnerRaw { value: i32, label: &'static str }
struct OuterRaw { name: String, inner: InnerRaw, count: u32 }

#[derive(Refield, Debug, PartialEq)]
struct Inner {
    value: i32,
    #[refield(with = |label: &'static str| label.to_string())]
    label: String,
}

#[derive(Refield, Debug, PartialEq)]
struct Outer {
    name: String,
    inner: Inner,
    count: u32,
}

fn main() {
    let raw = OuterRaw {
        name: "outer".into(),
        inner: InnerRaw { value: 1, label: "inner" },
        count: 2,
    };
    let outer = refield!(Outer {
        // Consumes `raw.inner` field by field...
        inner: refield!(Inner {
            value: raw.inner.value + 10,
            ..raw.inner
        }),
        // ...while the outer spread still moves `raw.name` and `raw.count`.
        ..raw
    });
    assert_eq!(outer.inner.value, 11);
    assert_eq!(outer.name, "outer");
    assert_eq!(outer.count, 2);
}
```

### `Self` diagnostics

`Self` only resolves inside an `impl` block, and only an `impl` annotated with `#[refieldable]`
provides the context macro that resolves it:

- Outside any `impl`, `refield!(Self { ... })` fails because the `__refield_ctx__` macro is not in
  scope.
- Inside an **unannotated** `impl`, `Self` names the implementing type but there is still no context
  macro, so the invocation fails the same way. Annotate the block with `#[refieldable]`, or write
  the concrete type name.

A `#[derive(Refield)]` deliberately does not define a fallback context macro: one per derived type
would shadow the enclosing `impl` context and break `refield!(Self { ... })` as soon as a method
body declares its own derived struct.

### Tuple and unit structs

```rust
use refield::{Refield, refield};

struct Named { x: i32, y: i32 }
struct Tuple(i32, i32);

#[derive(Refield, Debug, PartialEq)]
struct FromNamed(#[refield(name = "x")] i32, #[refield(name = "y")] i32);

#[derive(Refield, Debug, PartialEq)]
struct FromTuple {
    #[refield(name = "0")]
    x: i32,
    #[refield(name = "1")]
    y: i32,
}

#[derive(Refield, Debug, PartialEq)]
struct Unit;

fn main() {
    assert_eq!(refield!(FromNamed { ..Named { x: 1, y: 2 } }), FromNamed(1, 2));
    assert_eq!(refield!(FromTuple { ..Tuple(3, 4) }), FromTuple { x: 3, y: 4 });
    assert_eq!(refield!(Unit {}), Unit);
}
```

Tuple bodies use indices: `refield!(FromNamed { 0: 10, ..raw })`.

### Aliases and `Self`

```rust
use refield::{Refield, refield, refieldable};

struct Raw {
    first: i64,
    raw_second: String,
}

#[derive(Refield, Debug, PartialEq)]
struct Model<T, U> {
    first: T,
    #[refield(name = "raw_{}")]
    second: U,
}

#[refieldable]
type Specialized<T> = Model<T, String>;

#[refieldable]
impl<T: From<u8>> From<Raw> for Model<T, String> {
    fn from(raw: Raw) -> Self {
        let base = refield!(Self { first: T::from(10), ..raw });
        refield!(Self { first: base.first, second: base.second })
    }
}

fn main() {
    let model = refield!(Specialized::<i64> { ..Raw { first: 1, raw_second: "a".into() } });
    assert_eq!(model, Model { first: 1, second: "a".into() });
}
```

An alias without `#[refieldable]` still works by naming the schema after `as`:
`refield!(Plain as Model { ..raw })`. Inside `#[refieldable] impl` blocks, `Self` resolves
to the implementation's type, and associated types defined in the block resolve through
`Self::Assoc` or `<Self as Trait>::Assoc`. Associated types are matched by name, not by the
trait's spelling, so a renamed import such as `use Service as Renamed;` followed by
`refield!(<Self as Renamed>::Config { ..raw })` resolves the same way. What confirms the
choice is the schema identity check on the requested type: requesting an associated type
from a different trait fails to compile unless you name its schema with `as`.

### Associated types

An associated type has no companion macro of its own, but every associated type Rust can
normalize to a struct can still be constructed. Name the schema after `as`:

```rust
use refield::{Refield, refield, refieldable};

struct Raw { port: u16, host: String }

#[derive(Refield, Debug, PartialEq)]
struct ServiceConfig {
    port: u16,
    host: String,
}

trait Service { type Config; }
struct Concrete;
impl Service for Concrete { type Config = ServiceConfig; }

#[refieldable(as = ServiceConfig)]
type MyConfig = <Concrete as Service>::Config;

fn main() {
    // Explicit schema, also usable in a generic function constrained by
    // `T: Service<Config = ServiceConfig>`.
    let raw = Raw { port: 0, host: "a".into() };
    let config = refield!(<Concrete as Service>::Config as ServiceConfig {
        port: 8080,
        ..raw
    });
    assert_eq!(config.port, 8080);

    // Alias bound once at the definition site.
    let config = MyConfig! { ..Raw { port: 9090, host: "b".into() } };
    assert_eq!(config, ServiceConfig { port: 9090, host: "b".into() });
}
```

On an `#[refieldable] impl` block the associated types declared in that block are collected,
so no `as` is needed, and the block's `Self` type does not have to be a nominal struct:

```rust
use refield::{Refield, refield, refieldable};

struct Raw { port: u16 }

#[derive(Refield, Debug, PartialEq)]
struct ServiceConfig { port: u16 }

trait Service {
    type Config;
    fn build(raw: Raw) -> Self::Config;
}

#[refieldable]
impl Service for () {
    type Config = ServiceConfig;

    fn build(raw: Raw) -> Self::Config {
        let base = refield!(Self::Config { port: raw.port + 1, ..raw });
        refield!(<Self as Service>::Config { port: base.port + 1, ..base })
    }
}

fn main() {
    assert_eq!(
        <() as Service>::build(Raw { port: 80 }),
        ServiceConfig { port: 82 },
    );
}
```

A fully unconstrained generic parameter (`fn build<T: Trait>(raw: T::Assoc)`) still cannot
be constructed: Rust itself rejects struct literals for unnormalized associated types, so no
expression-level expansion can help. Adding `T: Trait<Assoc = Schema>` makes it work.

## Behavior to know

- **Evaluation order follows the source position.** With `..rest` last, explicit fields
  are evaluated in written order, then the automatic fields are read from the source. With
  `rest..` first, the source is read first, then the automatic fields, then the explicit
  fields. A *place expression* (`raw`, `inner.raw`, `*boxed`) is accessed directly, field by
  field, which preserves partial moves. A *value expression* such as `make_raw()` is bound
  once, before every field, and whatever is left of it is dropped at the end of the
  `refield!` expression.
- **Value sources come first.** `refield!(Target { a: 1, ..make_raw() })` is rejected: the
  value would have to be evaluated after `a`, yet bound before the automatic fields read it.
  Write `refield!(Target { make_raw().., a: 1 })`, or bind the value to a local and spread
  `..local`. Without written fields, `{ ..make_raw() }` is accepted and means
  `{ make_raw().. }`.
- **Explicit fields keep their native literal position.** A written field is emitted verbatim
  into the target literal, which is the tail of the expansion, so it behaves as in a native
  literal: closures infer their parameter from the field type, `&String` coerces to `&str`,
  a top-level `(expr)` keeps its native `unused_parens` behavior, and temporaries such as
  `&String::from("x")` or a `RefMut` guard live to the end of the enclosing statement, or as
  long as the binding when Rust extends them.
- **Deprecation linting follows the schema.** The macro only emits `#[allow(deprecated)]`
  for automatic fields whose target field is actually marked `#[deprecated]`, so a schema
  without deprecated fields compiles inside a `#[forbid(deprecated)]` scope, and user
  `#[expect(deprecated)]` attributes on written fields cover both the field name and the
  expression.
- **Custom `Deref` wrappers run once with `as &Raw`.** A place expression is projected per
  field, so `..wrapper` or `..*wrapper` calls `Deref::deref` once per field and can mix
  fields from different targets. Write `refield!(Target { &wrapper as &Raw.., a: 1 })` — or
  `&make_wrapper() as &Raw..` for a temporary — to pin the final reference once; deref
  coercion then walks any wrapper depth a single time, and the temporary stays alive until
  the end of the expression. The shared reference can only copy or clone fields, so use the
  plain place form when you need to move non-`Copy` values out of the source.
- **Automatic fields bypass attributes when written explicitly.** `with`,
  `default`, and `skip` only describe extraction; an explicit `field: expr` is used as-is.
  An explicit field keeps its own `#[cfg(...)]` / `#[cfg_attr(..., cfg(...))]`: when the
  condition is false the entry is stripped and the field is extracted automatically, just
  like native struct update syntax.
- **Struct-level defaults are overridable.** `name`, `with`, and
  `const` apply to every automatic field when written on the struct. A field-level `name`,
  `with`, or `skip` overrides them for that field. `skip` fields never read the source, so a struct-level
  `name` pattern does not apply to them.
- **Conversions evaluate at the definition site.** Custom `with` and `default = expr`
  expressions become inherent helper methods on the target struct. They can use the struct's
  generic parameters and lifetimes and any private item of the defining module, but they
  cannot capture locals from the call site. `bound(...)` extends the helper's `where`
  clause, and `const` turns the helper into a `const fn`.
- **Unused source fields.** Fields the target does not declare are ignored. With a place,
  they stay in the source; with a value expression, they are dropped with the temporary.
- **`..rest` must be last**, and no comma may follow it; **`rest..` must be first**, after an
  optional `@profile` selector.
- **Defaults without `..rest`.** `#[refield(default)]` still applies: it initializes the
  field whenever it is not written explicitly.
- **Associated types need a schema.** `<T as Trait>::Assoc { .. }` has no companion macro, so
  name the schema with `as Schema`, annotate the enclosing `impl` with `#[refieldable]`, or
  bind an alias with `#[refieldable(as = Schema)]`. Inside an annotated `impl`, associated
  types resolve by name regardless of how the trait was imported; the schema identity check
  on the requested type rejects a request for a *different* trait's associated type unless
  its schema is named with `as`.
- **Attribute combinations.** `skip` may only combine with `default`; `as` is only valid on type
  aliases.
- **Profiles must be declared.** Selecting `@name` at the call site is an error when no field
  declares a rule for `name`; the selector must be the first entry of the body. Multiple
  attributes for the same profile merge their configuration keys.

### Renamed imports and direct companion calls

`Target! { ... }` is a macro call through the name `Target`. When the type is imported with
`use path::Target as Alias`, the macro is renamed to `Alias` too, but its body still builds
`Target`, which is no longer in scope as a type:

```rust,ignore
// Does not compile:
// use model::Target as Alias;
// let value = Alias! { ..raw };
```

Use `refield!(Alias { ..raw })` instead: it carries the path written at the call site, so
renamed imports work. Annotated `type` aliases are the exception that works both ways,
because `#[refieldable]` generates a forwarding dedicated to that alias.

### Crate paths

Renamed dependencies are detected automatically. For a re-export or a local crate alias,
pass the path explicitly:

```rust,ignore
use refield as patches;

#[derive(patches::Refield)]
#[refield(crate = patches)]
struct Config { port: u16 }
```

`#[refieldable(crate = patches)]` is the equivalent for aliases and impl blocks.

The `crate = ...` path is evaluated where the annotated item is defined. A
`refield!` call expands with the runtime path resolved from its own compilation,
so a locally scoped alias never has to be visible at call sites:

```rust
use refield::{Refield, refield};

mod models {
    use refield as patches;

    #[derive(patches::Refield, Debug, PartialEq)]
    #[refield(crate = patches)]
    pub struct Target { pub value: u8 }
}

fn main() {
    // `patches` is not in scope here.
    let target = refield!(models::Target { value: 1 });
    assert_eq!(target, models::Target { value: 1 });
}
```

If `refield` is reachable only through a facade re-export, the call site has no
direct dependency entry to resolve; give the facade a `crate = ...` argument
naming the facade.

## Development

Use the `refield` crate in your application; it re-exports the macros from
`refield-macros`.

```sh
cargo test --workspace --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
```

Bug reports and usage examples are welcome in
[GitHub issues](https://github.com/ZnqbuZ/refield/issues). Please include a small
reproduction and your Rust version when reporting a problem.

## License

[MIT](https://github.com/ZnqbuZ/refield/blob/HEAD/LICENSE)
