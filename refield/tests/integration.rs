// `#[cfg(all())]` is deliberately used to mark an always-true candidate branch.
#![allow(clippy::non_minimal_cfg)]

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::ops::Deref;
use std::rc::Rc;

use refield::{Refield, refield, refieldable};
use refield_test_models as models;

// ---------------------------------------------------------------------------
// 1. Partial moves, borrow-then-move, non-Clone types, and const evaluation
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
struct NonClone(String);

#[allow(dead_code)]
struct RawMove {
    id: NonClone,
    name: NonClone,
    payload: NonClone,
    extra: NonClone,
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct ParsedMove {
    id_len: usize,
    name: NonClone,
    payload: NonClone,
}

#[test]
fn partial_move_before_and_borrow_during_spread_without_clone() {
    let raw = RawMove {
        id: NonClone("id-001".into()),
        name: NonClone("gateway".into()),
        payload: NonClone("body".into()),
        extra: NonClone("dropped".into()),
    };

    // Move `raw.id` out BEFORE `refield!`.
    let moved_id = raw.id;

    // Inside `refield!`, consume `moved_id` AND borrow `&raw.name` and `&raw.payload`,
    // while `..raw` automatically moves `raw.name` and `raw.payload` afterward!
    let parsed = refield!(ParsedMove {
        id_len: moved_id.0.len() + raw.name.0.len() + raw.payload.0.len(),
        ..raw
    });

    assert_eq!(parsed.id_len, 6 + 7 + 4);
    assert_eq!(parsed.name, NonClone("gateway".into()));
    assert_eq!(parsed.payload, NonClone("body".into()));
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct ConstTarget {
    a: i32,
    b: i32,
}

struct ConstRaw {
    a: i32,
    b: i32,
}

const fn build_const(raw: ConstRaw) -> ConstTarget {
    refield!(ConstTarget {
        a: raw.a + 10,
        ..raw
    })
}

#[test]
fn works_in_const_fn() {
    const RESULT: ConstTarget = build_const(ConstRaw { a: 5, b: 20 });
    assert_eq!(RESULT, ConstTarget { a: 15, b: 20 });
}

// ---------------------------------------------------------------------------
// 2. Field attributes: `name`, `from`, `with`, `default`, `skip`
// ---------------------------------------------------------------------------

struct RawConfig {
    origin: Option<String>,
    ttl: u32,
    zone: Vec<&'static str>,
    raw_port: u16,
    count: i32,
}

#[refieldable]
#[derive(Debug, PartialEq, Eq)]
struct ParsedConfig {
    origin: String,
    #[refield(default = 300)]
    ttl: u32,
    #[refield(name = "zone", with = |v: Vec<&str>| v.into_iter().map(str::to_owned).collect(), default)]
    zones: Vec<String>,
    #[refield(name = "raw_{}", with = |p: u16| p as u32)]
    port: u32,
    #[refield(with = |x: i32| x * 2)]
    count: i32,
    #[refield(skip, default = String::from("cached"))]
    cache_tag: String,
    #[refield(skip)]
    hits: usize,
}

#[test]
fn field_attributes_transform_unwrap_and_skip_as_configured() {
    let raw = RawConfig {
        origin: Some("example.com.".into()),
        ttl: 300,
        zone: vec!["@ IN A 10.0.0.1"],
        raw_port: 5353,
        count: 21,
    };

    let origin = raw.origin.unwrap();
    let parsed = refield!(ParsedConfig { origin, ..raw });

    assert_eq!(
        parsed,
        ParsedConfig {
            origin: "example.com.".into(),
            ttl: 300,
            zones: vec!["@ IN A 10.0.0.1".into()],
            port: 5353,
            count: 42,
            cache_tag: "cached".into(),
            hits: 0,
        }
    );

    // Explicitly overriding defaulted or skipped fields bypasses the auto rules.
    let raw2 = RawConfig {
        origin: None,
        ttl: 60,
        zone: vec![],
        raw_port: 80,
        count: 1,
    };
    let parsed2 = ParsedConfig! {
        origin: "override.test.".into(),
        cache_tag: "custom-tag".into(),
        hits: 99,
        ..raw2
    };
    assert_eq!(parsed2.ttl, 60);
    assert!(parsed2.zones.is_empty());
    assert_eq!(parsed2.port, 80);
    assert_eq!(parsed2.count, 2);
    assert_eq!(parsed2.cache_tag, "custom-tag");
    assert_eq!(parsed2.hits, 99);

    // Omitted fields take their defaults when there is no `..rest`.
    let parsed3 = refield!(ParsedConfig {
        origin: "no-raw.test.".into(),
        zones: vec!["@ IN A 127.0.0.1".into()],
        port: 8080,
        count: 10,
    });
    assert_eq!(parsed3.ttl, 300);
    assert_eq!(parsed3.cache_tag, "cached");
    assert_eq!(parsed3.hits, 0);
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct StructLevelFrom {
    #[refield(with = |a: i32| a as i64)]
    a: i64,
    #[refield(with = |b: &'static str| b.into())]
    b: String,
}

struct StructLevelFromRaw {
    a: i32,
    b: &'static str,
}

#[test]
fn field_level_with_converts_all_remaining_fields() {
    let raw = StructLevelFromRaw {
        a: 7,
        b: "converted",
    };
    let target = refield!(StructLevelFrom { ..raw });
    assert_eq!(
        target,
        StructLevelFrom {
            a: 7,
            b: "converted".into(),
        }
    );
}

// ---------------------------------------------------------------------------
// 3. Tuple structs, unit structs, and named <-> tuple cross-mapping
// ---------------------------------------------------------------------------

#[derive(Refield, Debug, PartialEq, Eq)]
struct TupleTarget(
    i32,
    #[refield(with = |s: &'static str| s.into())] String,
    u8,
);

#[allow(dead_code)]
struct TupleSource(i32, &'static str, u8, bool);

#[derive(Refield, Debug, PartialEq, Eq)]
struct NamedFromTuple {
    #[refield(name = "0")]
    first: i32,
    #[refield(name = "1", with = |s: &'static str| s.into())]
    second: String,
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct TupleFromNamed(
    #[refield(name = "a")] i64,
    #[refield(name = "b", with = |s: &'static str| s.into())] String,
);

#[derive(Refield, Debug, PartialEq, Eq)]
struct UnitTarget;

#[test]
fn tuple_and_unit_structs_and_cross_shape_mappings() {
    let src = TupleSource(10, "tuple", 3, true);
    let t = refield!(TupleTarget {
        0: src.0 + 5,
        ..src
    });
    assert_eq!(t, TupleTarget(15, "tuple".into(), 3));

    let src2 = TupleSource(42, "named", 0, false);
    let named = refield!(NamedFromTuple { ..src2 });
    assert_eq!(
        named,
        NamedFromTuple {
            first: 42,
            second: "named".into(),
        }
    );

    let named_raw = StructLevelFromRaw {
        a: 99,
        b: "into_tuple",
    };
    let tuple_from_named = refield!(TupleFromNamed {
        0: i64::from(named_raw.a) + 1,
        ..named_raw
    });
    assert_eq!(tuple_from_named, TupleFromNamed(100, "into_tuple".into()));

    let unit = refield!(UnitTarget { ..named_raw });
    assert_eq!(unit, UnitTarget);
    let unit2 = refield!(UnitTarget {});
    assert_eq!(unit2, UnitTarget);
}

// ---------------------------------------------------------------------------
// 4. Visibility, same-module ordering, `use ... as ...`, re-exports, cross-crate
// ---------------------------------------------------------------------------

mod vis_matrix {
    use refield::{Refield, refield};

    // Call BEFORE struct definition in the same module.
    pub fn call_before_definitions() -> i32 {
        let raw = Raw { a: 1, b: 2 };
        let p = PrivStruct! { a: 10, ..raw };
        let c = refield!(CrateStruct { a: 20, ..raw });
        p.a + p.b + c.a + c.b
    }

    #[allow(dead_code)]
    #[derive(Clone, Copy)]
    pub struct Raw {
        pub a: i32,
        pub b: i32,
    }

    #[derive(Refield)]
    struct PrivStruct {
        a: i32,
        b: i32,
    }

    #[derive(Refield)]
    pub(super) struct SuperStruct {
        pub a: i32,
        pub b: i32,
    }

    #[derive(Refield)]
    pub(crate) struct CrateStruct {
        pub a: i32,
        pub b: i32,
    }

    pub mod child {
        use refield::refield;

        pub fn access_parent_private() -> i32 {
            let raw = super::Raw { a: 3, b: 4 };
            let p = refield!(super::PrivStruct { a: 30, ..raw });
            p.a + p.b
        }
    }
}

#[test]
fn visibility_and_import_renaming_and_cross_crate_work() {
    assert_eq!(vis_matrix::call_before_definitions(), 12 + 22);
    assert_eq!(vis_matrix::child::access_parent_private(), 34);

    // `pub(super)` and `pub(crate)` via `use ... as ...`
    use vis_matrix::CrateStruct as LocalCrateAlias;
    use vis_matrix::SuperStruct as LocalSuperAlias;
    let raw = vis_matrix::Raw { a: 5, b: 6 };
    let s = refield!(LocalSuperAlias { a: 50, ..raw });
    let c = refield!(LocalCrateAlias { a: 60, ..raw });
    assert_eq!((s.a, s.b, c.a, c.b), (50, 6, 60, 6));

    // Inside a function body: call before and after local struct definition
    let in_fn_val = {
        let before = refield!(FnLocal { a: 7, ..raw });
        #[derive(Refield)]
        struct FnLocal {
            a: i32,
            b: i32,
        }
        let after = FnLocal! { a: 8, ..raw };
        before.a + before.b + after.a + after.b
    };
    assert_eq!(in_fn_val, 13 + 14);

    // Cross-crate: root model, submodule model with same struct name `RootModel`,
    // private-module re-export, renamed re-export, and glob re-export.
    use models::HiddenExport as LocalHidden;
    use models::RenamedExport as DoublyRenamed;
    use models::RootModel;
    use models::glob_reexport::*;
    use models::nested::RootModel as NestedRootModel;

    let raw_rec = models::RawRecord {
        origin: Some("et.net.".into()),
        ttl: 60,
        records: vec![],
        extra_unused: 123,
    };
    let origin = raw_rec.origin.unwrap();
    let root = refield!(RootModel { origin, ..raw_rec });
    assert_eq!(
        root,
        RootModel {
            origin: "et.net.".into(),
            ttl: 60,
            records: vec![],
        }
    );

    #[allow(dead_code)]
    struct NestedRaw {
        a: i32,
        b: String,
    }
    let nraw = NestedRaw {
        a: 1,
        b: "sub".into(),
    };
    let nested_root = refield!(NestedRootModel { a: 99, ..nraw });
    assert_eq!(
        nested_root,
        NestedRootModel {
            a: 99,
            b: "sub".into(),
        }
    );

    #[allow(dead_code)]
    struct XY {
        x: i32,
        y: i32,
        left: i32,
        right: i32,
    }
    let xy = XY {
        x: 1,
        y: 2,
        left: 3,
        right: 4,
    };
    let h1 = refield!(LocalHidden { x: 10, ..xy });
    let h2 = refield!(DoublyRenamed { y: 20, ..xy });
    let g1 = refield!(GlobExport { left: 30, ..xy });
    assert_eq!(h1, models::HiddenExport { x: 10, y: 2 });
    assert_eq!(h2, models::HiddenExport { x: 1, y: 20 });
    assert_eq!(g1, models::glob_source::GlobExport { left: 30, right: 4 });
}

// ---------------------------------------------------------------------------
// 5. Generics, turbofish, `type` aliases, and `#[refieldable] impl` (`Self`)
// ---------------------------------------------------------------------------

struct GenericRaw<T> {
    first: T,
    raw_second: &'static str,
}

#[refieldable]
type LocalSpecialized<T> = models::nested::GenericModel<T, String>;

#[refieldable]
impl<T: From<u8>> From<GenericRaw<T>> for models::nested::GenericModel<T, String> {
    fn from(raw: GenericRaw<T>) -> Self {
        // Test both `refield!(Self { ... })` and nested `refield!(Self { ... })`
        let base = refield!(Self {
            first: T::from(10),
            ..raw
        });
        refield!(Self {
            first: base.first,
            second: base.second,
            tag: base.tag + 1,
        })
    }
}

#[test]
fn generics_type_aliases_and_impl_self_work() {
    let raw = GenericRaw {
        first: 42_i32,
        raw_second: "hello",
    };
    // Both `Type<Args> { ... }` (without `::`) and `Type::<Args> { ... }` work!
    let m1 = refield!(models::nested::GenericModel < i32, String > { ..raw });
    assert_eq!(m1.first, 42);
    assert_eq!(m1.second, "hello");
    assert_eq!(m1.tag, 99);

    let raw2 = GenericRaw {
        first: 7_i64,
        raw_second: "world",
    };
    let m2 = refield!(LocalSpecialized::<i64> { ..raw2 });
    assert_eq!(m2.first, 7);
    assert_eq!(m2.second, "world");
    assert_eq!(m2.tag, 99);

    // Chained cross-crate type alias
    let raw3 = GenericRaw {
        first: 5_i64,
        raw_second: "chained",
    };
    let m3 = refield!(models::ChainedAlias { ..raw3 });
    assert_eq!(m3.first, 5_i64);
    assert_eq!(m3.second, "chained");

    // Unannotated type alias using explicit `as SchemaPath`
    type UnannotatedAlias = models::nested::GenericModel<i64, String>;
    let raw4 = GenericRaw {
        first: 8_i64,
        raw_second: "explicit-as",
    };
    let m4 = refield!(UnannotatedAlias as models::nested::GenericModel { ..raw4 });
    assert_eq!(m4.first, 8_i64);
    assert_eq!(m4.second, "explicit-as");

    // `#[refieldable] impl` with `Self`
    let raw5 = GenericRaw {
        first: 1_u16,
        raw_second: "from-impl",
    };
    let m5 = models::nested::GenericModel::<u16, String>::from(raw5);
    assert_eq!(m5.first, 10);
    assert_eq!(m5.second, "from-impl");
    assert_eq!(m5.tag, 100);
}

// ---------------------------------------------------------------------------
// 6. Strict evaluation order & drop order for place and value sources
// ---------------------------------------------------------------------------

struct DropTracker {
    label: &'static str,
    log: Rc<RefCell<Vec<&'static str>>>,
}

impl Drop for DropTracker {
    fn drop(&mut self) {
        self.log.borrow_mut().push(self.label);
    }
}

#[allow(dead_code)]
struct OrderRaw {
    a: i32,
    b: i32,
    c: i32,
    _guard: DropTracker,
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct OrderTarget {
    b: i32,
    a: i32,
    c: i32,
}

#[test]
fn evaluation_and_drop_order_for_place_and_value_rest_expressions() {
    let log = Rc::new(RefCell::new(Vec::new()));

    let make_raw = || {
        log.borrow_mut().push("make_raw");
        OrderRaw {
            a: 1,
            b: 2,
            c: 3,
            _guard: DropTracker {
                label: "drop_guard",
                log: log.clone(),
            },
        }
    };

    // A value source written first, `make_raw()..`:
    // 1. `make_raw()` is evaluated ONCE, before anything else.
    // 2. The remaining field `c` is moved out of it, then the explicit fields
    //    (`a`, then `b` in written order) are evaluated.
    // 3. Unused `_guard` is dropped at the end of the `refield!` expression.
    let target = refield!(OrderTarget {
        make_raw()..,
        a: {
            log.borrow_mut().push("eval_a");
            10
        },
        b: {
            log.borrow_mut().push("eval_b");
            20
        },
    });
    log.borrow_mut().push("after_refield");

    assert_eq!(target, OrderTarget { b: 20, a: 10, c: 3 });
    assert_eq!(
        *log.borrow(),
        vec![
            "make_raw",
            "eval_a",
            "eval_b",
            "drop_guard",
            "after_refield"
        ]
    );

    // `make_raw()..` when ALL fields are explicitly overridden (`reads_from_rest == 0`):
    // `make_raw()` must still be evaluated once, before the explicit fields.
    log.borrow_mut().clear();
    let target_all = refield!(OrderTarget {
        make_raw()..,
        a: {
            log.borrow_mut().push("eval_a");
            100
        },
        b: {
            log.borrow_mut().push("eval_b");
            200
        },
        c: {
            log.borrow_mut().push("eval_c");
            300
        },
    });
    assert_eq!(
        target_all,
        OrderTarget {
            b: 200,
            a: 100,
            c: 300
        }
    );
    assert_eq!(
        *log.borrow(),
        vec!["make_raw", "eval_a", "eval_b", "eval_c", "drop_guard"]
    );
}

type EventLog = std::sync::Mutex<Vec<&'static str>>;

// Attribute expressions resolve at the definition site, so the log has to be
// reachable from module scope to be named by `#[refield(...)]`.
static LOG: EventLog = EventLog::new(Vec::new());

fn record(log: &EventLog, event: &'static str) {
    log.lock().unwrap().push(event);
}

struct EventGuard(&'static EventLog);

impl Drop for EventGuard {
    fn drop(&mut self) {
        record(self.0, "drop raw");
    }
}

#[allow(dead_code)]
struct DeferredRaw {
    a: i32,
    b: i32,
    _guard: EventGuard,
}

fn make_raw(log: &'static EventLog) -> DeferredRaw {
    record(log, "make_raw");
    DeferredRaw {
        a: 1,
        b: 2,
        _guard: EventGuard(log),
    }
}

fn make_default(log: &EventLog) -> i32 {
    record(log, "make_default");
    7
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct DeferredTarget {
    a: i32,
    b: i32,
    #[refield(skip, default = make_default(&LOG))]
    c: i32,
}

#[test]
fn value_rest_is_dropped_after_automatic_defaults_run() {
    LOG.lock().unwrap().clear();

    // Every remaining field is either explicitly provided (`a`, `b`) or
    // skipped with a default (`c`), so nothing is read from `make_raw(&LOG)..`.
    // The source is still evaluated first and must stay alive until the
    // automatic default expression and the explicit fields have run, so it is
    // dropped only at the end of the expansion.
    let target = refield!(DeferredTarget {
        make_raw(&LOG)..,
        a: {
            record(&LOG, "explicit field");
            10
        },
        b: 20,
    });

    assert_eq!(target, DeferredTarget { a: 10, b: 20, c: 7 });
    assert_eq!(
        *LOG.lock().unwrap(),
        vec!["make_raw", "make_default", "explicit field", "drop raw"]
    );
}

// ---------------------------------------------------------------------------
// 7. Conditional compilation (`#[cfg]`, `#[cfg_attr]`), deprecated, hygiene
// ---------------------------------------------------------------------------

mod reexport {
    pub use refield::*;
}

#[reexport::refieldable(crate = reexport)]
#[derive(Debug, PartialEq, Eq)]
struct CfgTarget {
    #[cfg(any())]
    #[refield(invalid_attr)]
    removed: NonExistentType,
    #[cfg_attr(all(), refield(name = "raw_{}", with = |x: i32| x as i64))]
    active: i64,
    #[allow(non_snake_case)]
    __refield_source: i32,
    r#type: String,
}

#[reexport::refieldable(crate = reexport)]
#[cfg(any())]
struct RemovedStruct {
    field: NonExistentType,
}

#[reexport::refieldable(crate = reexport)]
#[cfg(any())]
type RemovedAlias = NonExistentType;

#[derive(Refield, Debug, PartialEq, Eq)]
struct DeprecatedTarget {
    #[deprecated(note = "legacy field")]
    old: i32,
    current: i32,
}

#[test]
#[deny(deprecated)]
fn cfg_deprecated_raw_idents_and_hygiene_work() {
    struct CfgRaw {
        raw_active: i32,
        __refield_source: i32,
        r#type: String,
    }
    let raw = CfgRaw {
        raw_active: 11,
        __refield_source: 22,
        r#type: "keyword".into(),
    };
    let target = reexport::refield!(CfgTarget { ..raw });
    assert_eq!(
        target,
        CfgTarget {
            active: 11,
            __refield_source: 22,
            r#type: "keyword".into(),
        }
    );

    // Automatic transfer of a `#[deprecated]` field does not trigger `deprecated` lint under `#[deny(deprecated)]`.
    #[allow(dead_code)]
    struct DepRaw {
        old: i32,
        current: i32,
    }
    let dep = refield!(DeprecatedTarget {
        DepRaw { old: 1, current: 2 }..,
        current: 5,
    });
    assert_eq!(dep.current, 5);
}

// ---------------------------------------------------------------------------
// 8. Place expressions through deref, shorthand fields, fully-overridden rest
// ---------------------------------------------------------------------------

#[derive(Refield, Debug, PartialEq, Eq)]
struct BoxTarget {
    a: i32,
    b: String,
    c: NonClone,
}

#[allow(dead_code)]
struct BoxSource {
    a: i32,
    b: String,
    c: NonClone,
    unused: NonClone,
}

#[test]
fn place_rest_through_box_deref_supports_partial_move_and_spread() {
    let boxed_raw = Box::new(BoxSource {
        a: 1,
        b: "boxed".into(),
        c: NonClone("boxed-c".into()),
        unused: NonClone("dropped".into()),
    });

    // `(*boxed_raw)` is a place expression: the remaining fields are moved out
    // of the box directly (Box supports moving out through deref), while the
    // explicitly overridden field is evaluated first.
    let target = refield!(BoxTarget {
        a: boxed_raw.a + 10,
        ..(*boxed_raw)
    });

    assert_eq!(target.a, 11);
    assert_eq!(target.b, "boxed");
    assert_eq!(target.c, NonClone("boxed-c".into()));
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct ShorthandTarget {
    a: i32,
    b: String,
    c: i32,
}

#[allow(dead_code)]
struct ShorthandRaw {
    a: i32,
    b: String,
    c: i32,
}

#[test]
fn shorthand_field_initialization_mixes_with_spread() {
    let a = 40;
    let raw = ShorthandRaw {
        a: 1,
        b: "shorthand".into(),
        c: 3,
    };

    // `a` is shorthand field init; `b` and `c` come from `..raw`.
    let target = refield!(ShorthandTarget { a, ..raw });
    assert_eq!(
        target,
        ShorthandTarget {
            a: 40,
            b: "shorthand".into(),
            c: 3,
        }
    );

    // Shorthand also works through the companion macro.
    let a = 41;
    let raw = ShorthandRaw {
        a: 2,
        b: "companion".into(),
        c: 4,
    };
    let target = ShorthandTarget! { a, ..raw };
    assert_eq!(
        target,
        ShorthandTarget {
            a: 41,
            b: "companion".into(),
            c: 4,
        }
    );
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct FullyOverriddenTarget {
    a: i32,
    b: i32,
    #[refield(default = 7)]
    c: i32,
}

#[allow(dead_code)]
struct FullyOverriddenRaw {
    a: i32,
    b: i32,
    c: i32,
    untouched: i32,
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct NoRestTarget {
    a: i32,
    #[refield(default = 7)]
    b: i32,
    #[refield(skip, default = String::from("skipped"))]
    c: String,
}

#[test]
fn defaults_fill_fields_when_rest_is_omitted() {
    // `b` and `c` are absent from the body and there is no `..rest` to read them from.
    let target = refield!(NoRestTarget { a: 1 });
    assert_eq!(
        target,
        NoRestTarget {
            a: 1,
            b: 7,
            c: "skipped".into(),
        }
    );
}

#[test]
#[deny(unused_variables)]
fn fully_overridden_place_rest_stays_warning_free() {
    let raw = FullyOverriddenRaw {
        a: 1,
        b: 2,
        c: 3,
        untouched: 4,
    };

    // Every field is explicit, so `reads_from_rest == 0`; the generated code
    // must still consume the place to avoid an `unused_variables` warning.
    let target = refield!(FullyOverriddenTarget {
        a: 10,
        b: 20,
        c: 30,
        ..raw
    });

    assert_eq!(
        target,
        FullyOverriddenTarget {
            a: 10,
            b: 20,
            c: 30,
        }
    );
}

// ---------------------------------------------------------------------------
// 9. Associated types: explicit `as Schema`, `#[refieldable] impl`, alias `as`
// ---------------------------------------------------------------------------

trait ServiceTrait {
    type Config;
    fn build(raw: ServiceRaw) -> Self::Config;
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct ServiceConfig {
    port: u16,
    host: String,
}

#[derive(Clone)]
#[allow(dead_code)]
struct ServiceRaw {
    port: u16,
    host: String,
}

struct Service;

// (a) Fully qualified associated type with an explicit schema.
#[test]
fn qualified_associated_type_with_explicit_schema_works() {
    let raw = ServiceRaw {
        port: 8080,
        host: "qualified".into(),
    };
    let config: <Service as ServiceTrait>::Config = refield!(<Service as ServiceTrait>::Config as ServiceConfig {
        port: raw.port + 1,
        ..raw
    });
    assert_eq!(
        config,
        ServiceConfig {
            port: 8081,
            host: "qualified".into(),
        }
    );

    // A plain struct path also works through its companion macro.
    let raw = ServiceRaw {
        port: 9090,
        host: "companion".into(),
    };
    let config = ServiceConfig! { ..raw };
    assert_eq!(config.port, 9090);
}

// Generic functions can only construct `<T as Trait>::Assoc` when the schema is
// named explicitly; the type equality constraint makes the result coerce back.
fn build_generic<T: ServiceTrait<Config = ServiceConfig>>(raw: ServiceRaw) -> T::Config {
    refield!(<T as ServiceTrait>::Config as ServiceConfig {
        port: raw.port + 2,
        ..raw
    })
}

#[test]
fn generic_function_builds_associated_type_with_schema() {
    let config = build_generic::<Service>(ServiceRaw {
        port: 80,
        host: "generic".into(),
    });
    assert_eq!(
        config,
        ServiceConfig {
            port: 82,
            host: "generic".into(),
        }
    );
}

// (b) `#[refieldable] impl` resolves `Self::Assoc` and
// `<Self as Trait>::Assoc` from `ImplItem::Type`.
#[refieldable]
impl ServiceTrait for Service {
    type Config = ServiceConfig;

    fn build(raw: ServiceRaw) -> Self::Config {
        let base = refield!(Self::Config {
            port: raw.port + 10,
            ..raw
        });
        let qualified = refield!(<Self as ServiceTrait>::Config {
            port: base.port + 1,
            host: base.host,
        });
        refield!(Self::Config {
            port: qualified.port + 1,
            ..qualified
        })
    }
}

// `Self` does not need to be a nominal struct: associated type definitions are
// enough to make the block rewriteable.
#[refieldable]
impl ServiceTrait for () {
    type Config = ServiceConfig;

    fn build(raw: ServiceRaw) -> Self::Config {
        let base = refield!(Self::Config {
            port: raw.port + 100,
            ..raw
        });
        refield!(Self::Config {
            port: base.port + 1,
            ..base
        })
    }
}

#[test]
fn refieldable_impl_rewrites_associated_type_constructors() {
    let config = Service::build(ServiceRaw {
        port: 1,
        host: "impl".into(),
    });
    assert_eq!(
        config,
        ServiceConfig {
            port: 13,
            host: "impl".into(),
        }
    );

    let config = <() as ServiceTrait>::build(ServiceRaw {
        port: 2,
        host: "unit".into(),
    });
    assert_eq!(
        config,
        ServiceConfig {
            port: 103,
            host: "unit".into(),
        }
    );
}

// (c) `#[refieldable(as = Schema)]` binds an alias to an associated type.
#[refieldable(as = ServiceConfig)]
type ServiceConfigAlias = <Service as ServiceTrait>::Config;

#[test]
fn alias_with_as_schema_constructs_associated_type() {
    let raw = ServiceRaw {
        port: 443,
        host: "alias".into(),
    };
    let config = refield!(ServiceConfigAlias { ..raw });
    assert_eq!(
        config,
        ServiceConfig {
            port: 443,
            host: "alias".into(),
        }
    );

    let raw = ServiceRaw {
        port: 8443,
        host: "alias-companion".into(),
    };
    let config = ServiceConfigAlias! { ..raw };
    assert_eq!(
        config,
        ServiceConfig {
            port: 8443,
            host: "alias-companion".into(),
        }
    );
}

// The same machinery works for generic associated types in an impl block.
trait GenericAssoc {
    type Item<T>;
    fn build(raw: PairRaw<'_>) -> Self::Item<u8>;
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct PairTarget<T>(T, #[refield(with = |s: &str| s.into())] String);

struct PairRaw<'a>(u8, &'a str);

struct PairService;

#[refieldable]
impl GenericAssoc for PairService {
    type Item<T> = PairTarget<T>;

    fn build(raw: PairRaw<'_>) -> Self::Item<u8> {
        refield!(Self::Item::<u8> { ..raw })
    }
}

#[test]
fn refieldable_impl_handles_generic_associated_types() {
    let pair = PairService::build(PairRaw(3, "gat"));
    assert_eq!(pair, PairTarget(3, "gat".into()));
}

// ---------------------------------------------------------------------------
// 10. `with` transforms: boolean polarity inversion and function-name strings
// ---------------------------------------------------------------------------

fn normalize_name(value: &'static str) -> String {
    value.trim().to_uppercase()
}

struct ToggleRaw {
    enabled: bool,
    name: &'static str,
    raw_port: u16,
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct ToggleTarget {
    #[refield(name = "enabled", with = |enabled: bool| !enabled)]
    disabled: bool,
    #[refield(with = |s: &'static str| normalize_name(s))]
    name: String,
    #[refield(name = "raw_port", with = |p: u16| p as u32)]
    port: u32,
}

#[test]
fn boolean_polarity_inversion_and_string_with_construct_from_raw() {
    let raw = ToggleRaw {
        enabled: true,
        name: "  gateway ",
        raw_port: 8080,
    };
    let target = refield!(ToggleTarget { ..raw });
    assert_eq!(
        target,
        ToggleTarget {
            disabled: false,
            name: "GATEWAY".into(),
            port: 8080,
        }
    );

    // `with = path` passes the function path directly.
    let raw = ToggleRaw {
        enabled: false,
        name: " edge ",
        raw_port: 53,
    };
    let target = ToggleTarget! { ..raw };
    assert!(target.disabled);
    assert_eq!(target.name, "EDGE");
    assert_eq!(target.port, 53);
}

// ---------------------------------------------------------------------------
// 11. Multi-source profile mapping: per-profile rules with default fallback
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct ProfileRaw {
    host: String,
    fast_host: String,
    thorough_host: String,
    port: u16,
    timeout: u32,
    verbose: bool,
    label: &'static str,
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct ProfileTarget {
    // Rules for two profiles; selecting any other profile falls back to `host`.
    #[refield(on(fast), name = "fast_host")]
    #[refield(on(thorough), name = "thorough_host")]
    host: String,
    // The default rule is the identity; each profile transforms it differently.
    #[refield(on(fast), with = |port: u16| port / 2)]
    #[refield(on(thorough), with = |port: u16| port * 2)]
    #[refield(on(minimal), with = |port: u16| port + 1000)]
    port: u16,
    // No profile rule at all: always falls back to the default rule.
    #[refield(default = 30, with = |timeout: u32| timeout + 1)]
    timeout: u32,
    // A single attribute binds the same rule to several profiles; `minimal`
    // is not listed and must fall back to the identity default rule.
    #[refield(on(fast, thorough), with = |verbose: bool| !verbose)]
    verbose: bool,
    // Plain source mapping, no profile involvement.
    #[refield(name = "label", with = |s: &'static str| s.into())]
    label: String,
}

fn profile_raw() -> ProfileRaw {
    ProfileRaw {
        host: "default.example".into(),
        fast_host: "fast.example".into(),
        thorough_host: "thorough.example".into(),
        port: 100,
        timeout: 29,
        verbose: true,
        label: "profile",
    }
}

#[test]
fn profile_rules_select_sources_and_fall_back_to_defaults() {
    // No profile: every field uses its default rule.
    let target = refield!(ProfileTarget { ..profile_raw() });
    assert_eq!(target.host, "default.example");
    assert_eq!(target.port, 100);
    // `None` takes the declared default; `with` only transforms `Some` values.
    assert_eq!(target.timeout, 30);
    assert!(target.verbose);
    assert_eq!(target.label, "profile");

    // `fast`: dedicated sources and transforms, plus defaults for `timeout`
    // and `label`, which declare no `fast` rule.
    let target = refield!(ProfileTarget { @fast ..profile_raw() });
    assert_eq!(target.host, "fast.example");
    assert_eq!(target.port, 50);
    assert_eq!(target.timeout, 30);
    assert!(!target.verbose);
    assert_eq!(target.label, "profile");

    // Companion-macro spelling of a different profile.
    let target = ProfileTarget! { @thorough ..profile_raw() };
    assert_eq!(target.host, "thorough.example");
    assert_eq!(target.port, 200);
    assert_eq!(target.timeout, 30);
    assert!(!target.verbose);

    // A profile that only some fields know about: `port` has a `minimal`
    // rule, every other field falls back to its default rule.
    let target = refield!(ProfileTarget { @minimal ..profile_raw() });
    assert_eq!(target.host, "default.example");
    assert_eq!(target.port, 1100);
    assert_eq!(target.timeout, 30);
    assert!(target.verbose);
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct KeywordProfileTarget {
    // A keyword is a valid profile name; the internal schema must accept it too.
    #[refield(on(r#type), with = |value: u16| value + 1)]
    value: u16,
}

#[allow(dead_code)]
struct KeywordProfileRaw {
    value: u16,
}

#[test]
fn keyword_profile_names_work_through_the_internal_schema() {
    let raw = KeywordProfileRaw { value: 41 };
    let target = refield!(KeywordProfileTarget { @r#type, ..raw });
    assert_eq!(target, KeywordProfileTarget { value: 42 });
}

#[test]
fn profile_rules_leave_explicit_fields_untouched() {
    let raw = ProfileRaw {
        port: 8,
        timeout: 5,
        verbose: false,
        ..profile_raw()
    };
    let target = ProfileTarget! {
        @fast,
        host: "explicit.example".into(),
        timeout: 99,
        ..raw
    };
    // Explicit fields win even when the selected profile has a rule.
    assert_eq!(target.host, "explicit.example");
    assert_eq!(target.timeout, 99);
    assert_eq!(target.port, 4);
    assert!(target.verbose);
}

// `profile` only selects a rule; value expressions still see `raw` normally.
#[test]
fn profile_selection_works_with_a_value_rest_expression() {
    let target = ProfileTarget! {
        @minimal,
        ..ProfileRaw {
            host: "temp.example".into(),
            fast_host: "fast.example".into(),
            thorough_host: "thorough.example".into(),
            port: 16,
            timeout: 7,
            verbose: false,
            label: "temp",
        }
    };
    assert_eq!(target.host, "temp.example");
    assert_eq!(target.port, 1016);
    assert_eq!(target.timeout, 8);
    assert!(!target.verbose);
    assert_eq!(target.label, "temp");
}

// Omitting `..rest` keeps the profile selection: fields with a rule for the
// selected profile use it, and the rest are filled from their `default` rules.
#[derive(Refield, Debug, PartialEq, Eq)]
struct ProfileDefaults {
    #[refield(on(alt), with = |port: u16| port + 1, default = 1)]
    #[refield(default = 2)]
    port: u16,
    #[refield(default = 3)]
    retries: u8,
}

#[test]
fn profile_selection_applies_without_a_rest_expression() {
    // No selector: both fields use their default rules.
    let target = refield!(ProfileDefaults {});
    assert_eq!(
        target,
        ProfileDefaults {
            port: 2,
            retries: 3
        }
    );

    // `@alt` switches only `port`, which has an `alt` rule; `retries` keeps
    // its default rule because it declares no `alt` rule.
    let target = refield!(ProfileDefaults { @alt });
    assert_eq!(
        target,
        ProfileDefaults {
            port: 1,
            retries: 3
        }
    );

    // Explicit fields still win over the selected profile.
    let target = ProfileDefaults! { @alt, port: 9 };
    assert_eq!(
        target,
        ProfileDefaults {
            port: 9,
            retries: 3
        }
    );

    // The comma after the selector is optional in every spelling.
    let target = ProfileDefaults! { @alt port: 10 };
    assert_eq!(
        target,
        ProfileDefaults {
            port: 10,
            retries: 3
        }
    );
}

// ---------------------------------------------------------------------------
// 12. Nested `refield!` and cross-layer partial moves
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct LayerInnerRaw {
    value: i32,
    label: &'static str,
    dropped: NonClone,
}

#[allow(dead_code)]
struct LayerOuterRaw {
    name: String,
    tag: NonClone,
    inner: LayerInnerRaw,
    count: u32,
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct LayerInner {
    value: i32,
    #[refield(with = |s: &'static str| s.into())]
    label: String,
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct LayerOuter {
    name: String,
    tag: NonClone,
    count: u32,
    inner: LayerInner,
}

#[test]
fn nested_refield_expands_inner_and_outer_layers_across_types() {
    let raw = LayerOuterRaw {
        name: "outer".into(),
        tag: NonClone("moved-tag".into()),
        inner: LayerInnerRaw {
            value: 1,
            label: "inner",
            dropped: NonClone("inner-unused".into()),
        },
        count: 2,
    };

    // Partial move a non-`Copy` field out of `raw` before the macro runs.
    let tag = raw.tag;

    let outer = refield!(LayerOuter {
        // Hand the moved value back explicitly, so `..raw` never touches it.
        tag,
        // The inner expansion consumes `raw.inner` field by field while the
        // outer expansion still moves `raw.name` and `raw.count` afterwards.
        inner: refield!(LayerInner {
            value: raw.inner.value + 10,
            ..raw.inner
        }),
        ..raw
    });

    assert_eq!(
        outer,
        LayerOuter {
            name: "outer".into(),
            tag: NonClone("moved-tag".into()),
            count: 2,
            inner: LayerInner {
                value: 11,
                label: "inner".into(),
            },
        }
    );
}

// Profile rules participate in hygiene: a `with` expression may reuse the
// names the macro uses for its own temporaries.
struct HygieneRaw {
    value: u32,
    flag: bool,
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct HygieneTarget {
    #[refield(
        on(fast),
        with = |__refield_value: u32| __refield_value + 1,
        default = 10,
    )]
    #[refield(with = |__refield_value: u32| __refield_value * 2, default = 10)]
    value: u32,
    #[refield(on(fast), with = |__refield_source: bool| !__refield_source)]
    flag: bool,
}

#[test]
fn profile_rules_are_hygienic_against_macro_temporaries() {
    let raw = HygieneRaw {
        value: 4,
        flag: true,
    };
    let target = refield!(HygieneTarget { @fast ..raw });
    assert_eq!(
        target,
        HygieneTarget {
            value: 5,
            flag: false
        }
    );

    let raw = HygieneRaw {
        value: 4,
        flag: true,
    };
    let target = refield!(HygieneTarget { ..raw });
    assert_eq!(
        target,
        HygieneTarget {
            value: 8,
            flag: true
        }
    );

    let target = refield!(HygieneTarget { flag: true });
    assert_eq!(
        target,
        HygieneTarget {
            value: 10,
            flag: true
        }
    );
}

// ---------------------------------------------------------------------------
// 13. `#[cfg]`-gated associated type definitions inside `#[refieldable] impl`
// ---------------------------------------------------------------------------

// Multiple `type Config = ...;` definitions may coexist in one `impl` block as
// long as their `#[cfg]` predicates are mutually exclusive. The rewrite keeps
// every candidate and lets cfg-stripping pick the active one; a `#[cfg(any())]`
// definition is skipped up front so its (deliberately nonexistent) type never
// reaches the companion-macro lookup.
trait CfgAssoc {
    #[allow(dead_code)]
    type Config;
    fn build(raw: ServiceRaw) -> Self::Config;
}

struct CfgService;

#[refieldable]
impl CfgAssoc for CfgService {
    #[cfg(all())]
    type Config = ServiceConfig;
    #[cfg(any())]
    type Config = NonExistentType;

    fn build(raw: ServiceRaw) -> Self::Config {
        let base = refield!(Self::Config {
            port: raw.port + 1,
            ..raw
        });
        refield!(Self::Config { ..base })
    }
}

#[test]
fn refieldable_impl_resolves_cfg_gated_associated_type() {
    let config = CfgService::build(ServiceRaw {
        port: 41,
        host: "cfg".into(),
    });
    assert_eq!(
        config,
        ServiceConfig {
            port: 42,
            host: "cfg".into(),
        }
    );
}

// A second, always-true candidate spelled with a non-trivial predicate
// (`#[cfg(not(any()))]`) is kept alongside the active one. Both the companion
// form and the qualified `refield!` form must still resolve to the active
// branch.
struct CfgServiceMulti;

#[refieldable]
impl CfgAssoc for CfgServiceMulti {
    #[cfg(not(any()))]
    type Config = ServiceConfig;
    #[cfg(any())]
    type Config = NonExistentType;

    fn build(raw: ServiceRaw) -> Self::Config {
        let _ = refield!(<Self as CfgAssoc>::Config { ..raw.clone() });
        refield!(Self::Config { ..raw })
    }
}

#[test]
fn refieldable_impl_resolves_multibranch_cfg_associated_type() {
    let config = CfgServiceMulti::build(ServiceRaw {
        port: 7,
        host: "multi".into(),
    });
    assert_eq!(
        config,
        ServiceConfig {
            port: 7,
            host: "multi".into(),
        }
    );
}

// Two candidates whose predicates are not statically decidable stay in the
// list, so the rewrite emits one `#[cfg]`-guarded branch per candidate and
// lets cfg-stripping select exactly one. The inactive branch's macro is never
// expanded, so its (deliberately nonexistent) target type is harmless.
struct CfgServiceTwoLive;

#[refieldable]
impl CfgAssoc for CfgServiceTwoLive {
    #[cfg(all())]
    type Config = ServiceConfig;
    #[cfg(not(all()))]
    type Config = NonExistentType;

    fn build(raw: ServiceRaw) -> Self::Config {
        let base = refield!(Self::Config {
            port: raw.port + 1,
            ..raw
        });
        // The path-qualified spelling shares the resolution.
        refield::refield!(<Self as CfgAssoc>::Config { ..base })
    }
}

#[test]
fn refieldable_impl_expands_two_live_cfg_candidates() {
    let config = CfgServiceTwoLive::build(ServiceRaw {
        port: 8,
        host: "two-live".into(),
    });
    assert_eq!(
        config,
        ServiceConfig {
            port: 9,
            host: "two-live".into(),
        }
    );
}

// ---------------------------------------------------------------------------
// 14. `#[refieldable] type` aliases in nested modules, renames, and chains
// ---------------------------------------------------------------------------

mod alias_scope {
    pub mod schema {
        use refield::{Refield, refieldable};

        pub struct NestedRaw {
            pub port: u16,
            pub host: &'static str,
        }

        #[derive(Refield, Debug, PartialEq, Eq)]
        pub struct NestedConfig {
            pub port: u16,
            #[refield(with = |h: &'static str| h.into())]
            pub host: String,
        }

        // The alias lives in a nested module. The old `$crate::__refield_type_*`
        // bridge looked it up at the crate root and failed here.
        #[refieldable]
        pub type NestedAlias = NestedConfig;
    }

    pub mod chain {
        use super::schema::NestedAlias;
        use refield::refieldable;

        // The target is itself an annotated alias imported into this module;
        // its schema is captured at *this* definition site.
        #[refieldable]
        pub type ChainedAlias = NestedAlias;
    }

    pub mod alias_reexport {
        pub use super::schema::NestedAlias as ReexportedAlias;
    }

    pub mod custom_target {
        // `custom_reexport` is private to this module; the alias below must not
        // need it in scope.
        #[allow(unused_imports)]
        use refield as custom_reexport;
        use refield::Refield;

        #[derive(Refield, Debug, PartialEq, Eq)]
        #[refield(crate = custom_reexport)]
        pub struct CustomConfig {
            pub port: u16,
        }
    }

    pub mod custom_alias {
        use super::custom_target::CustomConfig;
        use refield::refieldable;

        // No `custom_reexport` in scope: forwarding uses the alias definition
        // site's own `refield` path.
        #[refieldable]
        pub type CustomAlias = CustomConfig;
    }
}

#[test]
fn nested_module_aliases_work_through_renames_chains_and_custom_crate_paths() {
    use alias_scope::alias_reexport::ReexportedAlias;
    use alias_scope::chain::ChainedAlias;
    use alias_scope::custom_alias::CustomAlias;
    // The direct companion spelling expands to a struct literal that names the
    // alias defined in `schema`, and `macro_rules!` resolves that path at the
    // invocation site (the same rule as a renamed struct companion macro), so
    // the original name has to be in scope as well. `refield!` carries the
    // caller's spelling and therefore works through `Renamed` alone.
    use alias_scope::schema::NestedAlias;
    use alias_scope::schema::NestedAlias as Renamed;

    let raw = || alias_scope::schema::NestedRaw {
        port: 8080,
        host: "nested",
    };

    // Renamed import through `refield!`.
    let config = refield!(Renamed { ..raw() });
    assert_eq!(
        config,
        alias_scope::schema::NestedConfig {
            port: 8080,
            host: "nested".into(),
        }
    );

    // Direct companion spelling through the renamed macro.
    let config = Renamed! { raw().., port: 9090 };
    assert_eq!(config.port, 9090);
    assert_eq!(config.host, "nested");

    // The original name still works through `refield!` as well.
    let config = refield!(NestedAlias { ..raw() });
    assert_eq!(config.port, 8080);

    // Alias chains across modules, both by short name and fully qualified.
    let config = refield!(ChainedAlias { ..raw() });
    assert_eq!(config.port, 8080);
    let config = refield!(alias_scope::chain::ChainedAlias { raw().., port: 1 });
    assert_eq!(config.port, 1);

    // A re-exported alias keeps working under the re-exported name.
    let config = refield!(ReexportedAlias { ..raw() });
    assert_eq!(config.host, "nested");

    // The target used `#[refield(crate = custom_reexport)]` in a private
    // module; the alias is declared where that name is not in scope.
    struct CustomRaw {
        port: u16,
    }
    let config = refield!(CustomAlias {
        ..CustomRaw { port: 7 }
    });
    assert_eq!(config, alias_scope::custom_target::CustomConfig { port: 7 });
}

// ---------------------------------------------------------------------------
// 15. Schema identity: the requested type must own the schema
// ---------------------------------------------------------------------------

#[derive(Refield, Debug, PartialEq, Eq)]
struct GenericInferenceConfig<T> {
    value: T,
    #[refield(with = |l: &'static str| l.into())]
    label: String,
}

#[allow(dead_code)]
struct GenericInferenceRaw {
    value: u32,
    label: &'static str,
}

trait GenericInferenceService {
    type Config;
    fn build(raw: GenericInferenceRaw) -> Self::Config;
}

struct InferenceService;

#[refieldable]
impl GenericInferenceService for InferenceService {
    // The schema is specialized to `u32`; `as GenericInferenceConfig` below
    // does not repeat `<u32>`.
    type Config = GenericInferenceConfig<u32>;

    fn build(raw: GenericInferenceRaw) -> Self::Config {
        refield!(<Self as GenericInferenceService>::Config as GenericInferenceConfig {
            // `7` is only known to be `u32` through the requested type, so the
            // schema check must feed inference instead of losing it.
            value: 7,
            ..raw
        })
    }
}

#[test]
fn schema_check_preserves_generic_inference_of_the_requested_type() {
    let config = InferenceService::build(GenericInferenceRaw {
        value: 1,
        label: "inferred",
    });
    assert_eq!(
        config,
        GenericInferenceConfig {
            value: 7u32,
            label: "inferred".into(),
        }
    );
    let value: u32 = config.value;
    assert_eq!(value, 7);
}

// ---------------------------------------------------------------------------
// 16. Definition-site helpers: generics, private functions, closures, bounds
// ---------------------------------------------------------------------------

// A `default = expr` helper is defined inside `impl<T> GenericDefaultTarget<T>`,
// so `T::default()` resolves against the struct's own type parameter. The
// struct's `T: Default` bound is not needed when every path that names `T`
// carries its own bound; here the struct bound covers `T::default()`.
#[derive(Refield, Debug, PartialEq, Eq)]
struct GenericDefaultTarget<T: Default> {
    a: i32,
    #[refield(skip, default = T::default())]
    b: T,
    #[refield(skip, default = Vec::<T>::new())]
    c: Vec<T>,
}

#[allow(dead_code)]
struct GenericDefaultRaw {
    a: i32,
}

#[test]
fn generic_default_expressions_resolve_struct_type_parameters() {
    let target = refield!(GenericDefaultTarget::<String> {
        GenericDefaultRaw { a: 1 }..,
        a: 1,
    });
    assert_eq!(
        target,
        GenericDefaultTarget {
            a: 1,
            b: String::new(),
            c: Vec::new(),
        }
    );

    let target = refield!(GenericDefaultTarget::<u32> {
        GenericDefaultRaw { a: 2 }..,
        a: 2,
        b: 9,
    });
    assert_eq!(
        target,
        GenericDefaultTarget {
            a: 2,
            b: 9,
            c: Vec::new(),
        }
    );
}

// Custom conversions become inherent methods on the target struct, so private
// items stay reachable even when the `refield!` call is written outside the
// module that defines them.
mod private_helpers {
    use refield::Refield;

    fn parse_tag(value: &str) -> String {
        value.trim().to_uppercase()
    }

    fn negate(value: bool) -> bool {
        !value
    }

    fn fallback_port() -> u16 {
        8080
    }

    #[derive(Refield, Debug, PartialEq, Eq)]
    pub struct CrossModConfig {
        #[refield(with = |tag: &str| parse_tag(tag))]
        pub tag: String,
        #[refield(with = negate)]
        pub enabled: bool,
        #[refield(skip, default = fallback_port())]
        pub port: u16,
    }
}

struct CrossModRaw {
    tag: &'static str,
    enabled: bool,
}

#[test]
fn private_helper_functions_resolve_at_the_definition_site() {
    let raw = CrossModRaw {
        tag: "  node ",
        enabled: true,
    };
    let config = refield!(private_helpers::CrossModConfig { ..raw });
    assert_eq!(
        config,
        private_helpers::CrossModConfig {
            tag: "NODE".into(),
            enabled: false,
            port: 8080,
        }
    );
}

// Untyped closures default to the target field type, and closures whose input
// differs from the field type annotate the closure parameter.
#[derive(Refield, Debug, PartialEq, Eq)]
struct ClosureTarget {
    #[refield(with = |x| x * 3)]
    triple: i32,
    #[refield(with = |s: &str| s.len())]
    length: usize,
}

#[allow(dead_code)]
struct ClosureRaw {
    triple: i32,
    length: &'static str,
}

#[test]
fn untyped_closures_use_the_field_or_from_type_as_input() {
    let target = refield!(ClosureTarget {
        ..ClosureRaw {
            triple: 7,
            length: "four",
        }
    });
    assert_eq!(
        target,
        ClosureTarget {
            triple: 21,
            length: 4,
        }
    );
}

// Borrowed outputs require the struct lifetime in the input type; the helper
// method lives inside `impl<'a> BorrowedTarget<'a>`, so `'a` is in scope.
#[derive(Refield, Debug, PartialEq, Eq)]
struct BorrowedTarget<'a> {
    #[refield(with = |s| s.trim())]
    name: &'a str,
    #[refield(with = |b: &'a [u8]| core::str::from_utf8(b).unwrap())]
    body: &'a str,
}

#[allow(dead_code)]
struct BorrowedRaw<'a> {
    name: &'a str,
    body: &'a [u8],
}

#[test]
fn borrowed_conversions_keep_the_struct_lifetime() {
    let raw = BorrowedRaw {
        name: "  gateway ",
        body: b"payload",
    };
    let target = refield!(BorrowedTarget { ..raw });
    assert_eq!(
        target,
        BorrowedTarget {
            name: "gateway",
            body: "payload",
        }
    );
}

// `T` has no bound on the struct itself: the per-field `bound` is attached to
// the generated helper method, so naming `T` in the default expression is
// legal.
#[derive(Refield, Debug, PartialEq, Eq)]
struct BoundedTarget<T> {
    #[refield(bound(T: Default), default = T::default())]
    value: T,
}

#[test]
fn per_field_bounds_are_attached_to_the_helper_method() {
    let target = refield!(BoundedTarget::<u32> {});
    assert_eq!(target, BoundedTarget { value: 0 });
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct StringLiteralDefaultTarget {
    #[refield(default = "localhost")]
    host: &'static str,
}

#[test]
fn string_literal_default_works_as_string_literal_value() {
    let target = refield!(StringLiteralDefaultTarget {});
    assert_eq!(target, StringLiteralDefaultTarget { host: "localhost" });
}

// `const` promotes a helper to `const fn`; a literal `default` is promoted
// automatically, and the flag also works per field instead of struct-wide.
// Both helpers must be callable from a `const` item.
const fn const_double(value: u16) -> u16 {
    value * 2
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct ConstHelperTarget {
    #[refield(const, with = |value| const_double(value))]
    doubled: u16,
    #[refield(skip, const, default = 2 + 3)]
    port: u16,
}

#[allow(dead_code)]
struct ConstHelperRaw {
    doubled: u16,
}

const CONST_HELPER: ConstHelperTarget = refield!(ConstHelperTarget {
    ..ConstHelperRaw { doubled: 21 }
});

#[test]
fn const_helpers_evaluate_in_const_contexts() {
    assert_eq!(
        CONST_HELPER,
        ConstHelperTarget {
            doubled: 42,
            port: 5,
        }
    );
}

// Struct-level `#[refield(const)]` promotes every literal/`const fn` helper.
#[derive(Refield, Debug, PartialEq, Eq)]
#[refield(const)]
struct StructConstTarget {
    #[refield(with = |value| const_double(value))]
    doubled: u16,
    #[refield(skip, default = 7)]
    port: u16,
}

const STRUCT_CONST: StructConstTarget = refield!(StructConstTarget {
    ..ConstHelperRaw { doubled: 4 }
});

#[test]
fn struct_level_const_promotes_every_helper() {
    assert_eq!(
        STRUCT_CONST,
        StructConstTarget {
            doubled: 8,
            port: 7,
        }
    );
}

// Helpers are inherited through type aliases: `Alias::__refield_with_...`
// resolves to the inherent method on the aliased struct.
#[derive(Refield, Debug, PartialEq, Eq)]
struct AliasHelperTarget {
    #[refield(with = |value: &str| value.len())]
    length: usize,
}

#[refieldable]
type AliasHelper = AliasHelperTarget;

#[allow(dead_code)]
struct AliasHelperRaw {
    length: &'static str,
}

#[test]
fn helpers_resolve_through_type_aliases() {
    let target = refield!(AliasHelper {
        ..AliasHelperRaw { length: "alias" }
    });
    assert_eq!(target, AliasHelperTarget { length: 5 });
}

// `const` is also accepted as a `#[refieldable]` macro argument.
#[refieldable(const)]
#[derive(Debug, PartialEq, Eq)]
struct ProcConstTarget {
    #[refield(with = |value| const_double(value))]
    doubled: u16,
    #[refield(skip, default = 1)]
    port: u16,
}

const PROC_CONST: ProcConstTarget = refield!(ProcConstTarget {
    ..ConstHelperRaw { doubled: 3 }
});

#[test]
fn refieldable_const_argument_promotes_helpers() {
    assert_eq!(
        PROC_CONST,
        ProcConstTarget {
            doubled: 6,
            port: 1,
        }
    );
}

// ---------------------------------------------------------------------------
// 17. Deferred context resolution: macro transparency and trait disambiguation
// ---------------------------------------------------------------------------

// `refield!` expands at its own location: the surrounding macro sees the
// original tokens, and the request reaches the `impl` context only once the
// compiler expands it. `stringify!` must therefore observe exactly what the
// user wrote — no injected `as Target` and no context-macro call.
#[derive(Refield, Debug, PartialEq, Eq)]
struct TransparentConfig {
    port: u16,
    #[refield(with = |h: &'static str| h.into())]
    host: String,
}

#[allow(dead_code)]
struct TransparentRaw {
    port: u16,
    host: &'static str,
}

// A user macro that simply forwards its input back out.
macro_rules! identity_call {
    ($e:expr) => {
        $e
    };
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct TransparentServiceConfig {
    port: u16,
    host: String,
}

#[refieldable]
impl TransparentConfig {
    fn build(raw: TransparentRaw) -> (Self, &'static str, &'static str) {
        let target = refield!(Self { ..raw });
        (
            target,
            stringify!(refield!(Self { ..raw })),
            stringify!(identity_call!(refield!(Self { ..raw }))),
        )
    }
}

// `Self::Assoc` resolves through the block's own associated type, even when
// the invocation is wrapped by another macro.
#[refieldable]
impl ServiceTrait for TransparentServiceConfig {
    type Config = TransparentServiceConfig;

    fn build(raw: ServiceRaw) -> Self::Config {
        identity_call!(::refield::refield!(Self::Config { ..raw }))
    }
}

#[test]
fn refield_call_is_transparent_inside_refieldable_impl() {
    let raw = || TransparentRaw {
        port: 8080,
        host: "transparent",
    };
    let (target, rendered, wrapped) = TransparentConfig::build(raw());
    assert_eq!(
        target,
        TransparentConfig {
            port: 8080,
            host: "transparent".into(),
        }
    );
    assert_eq!(rendered, "refield!(Self { ..raw })");
    assert!(!rendered.contains("__refield_ctx__"));
    assert!(!rendered.contains(" as "));
    assert_eq!(wrapped, "identity_call!(refield!(Self { ..raw }))");

    // The wrapped `Self::Config` invocation resolves through this block.
    let config = <TransparentServiceConfig as ServiceTrait>::build(ServiceRaw {
        port: 10,
        host: "forwarded".into(),
    });
    assert_eq!(
        config,
        TransparentServiceConfig {
            port: 10,
            host: "forwarded".into(),
        }
    );
}

// Two traits can define associated types with the same name. Resolution is
// keyed on the actual qualified trait, not just the associated type's name.
trait TraitOne {
    type Output;
    fn build_one(raw: ServiceRaw) -> Self::Output;
}

trait TraitTwo {
    #[allow(dead_code)]
    type Output;
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct OtherServiceConfig {
    port: u16,
    host: String,
}

#[allow(dead_code)]
struct MultiTraitService;

#[refieldable]
impl TraitOne for MultiTraitService {
    type Output = ServiceConfig;

    fn build_one(raw: ServiceRaw) -> Self::Output {
        // `TraitTwo::Output` has a different schema; it is named explicitly.
        let _other = refield!(<Self as TraitTwo>::Output as OtherServiceConfig {
            port: raw.port,
            host: raw.host.clone(),
        });
        // Without a schema the associated type is resolved through this
        // block's `impl TraitOne`, not by the associated type's name alone.
        refield!(<Self as TraitOne>::Output {
            port: raw.port + 1,
            ..raw
        })
    }
}

#[refieldable]
impl TraitTwo for MultiTraitService {
    type Output = OtherServiceConfig;
}

#[test]
fn qualified_associated_types_are_disambiguated_by_trait() {
    // Both traits really do have an `Output` with a different schema.
    let other: <MultiTraitService as TraitTwo>::Output = refield!(<MultiTraitService as TraitTwo>::Output as OtherServiceConfig {
        port: 7,
        host: "other".into(),
    });
    assert_eq!(
        other,
        OtherServiceConfig {
            port: 7,
            host: "other".into(),
        }
    );

    let config = MultiTraitService::build_one(ServiceRaw {
        port: 100,
        host: "multi".into(),
    });
    assert_eq!(
        config,
        ServiceConfig {
            port: 101,
            host: "multi".into(),
        }
    );
}

// `#[cfg_attr]` on an associated type is expanded by `rustc` before
// `Prepare` inspects the block, so the attribute never reaches the
// `#[refieldable]` expansion.
trait AttrAssoc {
    type Config;
    fn build(raw: ServiceRaw) -> Self::Config;
}

struct AttrService;

#[refieldable]
impl AttrAssoc for AttrService {
    #[cfg_attr(all(), allow(dead_code))]
    type Config = ServiceConfig;

    fn build(raw: ServiceRaw) -> Self::Config {
        refield!(Self::Config { ..raw })
    }
}

#[test]
fn cfg_attr_on_associated_type_survives_prepare() {
    let config = AttrService::build(ServiceRaw {
        port: 12,
        host: "attr".into(),
    });
    assert_eq!(
        config,
        ServiceConfig {
            port: 12,
            host: "attr".into(),
        }
    );
}

// ---------------------------------------------------------------------------
// 18. Renamed trait imports in `<Self as Trait>::Assoc`
// ---------------------------------------------------------------------------

// The context macro keys associated types by name only; the schema identity
// check on the requested type is what proves that `<Self as Trait>::Assoc` is
// the type this block defines. A renamed import therefore resolves the same
// way as the spelling written in the `impl` header.
use ServiceTrait as AliasedTrait;

struct RenamedTraitService;

#[refieldable]
impl ServiceTrait for RenamedTraitService {
    type Config = ServiceConfig;

    fn build(raw: ServiceRaw) -> Self::Config {
        // The header above names `ServiceTrait`; the body reaches the same
        // trait through the renamed import. Matching on the trait's spelling
        // would miss this, while the requested type is identical either way.
        let base = refield!(<Self as AliasedTrait>::Config {
            port: raw.port + 30,
            ..raw
        });
        refield!(<Self as AliasedTrait>::Config { ..base })
    }
}

#[test]
fn renamed_trait_alias_resolves_associated_type() {
    let config = RenamedTraitService::build(ServiceRaw {
        port: 20,
        host: "renamed".into(),
    });
    assert_eq!(
        config,
        ServiceConfig {
            port: 50,
            host: "renamed".into(),
        }
    );
}

// ---------------------------------------------------------------------------
// 19. Custom `Deref` wrappers: `..&wrapper as &Raw`
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
struct DerefRaw {
    a: i32,
    b: i32,
    unused: String,
}

// `Deref` alternates between two distinct `DerefRaw` values: reading one
// field per deref would produce the mixed `(1, 20)`.
struct InnerWrapper {
    first_raw: DerefRaw,
    second_raw: DerefRaw,
    inner_calls: Cell<usize>,
}

impl Deref for InnerWrapper {
    type Target = DerefRaw;

    fn deref(&self) -> &Self::Target {
        let call = self.inner_calls.get() + 1;
        self.inner_calls.set(call);
        if call == 1 {
            &self.first_raw
        } else {
            &self.second_raw
        }
    }
}

struct OuterWrapper {
    inner: InnerWrapper,
    outer_calls: Cell<usize>,
}

impl Deref for OuterWrapper {
    type Target = InnerWrapper;

    fn deref(&self) -> &Self::Target {
        self.outer_calls.set(self.outer_calls.get() + 1);
        &self.inner
    }
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct DerefTarget {
    a: i32,
    b: i32,
}

fn make_inner_wrapper() -> InnerWrapper {
    InnerWrapper {
        first_raw: DerefRaw {
            a: 1,
            b: 10,
            unused: "first".into(),
        },
        second_raw: DerefRaw {
            a: 2,
            b: 20,
            unused: "second".into(),
        },
        inner_calls: Cell::new(0),
    }
}

#[test]
fn custom_deref_wrapper_reads_every_field_from_one_deref() {
    let outer = OuterWrapper {
        inner: make_inner_wrapper(),
        outer_calls: Cell::new(0),
    };

    // `as &DerefRaw` pins the final reference, so `Deref` runs once per layer
    // instead of once per field. Without it the expansion would read `a` from
    // the first raw and `b` from the second.
    let target = refield!(DerefTarget {
        ..&outer as &DerefRaw
    });
    assert_eq!(target, DerefTarget { a: 1, b: 10 });
    assert_eq!(outer.outer_calls.get(), 1);
    assert_eq!(outer.inner.inner_calls.get(), 1);

    // The wrapper is still alive and owns its non-`Copy` field.
    assert_eq!(outer.inner.first_raw.unused, "first");
    assert_eq!(outer.inner.second_raw.unused, "second");

    // A place wrapper without the final cast is expanded per field, so the
    // alternating `Deref` is observable there.
    let outer = OuterWrapper {
        inner: make_inner_wrapper(),
        outer_calls: Cell::new(0),
    };
    let mixed = refield!(DerefTarget { ..*outer });
    assert_eq!(mixed, DerefTarget { a: 1, b: 20 });
}

// ---------------------------------------------------------------------------
// 20. Temporary wrapper from a factory: `..&make_wrapper() as &Raw`
// ---------------------------------------------------------------------------

struct DropLogger {
    label: &'static str,
    log: &'static EventLog,
}

impl Drop for DropLogger {
    fn drop(&mut self) {
        record(self.log, self.label);
    }
}

// The wrapper records its own dereferences so the event sequence proves that
// both `Deref` layers run once, inside `refield!`, before any conversion.
struct DerefLogWrapper {
    raw: DerefRaw,
}

impl Deref for DerefLogWrapper {
    type Target = DerefRaw;

    fn deref(&self) -> &Self::Target {
        record(&DEREF_LOG, "deref inner");
        &self.raw
    }
}

struct OuterLogWrapper {
    inner: DerefLogWrapper,
    _guard: DropLogger,
}

impl Deref for OuterLogWrapper {
    type Target = DerefLogWrapper;

    fn deref(&self) -> &Self::Target {
        record(&DEREF_LOG, "deref outer");
        &self.inner
    }
}

// `with` expressions are compiled into inherent helper methods on the target
// struct, so they run in the definition-site scope and cannot capture a test
// local. Module-level logs carry the event sequence instead.
static DEREF_LOG: EventLog = EventLog::new(Vec::new());
static PANIC_LOG: EventLog = EventLog::new(Vec::new());

fn recorded_events(log: &EventLog) -> Vec<&'static str> {
    log.lock().unwrap().clone()
}

fn make_outer_wrapper() -> OuterLogWrapper {
    record(&DEREF_LOG, "make_wrapper");
    OuterLogWrapper {
        inner: DerefLogWrapper {
            raw: DerefRaw {
                a: 0,
                b: 20,
                unused: "factory".into(),
            },
        },
        _guard: DropLogger {
            label: "drop wrapper",
            log: &DEREF_LOG,
        },
    }
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct FactoryTarget {
    a: i32,
    #[refield(with = |value| {
        record(&DEREF_LOG, "convert b");
        value + 1
    })]
    b: i32,
}

#[test]
fn temporary_wrapper_from_factory_is_pinned_for_the_whole_call() {
    DEREF_LOG.lock().unwrap().clear();
    let target = refield!(FactoryTarget {
        // The cast pins the final `&DerefRaw` and Rust extends the temporary's
        // lifetime to the end of the expansion, so the wrapper stays alive
        // while the automatic fields are converted and the explicit fields run.
        &make_outer_wrapper() as &DerefRaw..,
        a: {
            record(&DEREF_LOG, "explicit a");
            99
        },
    });
    record(&DEREF_LOG, "after refield");

    assert_eq!(target, FactoryTarget { a: 99, b: 21 });
    assert_eq!(
        recorded_events(&DEREF_LOG),
        [
            "make_wrapper",
            "deref outer",
            "deref inner",
            "convert b",
            "explicit a",
            "drop wrapper",
            "after refield",
        ]
    );
}

// ---------------------------------------------------------------------------
// 21. Drop order when a conversion panics
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct PanicRaw {
    a: DropLogger,
    spare: DropLogger,
    b: DropLogger,
}

fn panic_conversion(value: DropLogger) -> DropLogger {
    // Dropping `value` happens while this call unwinds, before the caller's
    // `raw` starts dropping its remaining fields.
    panic!("convert a failed: {}", value.label);
}

#[allow(dead_code)]
#[derive(Refield)]
struct PanicTarget {
    #[refield(with = panic_conversion)]
    a: DropLogger,
    b: DropLogger,
}

// Taking `raw` by value keeps the whole struct in this frame, so unwinding
// drops its remaining fields here. Capturing it directly in the closure would
// let edition-2024 disjoint capture move `a` and `b` separately and leave
// `spare` to the caller.
fn build_panicking(raw: PanicRaw) {
    let _ = refield!(PanicTarget { ..raw });
}

#[test]
fn conversion_panic_keeps_remaining_fields_in_declaration_order() {
    PANIC_LOG.lock().unwrap().clear();
    let raw = PanicRaw {
        a: DropLogger {
            label: "a",
            log: &PANIC_LOG,
        },
        spare: DropLogger {
            label: "spare",
            log: &PANIC_LOG,
        },
        b: DropLogger {
            label: "b",
            log: &PANIC_LOG,
        },
    };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build_panicking(raw)));
    assert!(result.is_err());

    // `a` is moved into the panicking conversion, which drops its parameter.
    // The fields still inside `raw` were never moved out, so they drop in
    // `PanicRaw`'s declaration order rather than in target order.
    assert_eq!(recorded_events(&PANIC_LOG), ["a", "spare", "b"]);
}

// ---------------------------------------------------------------------------
// 22. Cross-crate definition-site helpers and generic inference (§0.9 & §2.1)
// ---------------------------------------------------------------------------

// `normalize_host` and `default_retries` are private to `refield-test-models`;
// the derived helper methods must still resolve them from this crate.
struct CrossCrateRaw<T> {
    host: &'static str,
    retries: u8,
    payload: T,
}

#[test]
fn cross_crate_private_helpers_and_generic_defaults_resolve_at_definition_site() {
    // When fields are omitted without rest, defaults evaluate at the definition site:
    // Turbofish spelling.
    let model = refield!(models::CrossCrateModel::<Vec<u8>> {
        host: "cache.example".into(),
    });
    assert_eq!(model.host, "cache.example");
    assert_eq!(
        model.retries, 3,
        "private `default_retries` ran in its crate"
    );
    assert_eq!(model.payload, Vec::<u8>::new());

    // No turbofish: the shared `PhantomData<Self>` passed to every helper and
    // to the schema identity check infers `T` from the annotation, even though
    // the helpers themselves live in another crate.
    let raw = CrossCrateRaw {
        host: "  NoTurbofish.Example ",
        retries: 9,
        payload: vec![1_u8, 2],
    };
    let model: models::CrossCrateModel<Vec<u8>> = refield!(models::CrossCrateModel { ..raw });
    assert_eq!(model.host, "noturbofish.example");
    assert_eq!(model.retries, 9);
    assert_eq!(model.payload, vec![1, 2]);

    // The companion macro spelling reaches the same definition-site helpers.
    use models::CrossCrateModel;
    let model: CrossCrateModel<Vec<u8>> = CrossCrateModel! {
        host: "companion.example".into(),
        payload: Vec::<u8>::new(),
    };
    assert_eq!(model.host, "companion.example");
    assert_eq!(model.retries, 3);
}

#[test]
fn cross_crate_tuple_struct_alias_and_local_counterpart_preserve_shape() {
    #[allow(dead_code)]
    struct TupleRaw(i32, &'static str);

    // Cross-crate alias to a tuple struct: the alias macro forwards the
    // target's tuple shape and field rules.
    let model = refield!(models::TupleModelAlias {
        TupleRaw(1, "cross-crate")..,
        0: 5,
    });
    assert_eq!(model, models::nested::TupleModel(5, "cross-crate".into()));

    use models::TupleModelAlias;
    let model = TupleModelAlias! {
        TupleRaw(1, "companion")..,
        0: 6,
    };
    assert_eq!(model, models::nested::TupleModel(6, "companion".into()));

    // Local tuple alias.
    let model = refield!(LocalTupleAlias {
        TupleRaw(1, "local")..,
        0: 7,
    });
    assert_eq!(model, LocalTupleAliasTarget(7, "local".into()));

    let model = LocalTupleAlias! {
        TupleRaw(1, "local-companion")..,
        0: 8,
    };
    assert_eq!(model, LocalTupleAliasTarget(8, "local-companion".into()));
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct LocalTupleAliasTarget(i32, #[refield(with = |s: &'static str| s.into())] String);

#[refieldable]
type LocalTupleAlias = LocalTupleAliasTarget;

// ---------------------------------------------------------------------------
// 23. `default` / `with` evaluation order and omitted fields
// ---------------------------------------------------------------------------

static LAZY_LOG: EventLog = EventLog::new(Vec::new());

#[derive(Refield, Debug, PartialEq, Eq)]
struct LazyTarget {
    #[refield(
        with = |v: u32| { record(&LAZY_LOG, "with called"); v + 1 },
        default = { record(&LAZY_LOG, "default called"); 30 },
    )]
    value: u32,
}

struct LazyRaw {
    value: u32,
}

#[test]
fn default_and_with_rules_evaluate_as_configured() {
    // With `..rest`: only the conversion runs.
    LAZY_LOG.lock().unwrap().clear();
    let target = refield!(LazyTarget {
        ..LazyRaw { value: 5 }
    });
    assert_eq!(target, LazyTarget { value: 6 });
    assert_eq!(*LAZY_LOG.lock().unwrap(), ["with called"]);

    // Without `..rest`: only the default runs.
    LAZY_LOG.lock().unwrap().clear();
    let target = LazyTarget! {};
    assert_eq!(target, LazyTarget { value: 30 });
    assert_eq!(*LAZY_LOG.lock().unwrap(), ["default called"]);

    // Explicit field: neither rule runs.
    LAZY_LOG.lock().unwrap().clear();
    let target = refield!(LazyTarget {
        LazyRaw { value: 5 }..,
        value: 99,
    });
    assert_eq!(target, LazyTarget { value: 99 });
    let events = LAZY_LOG.lock().unwrap().clone();
    assert!(
        events.is_empty(),
        "explicit fields bypass `with` and `default`, got {events:?}"
    );
}

// ---------------------------------------------------------------------------
// 24. `private_interfaces` / `private_bounds` under `deny` (Appendix #4)
// ---------------------------------------------------------------------------

#[deny(private_interfaces, private_bounds)]
mod private_interfaces {
    // Reachable only inside this module; the `pub(crate)` field and the
    // generated helpers that mention it are what the `deny` would flag.
    #[derive(Debug, Default, PartialEq, Eq)]
    struct PrivateFieldType(pub u8);

    // The struct-level `allow` covers the field the user declared; the helper
    // methods carry their own `#[allow(private_interfaces, private_bounds)]`
    // (asserted by `generated_helpers_allow_private_interface_lints`).
    #[allow(private_interfaces, private_bounds)]
    #[derive(refield::Refield, Debug, PartialEq, Eq)]
    pub struct PubStructWithPrivateField {
        #[refield(with = |v: u8| PrivateFieldType(v + 1), default = PrivateFieldType(7))]
        pub(crate) secret: PrivateFieldType,
    }

    pub fn build(secret: u8) -> PubStructWithPrivateField {
        struct Raw {
            secret: u8,
        }
        refield::refield!(PubStructWithPrivateField { ..Raw { secret } })
    }

    pub fn build_default() -> PubStructWithPrivateField {
        refield::refield!(PubStructWithPrivateField {})
    }

    pub fn value(target: &PubStructWithPrivateField) -> u8 {
        target.secret.0
    }

    pub fn build_explicit() -> PubStructWithPrivateField {
        // The caller-provided value bypasses both helpers; the field type is
        // private, so this has to happen inside the module too.
        refield::refield!(PubStructWithPrivateField {
            secret: PrivateFieldType::default(),
        })
    }
}

#[test]
fn public_struct_with_private_field_types_stays_lint_clean() {
    assert_eq!(private_interfaces::value(&private_interfaces::build(1)), 2);
    assert_eq!(
        private_interfaces::value(&private_interfaces::build_default()),
        7
    );

    // Explicit fields still bypass both helpers.
    assert_eq!(
        private_interfaces::value(&private_interfaces::build_explicit()),
        0
    );
}

// ---------------------------------------------------------------------------
// 25. `#[refieldable] impl`: `cfg_attr(..., cfg(...))`, renamed imports,
//     const contexts, closures, and nested `impl` blocks
// ---------------------------------------------------------------------------

// An active `cfg_attr` that expands to `#[cfg(any())]` is stripped by rustc
// before `Prepare` sees the block, so the deliberately nonexistent type never
// reaches schema resolution.
trait CfgAttrAssoc {
    type Config;
    fn build(raw: ServiceRaw) -> Self::Config;
}

struct CfgAttrService;

#[refieldable]
impl CfgAttrAssoc for CfgAttrService {
    #[cfg_attr(all(), cfg(any()))]
    type Config = NonExistentType;
    #[cfg_attr(all(), allow(dead_code))]
    type Config = ServiceConfig;

    fn build(raw: ServiceRaw) -> Self::Config {
        refield!(Self::Config {
            port: raw.port + 1,
            ..raw
        })
    }
}

#[test]
fn cfg_attr_cfg_any_associated_type_is_stripped_before_prepare() {
    let config = CfgAttrService::build(ServiceRaw {
        port: 30,
        host: "cfg-attr-cfg".into(),
    });
    assert_eq!(
        config,
        ServiceConfig {
            port: 31,
            host: "cfg-attr-cfg".into(),
        }
    );
}

// Renamed and absolute `refield!` spellings inside an annotated `impl` both go
// through the real proc-macro entry and then the same lexical context.
struct ImportModeRaw {
    a: i32,
    b: i32,
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct ImportModeTarget {
    a: i32,
    b: i32,
}

#[refieldable]
impl ImportModeTarget {
    fn build(raw: ImportModeRaw) -> Self {
        use refield::refield as my_refield;

        let renamed = my_refield!(Self { ..raw });
        let absolute = ::refield::refield!(Self {
            a: renamed.a + 1,
            b: renamed.b,
        });
        refield!(Self {
            a: absolute.a + 1,
            ..absolute
        })
    }
}

#[test]
fn renamed_and_absolute_macro_paths_resolve_inside_annotated_impl() {
    let target = ImportModeTarget::build(ImportModeRaw { a: 1, b: 9 });
    assert_eq!(target, ImportModeTarget { a: 3, b: 9 });
}

// `#[refield(const)]` promotes the helpers, so the whole construction can run
// in an associated constant and an inline `const` block. A non-capturing
// closure coerced to `fn(Raw) -> Self` proves the expansion really is a plain
// expression with no captured state.
#[derive(Refield, Debug, PartialEq, Eq)]
#[refield(const)]
struct ConstImplTarget {
    a: i32,
    b: i32,
}

struct ConstImplRaw {
    a: i32,
    b: i32,
}

#[refieldable]
impl ConstImplTarget {
    const DEFAULT: Self = refield!(Self {
        ..ConstImplRaw { a: 1, b: 2 }
    });

    fn from_inline_const() -> Self {
        const {
            refield!(Self {
                ..ConstImplRaw { a: 3, b: 4 }
            })
        }
    }

    fn from_fn_pointer(raw: ConstImplRaw) -> Self {
        let build: fn(ConstImplRaw) -> Self = |raw| refield!(Self { ..raw });
        build(raw)
    }
}

#[test]
fn associated_const_inline_const_and_fn_pointer_work_inside_annotated_impl() {
    assert_eq!(ConstImplTarget::DEFAULT, ConstImplTarget { a: 1, b: 2 });
    assert_eq!(
        ConstImplTarget::from_inline_const(),
        ConstImplTarget { a: 3, b: 4 }
    );
    assert_eq!(
        ConstImplTarget::from_fn_pointer(ConstImplRaw { a: 5, b: 6 }),
        ConstImplTarget { a: 5, b: 6 }
    );
}

// Nested `impl` blocks: a lexically nearer annotated block provides its own
// context, while an unannotated block that happens to use `Self = Outer`
// inherits the enclosing candidate and still passes the schema identity check.
#[derive(Refield, Debug, PartialEq, Eq)]
struct NestedOuterTarget {
    a: i32,
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct NestedInnerTarget {
    b: i32,
}

struct NestedOuterRaw {
    a: i32,
}

struct NestedInnerRaw {
    b: i32,
}

#[refieldable]
impl NestedOuterTarget {
    fn build(raw: NestedOuterRaw) -> (Self, NestedInnerTarget) {
        // Case 1: the inner annotated block overrides the outer context.
        #[allow(non_local_definitions)]
        #[refieldable]
        impl NestedInnerTarget {
            fn make(raw: NestedInnerRaw) -> Self {
                refield!(Self { ..raw })
            }
        }

        // Case 2: an unannotated `impl` whose `Self` is the same type as the
        // enclosing candidate; `Self` resolves through the inherited schema.
        #[allow(non_local_definitions)]
        impl NestedOuterTarget {
            fn build_inherited(raw: NestedOuterRaw) -> Self {
                refield!(Self { ..raw })
            }
        }

        let inner = NestedInnerTarget::make(NestedInnerRaw { b: 7 });
        (NestedOuterTarget::build_inherited(raw), inner)
    }
}

trait NestedHelperTrait {
    fn build_outer(raw: NestedOuterRaw) -> Self;
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct NestedTraitOuter {
    a: i32,
}

#[refieldable]
impl NestedTraitOuter {
    fn build(raw: NestedOuterRaw) -> Self {
        // An unannotated trait impl for the same `Self` type inherits the
        // outer candidate; the requested type owns the schema identity method.
        #[allow(non_local_definitions)]
        impl NestedHelperTrait for NestedTraitOuter {
            fn build_outer(raw: NestedOuterRaw) -> Self {
                refield!(Self { ..raw })
            }
        }
        NestedTraitOuter::build_outer(raw)
    }
}

#[test]
fn nested_impl_blocks_override_or_inherit_the_enclosing_context() {
    let (outer, inner) = NestedOuterTarget::build(NestedOuterRaw { a: 1 });
    assert_eq!(outer, NestedOuterTarget { a: 1 });
    assert_eq!(inner, NestedInnerTarget { b: 7 });

    assert_eq!(
        NestedTraitOuter::build(NestedOuterRaw { a: 2 }),
        NestedTraitOuter { a: 2 }
    );
}

// ---------------------------------------------------------------------------
// 26. Generic specialization through a renamed alias without the target
// ---------------------------------------------------------------------------

mod specialized_alias {
    use refield::{Refield, refieldable};

    #[derive(Refield, Debug, Clone, Copy, PartialEq, Eq)]
    pub struct GenTarget<T> {
        pub v: T,
    }

    #[refieldable]
    pub type U32Alias = GenTarget<u32>;
}

#[test]
fn renamed_alias_preserves_generic_specialization_without_target_import() {
    // `GenTarget` is deliberately NOT imported: `RenamedU32Alias` alone has to
    // supply the `u32` specialization, which also types the literal `7`.
    use specialized_alias::U32Alias as RenamedU32Alias;

    #[allow(dead_code)]
    struct Raw {
        v: u32,
    }
    let raw = Raw { v: 3 };
    let target = refield!(RenamedU32Alias { v: 7, ..raw });
    let value: u32 = target.v;
    assert_eq!(value, 7);
    assert_eq!(target, specialized_alias::GenTarget::<u32> { v: 7 });

    // The direct companion spelling is specialized the same way. Importing
    // the alias under its own name also imports its companion macro.
    use specialized_alias::U32Alias;
    let target = U32Alias! { v: 9 };
    let value: u32 = target.v;
    assert_eq!(value, 9);
}

// ---------------------------------------------------------------------------
// 27. `#[refieldable] type` aliases: conditional attributes and visibility
// ---------------------------------------------------------------------------

trait AliasAttrAssoc {
    type Config;
}

struct AliasAttrService;

impl AliasAttrAssoc for AliasAttrService {
    type Config = ServiceConfig;
}

// `cfg_attr` can conditionally apply `#[refieldable(as = ServiceConfig)]`.
#[cfg_attr(all(), refieldable(as = ServiceConfig))]
pub(crate) type CfgAttrAlias = <AliasAttrService as AliasAttrAssoc>::Config;

// Restricted visibility is inherited by the alias companion macro.
#[refieldable]
pub(crate) type RestrictedVisibilityAlias = ServiceConfig;

#[test]
fn conditional_alias_attributes_and_restricted_visibility_work() {
    let config = refield!(CfgAttrAlias {
        ..ServiceRaw {
            port: 2,
            host: "cfg".into(),
        }
    });
    assert_eq!(
        config,
        ServiceConfig {
            port: 2,
            host: "cfg".into(),
        }
    );

    let config = CfgAttrAlias! {
        ServiceRaw { port: 0, host: "companion".into() }..,
        port: 3,
    };
    assert_eq!(config.port, 3);
    assert_eq!(config.host, "companion");

    let config = refield!(RestrictedVisibilityAlias {
        ..ServiceRaw {
            port: 4,
            host: "restricted".into(),
        }
    });
    assert_eq!(
        config,
        ServiceConfig {
            port: 4,
            host: "restricted".into(),
        }
    );
}

// ---------------------------------------------------------------------------
// 28. Shared-reference sources, cast sources, and place drop timing
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct SharedRefRaw {
    a: i32,
    b: i32,
    c: i32,
    owned: String,
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct SharedRefTarget {
    a: i32,
    b: i32,
    c: i32,
}

#[test]
fn shared_reference_source_copies_copy_fields_by_dot_projection() {
    let raw = SharedRefRaw {
        a: 1,
        b: 2,
        c: 3,
        owned: "kept".into(),
    };
    let reference: &SharedRefRaw = &raw;

    // `(reference).a` etc. are places through a shared reference: `i32` is
    // copied, and neither `reference` nor `raw.owned` is moved.
    let target = refield!(SharedRefTarget { ..reference });
    assert_eq!(target, SharedRefTarget { a: 1, b: 2, c: 3 });
    assert_eq!(raw.owned, "kept");
    assert_eq!(reference.a, 1);

    // The same holds when the reference is itself a value expression.
    let target = refield!(SharedRefTarget {
        ..&raw as &SharedRefRaw
    });
    assert_eq!(target, SharedRefTarget { a: 1, b: 2, c: 3 });
    assert_eq!(raw.owned, "kept");
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct CastRestTarget;

#[test]
fn cast_expression_in_rest_position_is_a_value_source() {
    // `..1u8 as u16` is a legal Rust cast; the macro must treat it as a value
    // expression rather than interpreting `as` as a schema declaration.
    let target = refield!(CastRestTarget { ..1u8 as u16 });
    assert_eq!(target, CastRestTarget);
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct SingleFieldTarget {
    a: i32,
}

#[allow(dead_code)]
struct PlaceDropRaw {
    a: i32,
    spare: DropLogger,
}

#[test]
fn local_place_source_keeps_remaining_fields_until_the_local_drops() {
    let log = EventLog::new(Vec::new());
    // Leak the log so the guard can borrow it as `'static`, matching the
    // `DropLogger` definition used by the other ordering tests.
    let log: &'static EventLog = Box::leak(Box::new(log));

    let raw = PlaceDropRaw {
        a: 1,
        spare: DropLogger {
            label: "drop spare",
            log,
        },
    };

    // `..raw` is a place expression: the macro reads `raw.a` directly instead
    // of moving `raw` into an internal temporary, so `spare` stays in `raw`
    // and drops when the local goes out of scope — after `"after macro"`.
    let target = refield!(SingleFieldTarget { ..raw });
    record(log, "after macro");
    drop(raw);

    assert_eq!(target, SingleFieldTarget { a: 1 });
    assert_eq!(*log.lock().unwrap(), ["after macro", "drop spare"]);
}

// ---------------------------------------------------------------------------
// 29. Call-site local variable hygiene against macro temporaries (§0.11)
// ---------------------------------------------------------------------------

#[derive(Refield, Debug, PartialEq, Eq)]
struct HygieneCollisionTarget {
    a: i32,
    b: i32,
}

#[allow(dead_code)]
struct HygieneCollisionRaw {
    a: i32,
    b: i32,
}

#[test]
fn call_site_locals_named_like_macro_temporaries_do_not_collide() {
    // Every one of these names is also used by the expansion for a
    // `Span::mixed_site()` binding. The call site must keep referring to the
    // caller's bindings in both field expressions and the source.
    let __refield_source = 10_i32;
    let __refield_value = 20_i32;
    let __refield_base = 30_i32;
    let __refield_marker = 40_i32;

    // Place source: the caller's bindings resolve inside the explicit fields.
    let raw = HygieneCollisionRaw { a: 1, b: 2 };
    let target = refield!(HygieneCollisionTarget {
        a: __refield_source + __refield_marker,
        b: __refield_value + __refield_base,
        ..raw
    });
    assert_eq!(target, HygieneCollisionTarget { a: 50, b: 50 });

    // Value source: the macro binds `__refield_source` itself, so the caller's
    // binding of the same name must stay untouched inside the explicit fields
    // and usable afterwards.
    let target = refield!(HygieneCollisionTarget {
        HygieneCollisionRaw { a: 7, b: 8 }..,
        a: __refield_source,
        b: __refield_marker + __refield_value + __refield_base,
    });
    assert_eq!(target, HygieneCollisionTarget { a: 10, b: 90 });
    assert_eq!(__refield_source, 10);

    // Place source: `..__refield_source` and `__refield_source..` must resolve
    // to the caller's binding even though the macro also names its value
    // binding `__refield_source`.
    let __refield_source = HygieneCollisionRaw { a: 3, b: 4 };
    let target = refield!(HygieneCollisionTarget { ..__refield_source });
    assert_eq!(target, HygieneCollisionTarget { a: 3, b: 4 });
    let target = refield!(HygieneCollisionTarget { __refield_source.., b: 9 });
    assert_eq!(target, HygieneCollisionTarget { a: 3, b: 9 });
    assert_eq!(__refield_source.b, 4);

    // The companion macro spelling is hygienic in the same way.
    let raw = HygieneCollisionRaw { a: 5, b: 6 };
    let target = HygieneCollisionTarget! {
        a: __refield_value + __refield_base + __refield_marker,
        ..raw
    };
    assert_eq!(target, HygieneCollisionTarget { a: 90, b: 6 });
}

// ---------------------------------------------------------------------------
// 30. Struct-level `name`, `with`, and `from = Type`
// ---------------------------------------------------------------------------

struct StructNameRaw {
    raw_port: u32,
    raw_name: String,
}

// One struct-level pattern replaces one attribute per field. `{}` expands to
// each target field's name.
#[derive(Refield, Debug, PartialEq, Eq)]
#[refield(name = "raw_{}")]
struct StructNameTarget {
    port: u32,
    name: String,
}

#[test]
fn struct_level_name_expands_per_field() {
    let raw = StructNameRaw {
        raw_port: 8080,
        raw_name: "node".into(),
    };
    let target = refield!(StructNameTarget { ..raw });
    assert_eq!(
        target,
        StructNameTarget {
            port: 8080,
            name: "node".into(),
        }
    );

    // The companion macro spelling gets the same schema.
    let raw = StructNameRaw {
        raw_port: 1,
        raw_name: "second".into(),
    };
    let target = StructNameTarget! { ..raw };
    assert_eq!(target.port, 1);
    assert_eq!(target.name, "second");
}

// Tuple structs get the same struct-level treatment; `{}` expands to the
// tuple index, so `raw_{}` names `raw_0`, `raw_1`, ...
#[derive(Refield, Debug, PartialEq, Eq)]
#[refield(name = "raw_{}")]
struct StructNameTuple(u32, String);

#[test]
fn struct_level_name_maps_tuple_fields_to_named_sources() {
    #[allow(dead_code)]
    struct Raw {
        raw_0: u32,
        raw_1: String,
    }
    let target = refield!(StructNameTuple {
        ..Raw {
            raw_0: 3,
            raw_1: "tuple".into(),
        }
    });
    assert_eq!(target, StructNameTuple(3, "tuple".into()));
}

#[test]
fn struct_level_name_does_not_apply_to_skipped_fields() {
    #[derive(Refield, Debug, PartialEq, Eq)]
    #[refield(name = "raw_{}")]
    struct Target {
        port: u32,
        #[refield(skip, default = String::from("skipped"))]
        name: String,
    }

    #[allow(dead_code)]
    struct Raw {
        raw_port: u32,
        raw_name: &'static str,
    }

    let target = refield!(Target {
        ..Raw {
            raw_port: 7,
            raw_name: "unused"
        }
    });
    assert_eq!(target.port, 7);
    assert_eq!(target.name, "skipped");
}

fn struct_level_negate(value: bool) -> bool {
    !value
}

#[derive(Refield, Debug, PartialEq, Eq)]
#[refield(with = struct_level_negate)]
struct StructWithTarget {
    enabled: bool,
    // A field-level `with` replaces the struct-level one.
    #[refield(with = |value: bool| value)]
    passthrough: bool,
}

#[test]
fn struct_level_with_applies_to_every_automatic_field() {
    #[allow(dead_code)]
    struct Raw {
        enabled: bool,
        passthrough: bool,
    }
    let target = refield!(StructWithTarget {
        ..Raw {
            enabled: true,
            passthrough: true,
        }
    });
    assert_eq!(
        target,
        StructWithTarget {
            enabled: false,
            passthrough: true,
        }
    );
}

// An untyped closure under a struct-level `with` sees each field's own type,
// while an explicitly typed closure parameter names a shared input type.
#[derive(Refield, Debug, PartialEq, Eq)]
#[refield(with = |value: u16| value as u32 * 2)]
struct StructFromWithTarget {
    short: u32,
    other: u32,
}

#[test]
fn struct_level_from_type_pairs_with_struct_level_with() {
    #[allow(dead_code)]
    struct Raw {
        short: u16,
        other: u16,
    }
    let target = refield!(StructFromWithTarget {
        ..Raw { short: 4, other: 5 }
    });
    assert_eq!(
        target,
        StructFromWithTarget {
            short: 8,
            other: 10,
        }
    );
}

// Struct-level `name`, `with`, and `const` combine into one `const fn` helper
// per field, which must stay usable from a `const` context.
const fn const_struct_double(value: u16) -> u32 {
    value as u32 * 2
}

#[derive(Refield, Debug, PartialEq, Eq)]
#[refield(
    name = "raw_{}",
    with = |value: u16| const_struct_double(value),
    const
)]
struct StructCombinedTarget {
    port: u32,
    retries: u32,
}

const STRUCT_COMBINED: StructCombinedTarget = refield!(StructCombinedTarget {
    ..StructCombinedRaw {
        raw_port: 21,
        raw_retries: 2,
    }
});

#[allow(dead_code)]
struct StructCombinedRaw {
    raw_port: u16,
    raw_retries: u16,
}

#[test]
fn struct_level_name_with_and_const_combine() {
    assert_eq!(
        STRUCT_COMBINED,
        StructCombinedTarget {
            port: 42,
            retries: 4,
        }
    );
}

// Field-level `name`, `with`, `from`, and `skip` each override the
// corresponding struct-level default.
#[derive(Refield, Debug, PartialEq, Eq)]
#[refield(name = "raw_{}", with = struct_level_negate)]
struct OverrideTarget {
    // `name` override: read from `direct` instead of `raw_first`.
    #[refield(name = "direct")]
    first: bool,
    // `with` override: a typed closure wins over the struct-level conversion.
    #[refield(with = |value: bool| value)]
    second: bool,
    // The struct-level `with` applies here because the field names no rule.
    third: bool,
    // `skip` override: never read the source at all.
    #[refield(skip, default = false)]
    fourth: bool,
}

// A field-level `with` replaces the struct-level `with` instead of being
// combined with it.
#[derive(Refield, Debug, PartialEq, Eq)]
#[refield(with = struct_level_negate)]
struct FromOverrideTarget {
    #[refield(with = |value: bool| value)]
    passthrough: bool,
}

#[test]
fn field_level_attributes_override_struct_level_defaults() {
    #[allow(dead_code)]
    struct Raw {
        direct: bool,
        raw_second: bool,
        raw_third: bool,
        raw_fourth: bool,
        passthrough: bool,
    }

    let target = refield!(OverrideTarget {
        ..Raw {
            direct: true,
            raw_second: false,
            raw_third: true,
            raw_fourth: true,
            passthrough: true,
        }
    });
    assert_eq!(
        target,
        OverrideTarget {
            // `name` only changes the source; the struct-level `with` still
            // negates the value read from `direct`.
            first: false,
            second: false,
            third: false,
            fourth: false,
        }
    );

    // The field-level `with` replaces the struct-level negation.
    let target = refield!(FromOverrideTarget {
        ..Raw {
            direct: true,
            raw_second: true,
            raw_third: true,
            raw_fourth: true,
            passthrough: true,
        }
    });
    assert!(target.passthrough);
}

// Profile rules are also reachable through struct-level defaults: a profile
// rule that only overrides `name` still inherits the struct-level `name`
// pattern for fields it does not rename, plus the struct-level `with`.
#[derive(Refield, Debug, PartialEq, Eq)]
#[refield(name = "raw_{}", with = struct_level_negate)]
struct StructLevelProfileTarget {
    enabled: bool,
    #[refield(on(fast), name = "fast_enabled")]
    #[refield(on(thorough))]
    fast: bool,
}

#[test]
fn profiles_inherit_struct_level_with() {
    #[allow(dead_code)]
    struct Raw {
        raw_enabled: bool,
        fast_enabled: bool,
        raw_fast: bool,
    }
    let raw = Raw {
        raw_enabled: true,
        fast_enabled: false,
        raw_fast: true,
    };
    // The default rule reads `raw_fast`, the `fast` profile reads
    // `fast_enabled`, and the `thorough` profile inherits the default source;
    // all three still negate through the struct-level `with`.
    let target = refield!(StructLevelProfileTarget { @fast ..raw });
    assert!(!target.enabled);
    assert!(target.fast);

    let raw = Raw {
        raw_enabled: false,
        fast_enabled: true,
        raw_fast: false,
    };
    let target = refield!(StructLevelProfileTarget { @thorough ..raw });
    assert!(target.enabled);
    assert!(target.fast);

    let raw = Raw {
        raw_enabled: false,
        fast_enabled: false,
        raw_fast: false,
    };
    let target = refield!(StructLevelProfileTarget { ..raw });
    assert!(target.enabled);
    assert!(target.fast);
}

// Struct-level attributes work through `#[refieldable]` arguments too.
#[refieldable(name = "raw_{}")]
#[derive(Debug, PartialEq, Eq)]
struct AttributeMacroArgsTarget {
    port: u32,
    name: String,
}

#[test]
fn struct_level_attributes_are_accepted_as_attribute_macro_arguments() {
    struct Raw {
        raw_port: u32,
        raw_name: String,
    }
    let target = refield!(AttributeMacroArgsTarget {
        ..Raw {
            raw_port: 9,
            raw_name: "macro".into(),
        }
    });
    assert_eq!(
        target,
        AttributeMacroArgsTarget {
            port: 9,
            name: "macro".into(),
        }
    );
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct MergedProfileTarget {
    #[refield(on(fast, slow), name = "alt_port")]
    #[refield(on(fast), with = |p: u16| p * 2)]
    port: u16,
}

#[test]
fn per_profile_attributes_merge_across_multiple_attributes_on_field() {
    struct Raw {
        port: u16,
        alt_port: u16,
    }
    let raw = Raw {
        port: 10,
        alt_port: 20,
    };
    // Default profile reads `port` without transformation
    let def = refield!(MergedProfileTarget { ..raw });
    assert_eq!(def.port, 10);

    // `fast` profile reads `alt_port` and doubles it
    let fast = refield!(MergedProfileTarget { @fast ..raw });
    assert_eq!(fast.port, 40);

    // `slow` profile reads `alt_port` without transformation
    let slow = refield!(MergedProfileTarget { @slow ..raw });
    assert_eq!(slow.port, 20);
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct MultiSegmentProfileTarget {
    #[refield(on(mode::fast), name = "fast_v")]
    value: u8,
}

#[test]
fn multi_segment_path_profile_works_end_to_end() {
    struct Raw {
        value: u8,
        fast_v: u8,
    }
    let raw = Raw {
        value: 1,
        fast_v: 2,
    };
    let target = refield!(MultiSegmentProfileTarget { @mode::fast, ..raw });
    assert_eq!(target.value, 2);
    let default_target = refield!(MultiSegmentProfileTarget { ..raw });
    assert_eq!(default_target.value, 1);
}

// ---------------------------------------------------------------------------
// 31. REVIEW2 #1: attributes are classified by parsed syntax
// ---------------------------------------------------------------------------

// `on` is a bare type name inside the `with` expression's generic arguments.
// A raw-token scan treated it as the profile key and dropped the whole rule.
#[allow(non_camel_case_types)]
struct on;

#[allow(clippy::extra_unused_type_parameters)]
fn bump_generic<T, M>(value: u8) -> u8 {
    value + 1
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct OnArgumentTarget {
    #[refield(with = bump_generic::<(), on>, default = 1)]
    value: u8,
}

#[test]
fn on_inside_a_with_expression_is_not_a_profile() {
    struct Raw {
        value: u8,
    }
    let target = refield!(OnArgumentTarget {
        ..Raw { value: 41 }
    });
    assert_eq!(target, OnArgumentTarget { value: 42 });
    let target = refield!(OnArgumentTarget {});
    assert_eq!(target, OnArgumentTarget { value: 1 });
}

// `crate::identity` in an earlier value must not hide a later `crate = patches`
// entry: `peek_krate` parses each value with its own syntax, so the `crate`
// keyword path inside `with` is never read as the key.
mod krate_struct {
    use crate::patches_reexport::patches;
    use crate::patches_reexport::{refield, refieldable};

    pub fn identity(value: u8) -> u8 {
        value
    }

    // `with = crate::...` comes first on purpose: the `crate` keyword inside
    // the path must not end the scan before the real `crate = patches` entry.
    #[refieldable(with = crate::krate_struct::identity, crate = patches)]
    #[derive(Debug, PartialEq, Eq)]
    pub struct Target {
        pub value: u8,
    }

    pub fn build(raw: u8) -> Target {
        refield!(Target {
            ..Raw { value: raw }
        })
    }

    pub struct Raw {
        pub value: u8,
    }
}

mod patches_reexport {
    pub use refield as patches;
    pub use refield::*;
}

#[test]
fn crate_argument_after_a_path_valued_with_is_honored() {
    assert_eq!(krate_struct::build(7), krate_struct::Target { value: 7 });
}

// ---------------------------------------------------------------------------
// 32. REVIEW2 #2: closure signatures
// ---------------------------------------------------------------------------

#[derive(Refield, Debug, PartialEq, Eq)]
struct ClosureSignatureTarget {
    // The explicit return type is part of the helper's signature.
    #[refield(with = |value: u8| -> u16 { u16::from(value) + 1 })]
    widened: u16,
    // `-> _` keeps the field's type.
    #[refield(with = |value: u8| -> _ { value + 1 })]
    inferred: u8,
    // A block-bodied closure and a block default are spliced in directly, so
    // `#[deny(unused_braces)]` below has nothing to flag.
    #[refield(with = |value: u8| { value + 2 }, default = { 3 + 4 })]
    blocky: u8,
}

#[allow(dead_code)]
struct ClosureSignatureRaw {
    widened: u8,
    inferred: u8,
    blocky: u8,
}

#[test]
#[deny(unused_braces)]
fn closure_returns_and_block_bodies_are_preserved() {
    let target = refield!(ClosureSignatureTarget {
        ..ClosureSignatureRaw {
            widened: 41,
            inferred: 1,
            blocky: 1,
        }
    });
    assert_eq!(
        target,
        ClosureSignatureTarget {
            widened: 42,
            inferred: 2,
            blocky: 3,
        }
    );

    let target = refield!(ClosureSignatureTarget {
        widened: 0,
        inferred: 0,
    });
    assert_eq!(target.blocky, 7);
}

// ---------------------------------------------------------------------------
// 33. REVIEW2 #3: no per-type fallback context macro
// ---------------------------------------------------------------------------

// A local `#[derive(Refield)]` struct inside an annotated `impl` method used to
// shadow `__refield_ctx__` with the derive's fallback, so the following
// `refield!(Self { ... })` calls could no longer resolve.
#[derive(Refield, Debug, PartialEq, Eq)]
struct LocalDeriveOuter {
    a: i32,
}

#[allow(dead_code)]
struct LocalDeriveRaw {
    a: i32,
    b: i32,
}

trait LocalDeriveAssoc {
    type Config;
    fn build(raw: LocalDeriveRaw) -> Self::Config;
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct LocalDeriveConfig {
    a: i32,
    b: i32,
}

struct LocalDeriveService;

#[refieldable]
impl LocalDeriveOuter {
    fn make(raw: LocalDeriveRaw) -> LocalDeriveOuter {
        // The derive inside the method body used to define
        // `macro_rules! __refield_ctx__`, shadowing the `impl`'s context.
        #[derive(Refield)]
        struct Local {
            #[allow(dead_code)]
            value: u8,
        }

        let _local = Local { value: 0 };
        refield!(Self {
            a: raw.a + 1,
            ..raw
        })
    }
}

#[refieldable]
impl LocalDeriveAssoc for LocalDeriveService {
    type Config = LocalDeriveConfig;

    fn build(raw: LocalDeriveRaw) -> Self::Config {
        #[derive(Refield)]
        struct Local {
            #[allow(dead_code)]
            value: u8,
        }

        let _local = Local { value: 1 };
        let base = refield!(Self::Config {
            b: raw.b + 1,
            ..raw
        });
        refield!(Self::Config {
            a: base.a + 1,
            ..base
        })
    }
}

#[test]
fn local_derives_do_not_shadow_the_impl_context() {
    let target = LocalDeriveOuter::make(LocalDeriveRaw { a: 1, b: 2 });
    assert_eq!(target, LocalDeriveOuter { a: 2 });

    let config = LocalDeriveService::build(LocalDeriveRaw { a: 3, b: 4 });
    assert_eq!(config, LocalDeriveConfig { a: 4, b: 5 });
}

// A user struct keeps methods with the historical helper names; the generated
// helpers are prefixed with the schema identity, so both coexist.
#[derive(Refield, Debug, PartialEq, Eq)]
struct UserHelperNames {
    #[refield(with = |value: u8| value + 1, default = 1)]
    a: u8,
}

impl UserHelperNames {
    fn __refield_with_a_0(&self) -> u8 {
        self.a + 100
    }

    fn __refield_default_a_0(&self) -> u8 {
        self.a + 200
    }
}

#[test]
fn user_methods_named_like_generated_helpers_do_not_collide() {
    struct Raw {
        a: u8,
    }
    let target = refield!(UserHelperNames { ..Raw { a: 1 } });
    assert_eq!(target.a, 2);
    assert_eq!(target.__refield_with_a_0(), 102);
    assert_eq!(target.__refield_default_a_0(), 202);

    let target = refield!(UserHelperNames {});
    assert_eq!(target.a, 1);
}

// ---------------------------------------------------------------------------
// 34. REVIEW2 #4: `?Sized` structs derive cleanly
// ---------------------------------------------------------------------------

#[derive(Refield, Debug, PartialEq, Eq)]
struct UnsizedHolder<T: ?Sized> {
    head: u8,
    #[refield(with = |value| value)]
    tail: T,
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct UnsizedRef<'a, T: ?Sized> {
    #[refield(with = |value: u32| value + 1)]
    id: u32,
    ptr: &'a T,
}

#[test]
fn unsized_type_parameters_derive_and_construct() {
    #[allow(dead_code)]
    struct HolderRaw {
        head: u8,
        tail: u8,
    }

    let raw = HolderRaw { head: 1, tail: 7 };
    let held = refield!(UnsizedHolder::<u8> { ..raw });
    assert_eq!(held.head, 1);
    assert_eq!(held.tail, 7);

    #[allow(dead_code)]
    struct RefRaw<'a> {
        id: u32,
        ptr: &'a str,
    }
    let text = "unsized";
    let raw = RefRaw { id: 41, ptr: text };
    let target: UnsizedRef<'_, str> = refield!(UnsizedRef::<str> { ..raw });
    assert_eq!(target.id, 42);
    assert_eq!(target.ptr, "unsized");
}

// ---------------------------------------------------------------------------
// 35. REVIEW2 #5: the `impl` block's `crate` is used inside the context macro
// ---------------------------------------------------------------------------

mod impl_crate {
    use crate::patches_reexport::patches;
    use crate::patches_reexport::{Refield, refield, refieldable};

    pub struct Raw {
        pub value: u16,
    }

    #[derive(Refield, Debug, PartialEq, Eq)]
    pub struct Target {
        pub value: u16,
    }

    // The generated context macro and the companion calls it forwards to use
    // the `crate` path resolved from this `impl` block.
    #[refieldable(crate = patches)]
    impl Target {
        pub fn build(raw: Raw) -> Self {
            refield!(Self {
                value: raw.value + 1,
                ..raw
            })
        }
    }
}

#[test]
fn impl_level_crate_argument_drives_the_context_macro() {
    let target = impl_crate::Target::build(impl_crate::Raw { value: 41 });
    assert_eq!(target, impl_crate::Target { value: 42 });
}

// ---------------------------------------------------------------------------
// 36. Pseudo-closures: `_` inputs and heterogeneous parameter types
// ---------------------------------------------------------------------------

#[derive(Refield, Debug, PartialEq, Eq)]
struct HelperClosureTarget {
    // A typed parameter names a source type that differs from the field type.
    #[refield(with = |value: &'static str| value.to_owned())]
    text: String,
    // An underscore input takes the field type instead of reaching the method
    // signature (which would be an E0121).
    #[refield(with = |value: _| value.wrapping_mul(2))]
    doubled: u8,
}

#[allow(dead_code)]
struct HelperClosureRaw {
    text: &'static str,
    doubled: u8,
}

#[test]
fn pseudo_closures_with_typed_and_underscore_inputs_are_promoted() {
    let target = refield!(HelperClosureTarget {
        ..HelperClosureRaw {
            text: "hello",
            doubled: 21,
        }
    });
    assert_eq!(
        target,
        HelperClosureTarget {
            text: "hello".into(),
            doubled: 42,
        }
    );
}

// ---------------------------------------------------------------------------
// 37. An elided lifetime inside a generic argument gets no `Sized` predicate
// ---------------------------------------------------------------------------

#[derive(Refield, Debug, PartialEq, Eq)]
struct CowTarget {
    // `Cow<'_, str>` mentions an elided lifetime, so a
    // `where Cow<'_, str>: Sized` predicate would be E0637.
    #[refield(with = |value: Cow<'_, str>| value.into_owned())]
    text: String,
}

#[allow(dead_code)]
struct CowRaw<'a> {
    text: Cow<'a, str>,
}

#[test]
fn elided_lifetime_arguments_derive_without_a_sized_predicate() {
    let target = refield!(CowTarget {
        ..CowRaw {
            text: Cow::Borrowed("borrowed"),
        }
    });
    assert_eq!(target.text, "borrowed");

    let target = refield!(CowTarget {
        ..CowRaw {
            text: Cow::Owned(String::from("owned")),
        }
    });
    assert_eq!(target.text, "owned");
}

// ---------------------------------------------------------------------------
// 38. The call site's runtime path wins over a definition-site `crate` alias
// ---------------------------------------------------------------------------

mod local_crate_alias {
    // The alias exists only inside this module. The schema captured the alias
    // at its definition site, but the call below is expanded elsewhere.
    use refield as patches;

    #[derive(patches::Refield, Debug, PartialEq, Eq)]
    #[refield(crate = patches)]
    pub struct Target {
        pub value: u8,
    }
}

#[test]
fn call_site_crate_path_does_not_leak_the_definition_site_alias() {
    let target = refield!(local_crate_alias::Target { value: 7 });
    assert_eq!(target, local_crate_alias::Target { value: 7 });
}

#[test]
fn facade_only_dependency_falls_back_to_the_definition_site_crate() {
    // `refield-test-facade-consumer` has no direct `refield` dependency: the
    // call site cannot resolve a runtime path, so the definition-site
    // `crate = facade` must be used.
    let target =
        refield_test_facade_consumer::build(refield_test_facade_consumer::Raw { value: 3 });
    assert_eq!(target, refield_test_facade_consumer::Target { value: 3 });

    let converted =
        refield_test_facade_consumer::build_converted(refield_test_facade_consumer::Raw {
            value: 3,
        });
    assert_eq!(
        converted,
        refield_test_facade_consumer::ConvertedTarget { value: 4 }
    );
}

// ---------------------------------------------------------------------------
// 39. `cfg` on explicit fields selects the automatic field instead
// ---------------------------------------------------------------------------

#[derive(Refield, Debug, PartialEq, Eq)]
struct CfgFieldTarget {
    value: u8,
    other: u8,
}

#[allow(dead_code)]
struct CfgFieldRaw {
    value: u8,
    other: u8,
}

#[test]
fn cfg_disabled_explicit_fields_let_the_automatic_field_apply() {
    // The explicit `value` is stripped, so the automatic extraction fills it
    // from `raw`; `other` keeps its explicit value.
    let target = refield!(CfgFieldTarget {
        CfgFieldRaw { value: 7, other: 0 }..,
        #[cfg(any())]
        value: 200,
        other: 5,
    });
    assert_eq!(target, CfgFieldTarget { value: 7, other: 5 });

    // The enabled branch wins and suppresses the automatic field.
    let target = refield!(CfgFieldTarget {
        CfgFieldRaw { value: 7, other: 1 }..,
        #[cfg(all())]
        value: 9,
    });
    assert_eq!(target, CfgFieldTarget { value: 9, other: 1 });
}

#[test]
fn mutually_exclusive_explicit_candidates_keep_one_initializer() {
    // Both candidates are `cfg`-gated on the same predicate; exactly one
    // survives stripping and the automatic field must not appear at all.
    let target = refield!(CfgFieldTarget {
        CfgFieldRaw { value: 7, other: 2 }..,
        #[cfg(all())]
        value: 10,
        #[cfg(any())]
        value: 20,
    });
    assert_eq!(
        target,
        CfgFieldTarget {
            value: 10,
            other: 2
        }
    );
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct CfgAttrFieldTarget {
    #[refield(with = |value: u8| value + 1)]
    value: u8,
    other: u8,
}

#[test]
fn cfg_attr_cfg_on_an_explicit_field_is_honored() {
    let target = refield!(CfgAttrFieldTarget {
        CfgFieldRaw { value: 5, other: 3 }..,
        #[cfg_attr(all(), cfg(any()))]
        value: 100,
    });
    assert_eq!(target, CfgAttrFieldTarget { value: 6, other: 3 });

    let target = refield!(CfgAttrFieldTarget {
        CfgFieldRaw { value: 5, other: 3 }..,
        #[cfg_attr(any(), cfg(any()))]
        value: 100,
    });
    assert_eq!(
        target,
        CfgAttrFieldTarget {
            value: 100,
            other: 3
        }
    );
}

#[test]
fn cfg_explicit_field_with_a_value_source_stays_single_evaluation() {
    let log = Rc::new(RefCell::new(Vec::new()));
    let make_raw = || {
        log.borrow_mut().push("rest");
        CfgFieldRaw { value: 7, other: 1 }
    };
    let target = refield!(CfgFieldTarget {
        make_raw()..,
        #[cfg(any())]
        value: {
            log.borrow_mut().push("explicit value");
            100
        },
        other: {
            log.borrow_mut().push("explicit other");
            5
        },
    });
    assert_eq!(target, CfgFieldTarget { value: 7, other: 5 });
    assert_eq!(*log.borrow(), vec!["rest", "explicit other"]);

    // With both explicit entries active the value source is still evaluated
    // exactly once, before the explicit fields.
    log.borrow_mut().clear();
    let target = refield!(CfgFieldTarget {
        make_raw()..,
        #[cfg(all())]
        value: {
            log.borrow_mut().push("explicit value");
            100
        },
        other: {
            log.borrow_mut().push("explicit other");
            5
        },
    });
    assert_eq!(
        target,
        CfgFieldTarget {
            value: 100,
            other: 5
        }
    );
    assert_eq!(
        *log.borrow(),
        vec!["rest", "explicit value", "explicit other"]
    );
}

// ---------------------------------------------------------------------------
// 40. Explicit field attributes are processed by the compiler first
//
// A written field carrying any attribute routes the construction through a
// carrier struct, so `rustc` itself strips disabled fields and expands
// `cfg_attr` before the macro decides which automatic fields to emit.
// ---------------------------------------------------------------------------

#[derive(Refield, Debug, PartialEq, Eq)]
struct PreparedTarget {
    value: u8,
    other: u8,
    converted: u8,
}

#[allow(dead_code)]
struct PreparedRaw {
    value: u8,
    other: u8,
    converted: u8,
}

fn prepared_raw() -> PreparedRaw {
    PreparedRaw {
        value: 7,
        other: 0,
        converted: 3,
    }
}

#[test]
fn disabled_explicit_fields_never_leak_their_expressions() {
    let target = refield!(PreparedTarget {
        prepared_raw()..,
        // The disabled field names both an undefined macro and an unresolved
        // name; neither may reach the final expansion.
        #[cfg(any())]
        value: undefined_macro!(missing_name),
        #[cfg(any())]
        converted: missing_name + 1,
        other: 5,
    });
    assert_eq!(
        target,
        PreparedTarget {
            value: 7,
            other: 5,
            converted: 3,
        }
    );
}

#[test]
fn nested_cfg_attr_and_multiple_cfg_predicates_follow_rustc() {
    // A nested `cfg_attr` is expanded recursively by the compiler.
    let target = refield!(PreparedTarget {
        prepared_raw()..,
        #[cfg_attr(all(), cfg_attr(all(), cfg(any())))]
        value: 100,
        #[cfg_attr(all(), cfg(any()))]
        converted: 200,
        #[cfg(all())]
        #[cfg(any())]
        other: 300,
    });
    assert_eq!(
        target,
        PreparedTarget {
            value: 7,
            other: 0,
            converted: 3
        }
    );

    // A true `cfg_attr` that expands to a true `cfg` keeps the field.
    let target = refield!(PreparedTarget {
        prepared_raw()..,
        #[cfg_attr(all(), cfg(all()))]
        value: 100,
    });
    assert_eq!(
        target,
        PreparedTarget {
            value: 100,
            other: 0,
            converted: 3,
        }
    );
}

#[test]
fn mutually_exclusive_cfg_attr_candidates_keep_one_initializer() {
    let target = refield!(PreparedTarget {
        prepared_raw()..,
        #[cfg_attr(all(), cfg(all()))]
        value: 10,
        #[cfg_attr(all(), cfg(any()))]
        value: 20,
    });
    assert_eq!(
        target,
        PreparedTarget {
            value: 10,
            other: 0,
            converted: 3,
        }
    );
}

#[test]
#[deny(unused_variables)]
fn non_cfg_attributes_reach_the_kept_initializer() {
    // `cfg_attr` expands to a lint attribute, which must still apply to the
    // kept field's expression after the carrier is discarded.
    let target = refield!(PreparedTarget {
        prepared_raw()..,
        #[cfg_attr(all(), allow(unused_variables))]
        value: {
            let unused = 1;
            42
        },
        other: 1,
    });
    assert_eq!(
        target,
        PreparedTarget {
            value: 42,
            other: 1,
            converted: 3,
        }
    );
}

// The same attribute handling applies when the construction does not come
// through `refield!`: direct companion calls, annotated aliases, and the
// `Self`/associated-type spellings of an `#[refieldable] impl`.

#[refieldable]
type PreparedAlias = PreparedTarget;

// The `crate = patches` configuration of an annotated `impl` is resolved at
// the definition site; a carrier for its body must call the same runtime path.
mod prepared_crate {
    use crate::patches_reexport::patches;
    use crate::patches_reexport::{refield, refieldable};

    pub struct Raw {
        pub value: u8,
        pub other: u8,
    }

    #[derive(patches::Refield, Debug, PartialEq, Eq)]
    pub struct Target {
        pub value: u8,
        pub other: u8,
    }

    #[refieldable(crate = patches)]
    impl Target {
        pub fn build(raw: Raw) -> Self {
            refield!(Self {
                #[cfg(any())]
                value: missing_name,
                ..raw
            })
        }
    }
}

trait PreparedBuilder {
    type Config;
    fn build(raw: PreparedRaw) -> Self::Config;
}

struct PreparedService;

#[refieldable]
impl PreparedBuilder for PreparedService {
    type Config = PreparedTarget;

    fn build(raw: PreparedRaw) -> Self::Config {
        refield!(Self::Config {
            #[cfg(any())]
            value: 100,
            other: 8,
            ..raw
        })
    }
}

#[test]
fn every_construction_entry_processes_attributes_the_same_way() {
    let direct = PreparedTarget! {
        prepared_raw()..,
        #[cfg(any())]
        value: 100,
        other: 2,
    };
    assert_eq!(
        direct,
        PreparedTarget {
            value: 7,
            other: 2,
            converted: 3
        }
    );

    let alias = PreparedAlias! {
        prepared_raw()..,
        #[cfg(any())]
        value: 100,
    };
    assert_eq!(
        alias,
        PreparedTarget {
            value: 7,
            other: 0,
            converted: 3
        }
    );

    let via_refield = refield!(PreparedAlias as PreparedTarget {
        prepared_raw()..,
        #[cfg(any())]
        value: 100,
    });
    assert_eq!(
        via_refield,
        PreparedTarget {
            value: 7,
            other: 0,
            converted: 3
        }
    );

    let via_self = PreparedService::build(prepared_raw());
    assert_eq!(
        via_self,
        PreparedTarget {
            value: 7,
            other: 8,
            converted: 3
        }
    );

    let via_assoc = refield!(<PreparedService as PreparedBuilder>::Config as PreparedTarget {
        prepared_raw()..,
        #[cfg(any())]
        value: 100,
        other: 9,
    });
    assert_eq!(
        via_assoc,
        PreparedTarget {
            value: 7,
            other: 9,
            converted: 3
        }
    );

    let via_impl_crate = prepared_crate::Target::build(prepared_crate::Raw { value: 3, other: 4 });
    assert_eq!(
        via_impl_crate,
        prepared_crate::Target { value: 3, other: 4 }
    );
}

#[test]
fn value_rest_with_attributes_keeps_evaluation_and_drop_order() {
    LOG.lock().unwrap().clear();

    let target = refield!(DeferredTarget {
        make_raw(&LOG)..,
        #[cfg(all())]
        a: {
            record(&LOG, "explicit field");
            10
        },
        b: 20,
    });

    assert_eq!(target, DeferredTarget { a: 10, b: 20, c: 7 });
    assert_eq!(
        *LOG.lock().unwrap(),
        vec!["make_raw", "make_default", "explicit field", "drop raw"]
    );
}

#[test]
fn attributes_keep_a_place_source_usable_after_a_partial_move() {
    let raw = RawMove {
        id: NonClone("id-002".into()),
        name: NonClone("gateway".into()),
        payload: NonClone("body".into()),
        extra: NonClone("dropped".into()),
    };

    let moved_id = raw.id;
    let parsed = refield!(ParsedMove {
        #[cfg(all())]
        id_len: moved_id.0.len() + raw.name.0.len(),
        ..raw
    });
    assert_eq!(parsed.id_len, 6 + 7);
    assert_eq!(parsed.name, NonClone("gateway".into()));
}

mod prepared_flow {
    use super::{PreparedRaw, PreparedTarget};
    use refield::refield;

    // The control flow below runs inside the field expression, so the helper
    // reuses the caller's local variables, `?`, `return`, and labels.
    pub fn try_build(raw: PreparedRaw, fail: bool) -> Result<PreparedTarget, &'static str> {
        let base: u8 = if fail { Err("failed")? } else { 10 };
        Ok(refield!(PreparedTarget {
            #[cfg(all())]
            value: {
                let parsed: u8 = "5".parse().map_err(|_| "parse")?;
                parsed + base
            },
            ..raw
        }))
    }

    pub fn early_return(raw: PreparedRaw, early: bool) -> PreparedTarget {
        refield!(PreparedTarget {
            #[cfg(all())]
            value: if early {
                return PreparedTarget {
                    value: 99,
                    other: 0,
                    converted: 0,
                };
            } else {
                1
            },
            ..raw
        })
    }

    #[allow(unreachable_code)]
    pub fn break_with_label(raw: PreparedRaw) -> PreparedTarget {
        // A labeled block: the written field expression breaks out of it and
        // the `..rest` is never reached.
        'outer: {
            refield!(PreparedTarget {
                #[cfg(all())]
                value: break 'outer PreparedTarget {
                    value: raw.value + 50,
                    other: raw.other,
                    converted: raw.converted,
                },
                ..raw
            })
        }
    }

    // `#[refield(const)]` helpers and the carrier both work in a `const fn`.
    pub const fn build_const(raw: PreparedRaw) -> PreparedTarget {
        refield!(PreparedTarget {
            #[cfg(all())]
            value: raw.value + 10,
            ..raw
        })
    }

    pub async fn bump(value: u8) -> u8 {
        value + 1
    }

    pub async fn build_async(raw: PreparedRaw) -> PreparedTarget {
        refield!(PreparedTarget {
            #[cfg(all())]
            value: bump(raw.value).await,
            ..raw
        })
    }

    pub fn block_on<F: core::future::Future>(future: F) -> F::Output {
        use core::task::{Context, Poll, Waker};

        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = core::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("the test future is always ready"),
        }
    }
}

#[test]
fn attributes_leave_question_mark_return_break_and_const_working() {
    let raw = prepared_raw();
    assert_eq!(
        prepared_flow::try_build(raw, false).unwrap(),
        PreparedTarget {
            value: 15,
            other: 0,
            converted: 3
        }
    );
    assert!(prepared_flow::try_build(prepared_raw(), true).is_err());
    assert_eq!(
        prepared_flow::early_return(prepared_raw(), true),
        PreparedTarget {
            value: 99,
            other: 0,
            converted: 0
        }
    );
    assert_eq!(
        prepared_flow::break_with_label(prepared_raw()),
        PreparedTarget {
            value: 57,
            other: 0,
            converted: 3
        }
    );
    const CONST_BUILT: PreparedTarget = prepared_flow::build_const(PreparedRaw {
        value: 1,
        other: 2,
        converted: 3,
    });
    assert_eq!(
        CONST_BUILT,
        PreparedTarget {
            value: 11,
            other: 2,
            converted: 3
        }
    );
}

#[test]
fn attributes_work_across_await_points() {
    let target = prepared_flow::block_on(prepared_flow::build_async(prepared_raw()));
    assert_eq!(
        target,
        PreparedTarget {
            value: 8,
            other: 0,
            converted: 3,
        }
    );
}

#[derive(Refield, Debug, PartialEq, Eq)]
struct PreparedInner {
    value: u8,
}

#[test]
fn a_kept_expression_may_contain_other_macros() {
    // The kept expression is not expanded by `refield!` itself; nested macros
    // expand at the original position, where `inner` is in scope.
    let inner = PreparedInner { value: 4 };
    let target = refield!(PreparedTarget {
        prepared_raw()..,
        #[cfg(all())]
        value: refield!(PreparedInner { ..inner }).value + stringify!(nested).len() as u8,
    });
    assert_eq!(
        target,
        PreparedTarget {
            value: 10,
            other: 0,
            converted: 3,
        }
    );
}

#[test]
fn a_kept_expression_may_define_and_call_dollar_macros() {
    // A `macro_rules!` written inside a kept expression uses `$` tokens that
    // belong to the user's macro, not to the prepared callback that rebuilds
    // the construction. Both an attributed field expression and a value
    // source block are covered here, so the escaping applies to the whole
    // final expression.
    let target = refield!(PreparedTarget {
        {
            macro_rules! count {
                ($($value:expr),*) => {{
                    let mut total = 0;
                    $(total += $value;)*
                    total
                }};
            }
            PreparedRaw {
                value: 0,
                other: count!(4, 5),
                converted: 3,
            }
        }..,
        #[cfg(all())]
        value: {
            macro_rules! sum {
                ($($value:expr),*) => { 0 $(+ $value)* };
            }
            sum!(1, 2, 3)
        },
    });
    assert_eq!(
        target,
        PreparedTarget {
            value: 6,
            other: 9,
            converted: 3,
        }
    );
}

#[test]
fn a_disabled_field_does_not_expand_its_compile_error() {
    let target = refield!(PreparedTarget {
        prepared_raw()..,
        #[cfg(any())]
        value: compile_error!("the disabled field must be stripped before expansion"),
    });
    assert_eq!(
        target,
        PreparedTarget {
            value: 7,
            other: 0,
            converted: 3
        }
    );
}

#[test]
fn prepared_carriers_work_in_a_no_std_crate() {
    let model = models::build_prepared(models::PreparedRaw { value: 4, other: 9 });
    assert_eq!(model, models::PreparedModel { value: 5, other: 9 });
}

#[test]
fn facade_only_dependency_prepares_attributed_fields() {
    let target =
        refield_test_facade_consumer::build_with_attribute(refield_test_facade_consumer::Raw {
            value: 6,
        });
    assert_eq!(target, refield_test_facade_consumer::Target { value: 6 });
}

// ---------------------------------------------------------------------------
// 41. Explicit fields keep their native literal position
// ---------------------------------------------------------------------------

// Written fields are emitted verbatim in the final struct literal, which is
// the tail of the expansion, so they stay native field initializers: coercion
// and inference sites, attributes, parentheses, and temporary lifetime
// extension are the ones of a plain literal.
mod native_positions {
    use super::*;

    #[allow(dead_code)]
    struct Raw {
        g: String,
        n: u8,
    }

    #[derive(Refield)]
    #[allow(dead_code)]
    struct InferenceTarget<X> {
        f: fn(&X) -> usize,
        g: X,
        n: u8,
    }

    #[derive(Refield)]
    struct AutoRaw {
        a: i32,
        b: i32,
    }

    #[derive(Refield, Debug, PartialEq, Eq)]
    struct ParenTarget {
        n: u8,
    }

    #[test]
    fn explicit_fields_keep_closure_inference_and_coercion() {
        // A closure parameter is only known from the field's expected type.
        let target: InferenceTarget<String> = refield!(InferenceTarget {
            Raw {
                g: "text".into(),
                n: 1,
            }..,
            f: |s| s.len(),
        });
        assert_eq!((target.f)(&"abcd".to_owned()), 4);
        assert_eq!(target.n, 1);

        // `&String` coerces to `&str` at the field, not inside a temporary.
        #[derive(Refield, Debug, PartialEq, Eq)]
        struct Borrowing<'a> {
            text: &'a str,
            n: u8,
        }
        let owned = String::from("owned");
        let target = refield!(Borrowing {
            Raw {
                g: String::new(),
                n: 0,
            }..,
            text: &owned,
            n: 2,
        });
        assert_eq!(
            target,
            Borrowing {
                text: "owned",
                n: 2
            }
        );
    }

    #[test]
    #[deny(unused_parens)]
    fn explicit_fields_keep_native_paren_diagnostics() {
        // `n: (1)` is legal in a native literal, so it has to stay legal here.
        let target = refield!(ParenTarget {
            AutoRaw { a: 0, b: 0 }..,
            n: (1),
        });
        assert_eq!(target, ParenTarget { n: 1 });
    }

    #[test]
    fn an_all_explicit_construction_still_evaluates_its_value_source_once() {
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        fn make() -> AutoRaw {
            CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            AutoRaw { a: 1, b: 2 }
        }

        #[derive(Refield, Debug, PartialEq, Eq)]
        struct Both {
            a: i32,
            b: i32,
        }

        let target = refield!(Both {
            make()..,
            a: 10,
            b: 20,
        });
        assert_eq!(target, Both { a: 10, b: 20 });
        assert_eq!(CALLS.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn a_temporary_guard_in_a_written_field_never_overlaps_the_source() {
        let cell = std::cell::RefCell::new(1_i32);
        let make = || AutoRaw {
            a: *cell.borrow() + 10,
            b: 0,
        };

        // A value source is written first, so it has already released its
        // borrow when the written field's `RefMut` is created.
        let target: AutoRaw = refield!(AutoRaw { make().., a: *cell.borrow_mut() + 100 });
        assert_eq!(target.a, 101);
    }
}

// ---------------------------------------------------------------------------
// 42. `#[forbid(deprecated)]` survives a clean schema
// ---------------------------------------------------------------------------

mod forbid_without_deprecations {
    use super::*;

    #[derive(Refield, Debug, PartialEq, Eq)]
    struct Clean {
        a: i32,
        b: i32,
    }

    #[allow(dead_code)]
    struct Raw {
        a: i32,
        b: i32,
    }

    // The macro only emits `#[allow(deprecated)]` for automatic fields whose
    // target field really is deprecated. A schema without any deprecation must
    // therefore compile even where the lint is forbidden.
    #[test]
    #[forbid(deprecated)]
    fn clean_schema_has_no_injected_allow() {
        let target = refield!(Clean {
            Raw { a: 0, b: 2 }..,
            a: 1,
        });
        assert_eq!(target, Clean { a: 1, b: 2 });
    }
}

// ---------------------------------------------------------------------------
// 43. Field-less targets still evaluate their value source once
// ---------------------------------------------------------------------------

mod field_less_targets {
    use super::*;

    static EVALUATIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    #[allow(dead_code)]
    struct Raw {
        x: i32,
    }

    fn make() -> Raw {
        EVALUATIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Raw { x: 1 }
    }

    #[derive(Refield, Debug, PartialEq, Eq)]
    struct Unit;

    #[derive(Refield, Debug, PartialEq, Eq)]
    struct AllCfgRemoved {
        #[cfg(any())]
        removed: NonExistentType,
    }

    #[test]
    fn a_unit_struct_still_evaluates_its_value_source() {
        // A field-less literal has no initializer to carry the source, so the
        // binding moves after the literal; the source still runs exactly once.
        EVALUATIONS.store(0, std::sync::atomic::Ordering::SeqCst);
        let _unit: Unit = refield!(Unit { ..make() });
        assert_eq!(EVALUATIONS.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn a_schema_removed_by_cfg_still_evaluates_its_value_source() {
        EVALUATIONS.store(0, std::sync::atomic::Ordering::SeqCst);
        let _empty: AllCfgRemoved = refield!(AllCfgRemoved { ..make() });
        assert_eq!(EVALUATIONS.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

// ---------------------------------------------------------------------------
// 44. Conventional Rust targets keep their literal shape
// ---------------------------------------------------------------------------

mod literal_shapes {
    use super::*;

    #[derive(Refield, Debug, PartialEq, Eq)]
    struct Tuple(u8, u8);

    #[allow(dead_code)]
    struct TupleRaw(u8, u8);

    #[derive(Refield, Debug, PartialEq, Eq)]
    struct Defaulted<X = String> {
        val: X,
    }

    #[allow(dead_code)]
    struct DefaultedRaw {
        val: u32,
    }

    #[test]
    fn tuple_structs_take_numbered_fields() {
        let target: Tuple = refield!(Tuple { TupleRaw(3, 4).., 0: 9 });
        assert_eq!(target, Tuple(9, 4));
    }

    #[test]
    fn default_generic_parameters_are_not_captured_eagerly() {
        // The field decides `X`; a projection-based design would have used the
        // declared default `String` here instead.
        let target: Defaulted<u32> = refield!(Defaulted { DefaultedRaw { val: 1 }.., val: 42u32 });
        assert_eq!(target.val, 42);
    }
}

// ---------------------------------------------------------------------------
// 45. The written source position decides when the source is read
// ---------------------------------------------------------------------------

mod source_position {
    use super::*;

    static READS: EventLog = EventLog::new(Vec::new());

    #[allow(dead_code)]
    struct Raw {
        auto: i32,
        written: NonClone,
        spare: NonClone,
    }

    fn raw() -> Raw {
        record(&READS, "source");
        Raw {
            auto: 1,
            written: NonClone("written".into()),
            spare: NonClone("spare".into()),
        }
    }

    #[derive(Refield, Debug, PartialEq, Eq)]
    struct Target {
        #[refield(with = |value| {
            record(&READS, "auto");
            value
        })]
        auto: i32,
        written: NonClone,
    }

    fn written(value: &'static str) -> NonClone {
        record(&READS, "written");
        NonClone(value.into())
    }

    #[test]
    fn a_source_is_read_where_it_is_written() {
        READS.lock().unwrap().clear();
        let place = raw();
        let front = refield!(Target { place.., written: written("front") });
        let place = raw();
        let back = refield!(Target {
            written: written("back"),
            ..place
        });
        let value = refield!(Target { raw().., written: written("value") });

        assert_eq!(front.written, NonClone("front".into()));
        assert_eq!(back.written, NonClone("back".into()));
        assert_eq!(value.written, NonClone("value".into()));
        assert_eq!(
            *READS.lock().unwrap(),
            [
                "source", "auto", "written", // `place..`
                "source", "written", "auto", // `..place`
                "source", "auto", "written", // `raw()..`
            ]
        );
    }

    #[test]
    fn a_front_place_keeps_its_unread_fields() {
        let raw = Raw {
            auto: 1,
            written: NonClone("from raw".into()),
            spare: NonClone("spare".into()),
        };
        // `raw.auto` is read first; the written field still moves `raw.written`,
        // and `raw.spare` stays in `raw`.
        let target = refield!(Target { raw.., written: raw.written });
        assert_eq!(
            target,
            Target {
                auto: 1,
                written: NonClone("from raw".into()),
            }
        );
        assert_eq!(raw.spare, NonClone("spare".into()));
    }

    #[test]
    fn a_back_place_lets_written_fields_borrow_the_whole_source() {
        fn describe(raw: &Raw) -> NonClone {
            NonClone(format!("{}+{}", raw.written.0, raw.spare.0))
        }
        let raw = Raw {
            auto: 2,
            written: NonClone("w".into()),
            spare: NonClone("s".into()),
        };
        // The borrow of `raw` ends before `..raw` moves `raw.auto`.
        let target = refield!(Target {
            written: describe(&raw),
            ..raw
        });
        assert_eq!(target.written, NonClone("w+s".into()));
    }

    #[derive(Refield, Debug, PartialEq, Eq)]
    struct Borrowed<'a> {
        auto: i32,
        text: &'a str,
        last: &'a str,
    }

    #[allow(dead_code)]
    struct BorrowedRaw {
        auto: i32,
        text: &'static str,
        last: &'static str,
    }

    fn borrowed_raw() -> BorrowedRaw {
        BorrowedRaw {
            auto: 3,
            text: "raw",
            last: "raw",
        }
    }

    #[test]
    fn written_temporaries_are_extended_like_in_a_native_literal() {
        // Held by `let` across statements: every source position extends the
        // written fields' temporaries to the enclosing block.
        let place = borrowed_raw();
        let front_place = refield!(Borrowed { place.., text: &String::from("a"), last: "x" });
        let place = borrowed_raw();
        let back_place = refield!(Borrowed {
            text: &String::from("b"),
            last: "x",
            ..place
        });
        let front_value =
            refield!(Borrowed { borrowed_raw().., text: &String::from("c"), last: "x" });
        assert_eq!(
            (front_place.text, back_place.text, front_value.text),
            ("a", "b", "c")
        );

        // Consumed in the same statement: the temporaries live to its end.
        let place = borrowed_raw();
        assert_eq!(
            refield!(Borrowed { place.., text: &String::from("d"), last: "x" }).text,
            "d"
        );
        let place = borrowed_raw();
        assert_eq!(
            refield!(Borrowed {
                text: &String::from("e"),
                last: "x",
                ..place
            })
            .text,
            "e"
        );
        assert_eq!(
            refield!(Borrowed { borrowed_raw().., text: &String::from("f"), last: "x" }).text,
            "f"
        );
    }

    #[test]
    fn every_written_field_stays_native_when_all_fields_are_written() {
        static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        fn counted() -> BorrowedRaw {
            CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            borrowed_raw()
        }

        // The source is evaluated once, before the fields, so even the last
        // written field keeps its temporary lifetime extension.
        let target = refield!(Borrowed {
            counted()..,
            auto: 4,
            text: &String::from("first"),
            last: &String::from("last"),
        });
        assert_eq!(
            (target.auto, target.text, target.last),
            (4, "first", "last")
        );
        assert_eq!(CALLS.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[derive(Refield, Debug, PartialEq, Eq)]
    struct ConstPair {
        a: i32,
        b: i32,
    }

    struct ConstPairRaw {
        a: i32,
        b: i32,
    }

    const fn make_const_pair_raw() -> ConstPairRaw {
        ConstPairRaw { a: 1, b: 2 }
    }

    const fn build_front_place(raw: ConstPairRaw) -> ConstPair {
        refield!(ConstPair { raw.., a: raw.a + 10 })
    }

    const fn build_front_value() -> ConstPair {
        refield!(ConstPair { make_const_pair_raw().., a: 20 })
    }

    #[test]
    fn front_sources_work_in_const_fn() {
        const PLACE: ConstPair = build_front_place(ConstPairRaw { a: 5, b: 6 });
        const VALUE: ConstPair = build_front_value();
        assert_eq!(PLACE, ConstPair { a: 15, b: 6 });
        assert_eq!(VALUE, ConstPair { a: 20, b: 2 });
    }

    fn parse_front(text: &str, port: &str) -> Result<ConstPair, core::num::ParseIntError> {
        Ok(refield!(ConstPair {
            ConstPairRaw { a: text.parse()?, b: 0 }..,
            b: port.parse()?,
        }))
    }

    async fn ready<T>(value: T) -> T {
        value
    }

    async fn build_async() -> ConstPair {
        refield!(ConstPair {
            ready(ConstPairRaw { a: 1, b: 2 }).await..,
            b: ready(30).await,
        })
    }

    #[test]
    fn front_sources_keep_question_mark_and_await() {
        assert_eq!(parse_front("7", "8"), Ok(ConstPair { a: 7, b: 8 }));
        assert!(parse_front("x", "8").is_err());
        assert!(parse_front("7", "x").is_err());

        fn assert_send<T: Send>(value: T) -> T {
            value
        }
        let target = prepared_flow::block_on(assert_send(build_async()));
        assert_eq!(target, ConstPair { a: 1, b: 30 });
    }
}

// ---------------------------------------------------------------------------
// 46. `Drop` targets and sources, const generics, bounds, and projections
//     (exploration dimensions D3, D4, D31, D34)
// ---------------------------------------------------------------------------

mod drop_and_const_generic_targets {
    use super::*;
    use std::marker::PhantomData;

    static DROPS: EventLog = EventLog::new(Vec::new());

    #[derive(Refield, Debug)]
    struct Guarded {
        name: String,
        tags: Vec<&'static str>,
    }

    impl Drop for Guarded {
        fn drop(&mut self) {
            record(&DROPS, "drop Guarded");
        }
    }

    struct GuardedRaw {
        name: String,
        tags: Vec<&'static str>,
    }

    fn guarded_raw() -> GuardedRaw {
        GuardedRaw {
            name: "raw".into(),
            tags: vec!["tag"],
        }
    }

    #[test]
    fn a_drop_target_with_non_copy_fields_is_built_in_every_position() {
        DROPS.lock().unwrap().clear();
        let place = guarded_raw();
        let front_place = refield!(Guarded { place.., name: "front place".into() });
        let place = guarded_raw();
        let back_place = refield!(Guarded {
            name: "back place".into(),
            ..place
        });
        let front_value = refield!(Guarded { guarded_raw().., name: "front value".into() });
        let bare_value = refield!(Guarded { ..guarded_raw() });
        assert_eq!(
            (
                front_place.name.as_str(),
                back_place.name.as_str(),
                front_value.name.as_str(),
                bare_value.name.as_str(),
            ),
            ("front place", "back place", "front value", "raw")
        );
        assert_eq!(bare_value.tags, ["tag"]);
        // No intermediate `Guarded` is built, so nothing has been dropped yet.
        assert!(DROPS.lock().unwrap().is_empty());
        drop((front_place, back_place, front_value, bare_value));
        assert_eq!(DROPS.lock().unwrap().len(), 4);
    }

    #[derive(Refield, Debug, PartialEq, Eq)]
    struct Frame<const N: usize> {
        data: [u8; N],
        label: String,
    }

    impl<const N: usize> Drop for Frame<N> {
        fn drop(&mut self) {}
    }

    #[allow(dead_code)]
    struct FrameRaw<const N: usize> {
        data: [u8; N],
        spare: u8,
    }

    const fn frame_from_place<const N: usize>(raw: FrameRaw<N>) -> Frame<N> {
        refield!(Frame { raw.., label: String::new() })
    }

    const fn frame_from_value() -> Frame<2> {
        refield!(Frame { FrameRaw { data: [7, 8], spare: 0 }.., label: String::new() })
    }

    #[test]
    fn const_generic_drop_targets_build_in_const_fn() {
        const PLACE: Frame<3> = frame_from_place(FrameRaw {
            data: [1, 2, 3],
            spare: 0,
        });
        const VALUE: Frame<2> = frame_from_value();
        assert_eq!(PLACE.data, [1, 2, 3]);
        assert_eq!(VALUE.data, [7, 8]);
    }

    trait Codec {
        type Wire;
    }

    struct Json;

    impl Codec for Json {
        type Wire = String;
    }

    #[derive(Refield)]
    struct Message<'a, C: Codec, const N: usize>
    where
        C::Wire: Clone,
    {
        body: C::Wire,
        header: &'a str,
        bytes: [u8; N],
        #[refield(skip, default)]
        codec: PhantomData<C>,
    }

    struct MessageRaw {
        body: String,
        bytes: [u8; 2],
    }

    #[test]
    fn lifetimes_const_generics_bounds_and_projections_instantiate() {
        let header = String::from("header");
        let message = refield!(Message::<Json, 2> {
            MessageRaw {
                body: "body".into(),
                bytes: [1, 2],
            }..,
            header: &header,
        });
        assert_eq!(
            (message.body.as_str(), message.header, message.bytes),
            ("body", "header", [1, 2])
        );
    }

    struct DropRaw {
        n: u8,
        label: &'static str,
    }

    impl Drop for DropRaw {
        fn drop(&mut self) {}
    }

    #[derive(Refield, Debug, PartialEq, Eq)]
    struct Copied {
        n: u8,
        label: &'static str,
    }

    #[test]
    fn copy_fields_are_read_from_a_drop_source() {
        // Moving a non-`Copy` field out of such a source is E0509, exactly as
        // natively; see the compile-fail doctests in `refield/src/lib.rs`.
        let raw = DropRaw { n: 1, label: "raw" };
        let front = refield!(Copied { raw.., n: 2 });
        let back = refield!(Copied { n: 3, ..raw });
        let value = refield!(Copied { DropRaw { n: 4, label: "value" }.. });
        assert_eq!(front, Copied { n: 2, label: "raw" });
        assert_eq!(back, Copied { n: 3, label: "raw" });
        assert_eq!(
            value,
            Copied {
                n: 4,
                label: "value"
            }
        );
    }
}

// ---------------------------------------------------------------------------
// 47. Borrowed value sources and attributed written fields keep their
//     temporaries (D7, D14; Rust 2024 callers are covered by `tests/editions`)
// ---------------------------------------------------------------------------

mod borrowed_sources_and_prepared_temporaries {
    use super::*;

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

    macro_rules! borrowed_raw {
        () => {{ &raw() }};
    }

    #[test]
    fn a_value_source_may_borrow_a_temporary() {
        // The source is bound once, so the temporary it borrows lives as long
        // as that binding, whether the borrow is bare, in a block, or produced
        // by a macro.
        let bare = refield!(View { &raw().., text: "bare" });
        let block = refield!(View { { &raw() }.., text: "block" });
        let from_macro = refield!(View { borrowed_raw!().., text: "macro" });
        assert_eq!((bare.id, block.id, from_macro.id), (1, 1, 1));
    }

    #[test]
    fn attributed_written_fields_keep_their_temporaries() {
        // An attribute routes the construction through the `Prepare` carrier;
        // the source block and the written fields' temporaries still live as
        // they do without it.
        let target = refield!(View { { &raw() }.., #[cfg(all())] id: 7 });
        assert_eq!((target.id, target.text), (7, "raw"));

        let place = raw();
        let front_place = refield!(View { place.., #[cfg(all())] text: &String::from("a") });
        let back_place = refield!(View {
            #[cfg(all())]
            text: &String::from("b"),
            ..place
        });
        let front_value = refield!(View { raw().., #[cfg(all())] text: &String::from("c") });
        let user_block = refield!(View { raw().., #[cfg(all())] text: { &String::from("d") } });
        assert_eq!(
            (
                front_place.text,
                back_place.text,
                front_value.text,
                user_block.text
            ),
            ("a", "b", "c", "d")
        );
        assert_eq!(
            refield!(View { raw().., #[cfg(all())] text: &String::from("ee") })
                .text
                .len(),
            2
        );
    }
}

// ---------------------------------------------------------------------------
// 48. Cleanup when a written field or the source fails (D21-D24)
// ---------------------------------------------------------------------------

mod failure_cleanup {
    use refield::{Refield, refield};
    use std::cell::RefCell;

    thread_local! {
        static LOG: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
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

    #[allow(dead_code)]
    struct Raw {
        a: D,
        b: D,
        spare: D,
    }

    fn raw() -> Raw {
        note("source");
        Raw {
            a: D("drop a"),
            b: D("drop b"),
            spare: D("drop spare"),
        }
    }

    fn fail_raw() -> Result<Raw, ()> {
        note("source fails");
        Err(())
    }

    #[allow(dead_code)]
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

    fn early() -> Result<D, ()> {
        note("written y returns early");
        Err(())
    }

    // Taking the source by value keeps it whole in this frame: a closure
    // capturing it directly would only capture the fields it reads.
    fn back_place_with_a_panicking_field(place: Raw) -> Target {
        refield!(Target {
            x: written("written x", "drop x"),
            y: boom(),
            ..place
        })
    }

    fn question_mark_in_written_field() -> Result<Target, ()> {
        Ok(refield!(Target { raw().., x: written("written x", "drop x"), y: early()? }))
    }

    fn question_mark_in_source() -> Result<Target, ()> {
        Ok(refield!(Target {
            fail_raw()?..,
            x: written("written x", "drop x"),
            y: written("written y", "drop y"),
        }))
    }

    fn question_mark_in_source_with_every_field_written() -> Result<Copyable, ()> {
        Ok(refield!(Copyable { fail_raw()?.., n: 1 }))
    }

    #[allow(dead_code)]
    #[derive(Refield)]
    struct Copyable {
        n: u8,
    }

    #[test]
    fn a_failing_written_field_drops_what_was_evaluated_in_reverse() {
        take_log();
        // A value written first: the automatic fields were already moved out,
        // so they drop after the written field, then the source remainder.
        let result = std::panic::catch_unwind(
            || refield!(Target { raw().., x: written("written x", "drop x"), y: boom() }),
        );
        assert!(result.is_err());
        assert_eq!(
            take_log(),
            [
                "source",
                "auto a",
                "auto b",
                "written x",
                "written y panics",
                "drop x",
                "drop b",
                "drop a",
                "drop spare",
            ]
        );

        assert!(question_mark_in_written_field().is_err());
        assert_eq!(
            take_log(),
            [
                "source",
                "auto a",
                "auto b",
                "written x",
                "written y returns early",
                "drop x",
                "drop b",
                "drop a",
                "drop spare",
            ]
        );

        // A place written last: nothing was moved out of it yet, so the whole
        // source drops in declaration order after the written field.
        let result = std::panic::catch_unwind(|| back_place_with_a_panicking_field(raw()));
        assert!(result.is_err());
        assert_eq!(
            take_log(),
            [
                "source",
                "written x",
                "written y panics",
                "drop x",
                "drop a",
                "drop b",
                "drop spare",
            ]
        );
    }

    #[test]
    fn a_failing_source_evaluates_and_builds_nothing() {
        take_log();
        assert!(question_mark_in_source().is_err());
        assert_eq!(take_log(), ["source fails"]);
        assert!(question_mark_in_source_with_every_field_written().is_err());
        assert_eq!(take_log(), ["source fails"]);
    }
}

// ---------------------------------------------------------------------------
// 49. Inference without an outer annotation and `Option` coercion (D9, D29,
//     D30)
// ---------------------------------------------------------------------------

mod unannotated_inference {
    use super::*;

    struct Raw {
        n: u8,
    }

    #[derive(Refield)]
    struct Callback<T> {
        value: T,
        f: fn(&T) -> usize,
        n: u8,
    }

    #[test]
    fn a_typed_field_decides_the_generic_before_a_closure() {
        // No annotation anywhere: `value: 42u32` fixes `T`, then the closure
        // parameter is inferred from `f`'s field type.
        let callback = refield!(Callback {
            Raw { n: 1 }..,
            value: 42u32,
            f: |value| *value as usize,
        });
        assert_eq!(((callback.f)(&callback.value), callback.n), (42, 1));
    }

    #[derive(Refield)]
    struct WithDefault<X = String> {
        val: X,
        n: u8,
    }

    fn type_name_of<T>(_: &T) -> &'static str {
        std::any::type_name::<T>()
    }

    #[test]
    fn a_written_field_overrides_a_default_generic_without_annotation() {
        let place = Raw { n: 2 };
        let target = refield!(WithDefault {
            val: 42u32,
            ..place
        });
        assert!(type_name_of(&target.val).ends_with("u32"));
        assert_eq!((target.val, target.n), (42, 2));
    }

    #[derive(Refield)]
    struct Named<'a> {
        #[refield(default = "anonymous")]
        name: &'a str,
    }

    struct NamedRaw<'a> {
        name: &'a String,
    }

    #[test]
    fn a_source_field_coerces_to_the_field_type() {
        // `&String` becomes `&str` at the field, like a native literal.
        let owned = String::from("owned");
        let named = refield!(Named {
            ..NamedRaw { name: &owned }
        });
        assert_eq!(named.name, "owned");
        let named = refield!(Named {});
        assert_eq!(named.name, "anonymous");
    }
}
