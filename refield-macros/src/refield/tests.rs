use super::*;
use crate::refield::args::FunctionArg;
use quote::{ToTokens, quote};

fn error_message<T, E: std::fmt::Display>(result: std::result::Result<T, E>) -> String {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(error) => error.to_string(),
    }
}

/// Expand a derive and parse the generated items back with `syn`, so the
/// tests can inspect the inherent impl block and the companion macro.
fn derive_file(input: TokenStream) -> syn::File {
    let output = derive(input).expect("derive should expand");
    parse2::<syn::File>(output).expect("generated output should parse as items")
}

fn derived_impl(file: &syn::File) -> &syn::ItemImpl {
    file.items
        .iter()
        .find_map(|item| match item {
            Item::Impl(item_impl) => Some(item_impl),
            _ => None,
        })
        .expect("the derive emits one inherent impl block")
}

fn impl_methods(item_impl: &syn::ItemImpl) -> Vec<&syn::ImplItemFn> {
    item_impl
        .items
        .iter()
        .filter_map(|item| match item {
            syn::ImplItem::Fn(func) => Some(func),
            _ => None,
        })
        .collect()
}

/// Whether a generated method matches `name`.
///
/// Helper methods are prefixed with the schema identity (`#check_ident`),
/// which is unique per derive. A lookup written as `__refield_with_a_0`
/// therefore matches `__refield_check_Sample_<hash>_<seq>_with_a_0` by
/// suffix, while any other name must match exactly.
fn helper_name_matches(actual: &str, name: &str) -> bool {
    if name.starts_with("__refield_") {
        actual.ends_with(&name["__refield".len()..])
    } else {
        actual == name
    }
}

fn helper_method<'a>(item_impl: &'a syn::ItemImpl, name: &str) -> &'a syn::ImplItemFn {
    impl_methods(item_impl)
        .into_iter()
        .find(|func| helper_name_matches(&func.sig.ident.to_string(), name))
        .unwrap_or_else(|| panic!("expected a generated method named `{name}`"))
}

fn helper_count(item_impl: &syn::ItemImpl, name: &str) -> usize {
    impl_methods(item_impl)
        .into_iter()
        .filter(|func| helper_name_matches(&func.sig.ident.to_string(), name))
        .count()
}

fn check_method(item_impl: &syn::ItemImpl) -> &syn::ImplItemFn {
    impl_methods(item_impl)
        .into_iter()
        .find(|func| func.sig.ident.to_string().starts_with("__refield_check_"))
        .expect("every schema generates an identity check method")
}

/// The companion macro is named `__refield__<Struct>__IN__<hash>_<seq>`;
/// the fallback context macro is `__refield_ctx__`, so the prefix filter
/// picks out the schema carrier.
fn companion_macro<'a>(file: &'a syn::File, struct_name: &str) -> &'a syn::ItemMacro {
    let prefix = format!("__refield__{struct_name}__IN__");
    file.items
        .iter()
        .find_map(|item| match item {
            Item::Macro(item_macro)
                if item_macro
                    .ident
                    .as_ref()
                    .is_some_and(|ident| ident.to_string().starts_with(&prefix)) =>
            {
                Some(item_macro)
            }
            _ => None,
        })
        .expect("every derive emits a companion macro")
}

fn companion_macro_ident(file: &syn::File, struct_name: &str) -> String {
    companion_macro(file, struct_name)
        .ident
        .as_ref()
        .expect("the companion macro has a name")
        .to_string()
}

fn assert_error<T, E: std::fmt::Display>(result: std::result::Result<T, E>, expected: &str) {
    let message = error_message(result);
    assert!(
        message.contains(expected),
        "expected `{expected}`, got `{message}`"
    );
}

/// Drive the same pipeline `rustc` uses: `proc`, then `derive` for structs
/// or `prepare` for carriers generated from `impl` blocks.
fn expand_refieldable(args: TokenStream, input: TokenStream) -> Result<TokenStream> {
    let expanded = proc(args, &input)?;
    if let Ok(file) = parse2::<syn::File>(expanded.clone())
        && let Some(Item::Struct(s)) = file.items.first()
    {
        if s.ident == "__RefieldCarrier" {
            return prepare(&expanded);
        }
        if file.items.len() == 1 {
            return derive(expanded);
        }
    }
    Ok(expanded)
}

fn construct_input(body: TokenStream) -> TokenStream {
    quote! {
        (::refield),
        __refield_check_test,
        [
            {
                value,
                { value, false, (), () },
                [],
                false,
            },
        ],
        (Sample),
        (Sample),
        { #body }
    }
}

fn construct_profile_input(profiles: TokenStream, body: TokenStream) -> TokenStream {
    use syn::parse::Parser;
    let profile_paths: Vec<syn::Path> = (|input: syn::parse::ParseStream<'_>| {
        syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated_with(
            input,
            syn::Path::parse_mod_style,
        )
    })
    .parse2(profiles)
    .expect("test profiles are comma-separated paths")
    .into_iter()
    .collect();
    quote! {
        (::refield),
        __refield_check_test,
        [
            {
                value,
                { value, false, (), () },
                [ ([ #(#profile_paths),* ], { value, false, (), () }) ],
                false,
            },
        ],
        (Sample),
        (Sample),
        { #body }
    }
}

#[test]
fn derive_rejects_enums() {
    assert_error(
        derive(quote! { enum Sample { A } }),
        "can only be derived for structs",
    );
}

#[test]
fn proc_rejects_functions() {
    assert_error(
        proc(TokenStream::new(), &quote! { fn sample() {} }),
        "can only be applied to structs, type aliases, or `impl` blocks",
    );
}

#[test]
fn proc_rejects_as_on_structs_and_impls() {
    assert_error(
        expand_refieldable(
            quote! { as = Target },
            quote! { struct Sample { value: u8 } },
        ),
        "Unknown field: `as`. Available values: `const`, `crate`, `name`, `with`",
    );
    assert_error(
        expand_refieldable(
            quote! { as = Target },
            quote! { impl Sample { fn sample() {} } },
        ),
        "Unknown field: `as`. Available values: `crate`",
    );
    assert_error(
        derive(quote! {
            #[refield(as = Target)]
            struct Sample { value: u8 }
        }),
        "Unknown field: `as`. Available values: `const`, `crate`, `name`, `with`",
    );
}

#[test]
fn expand_item_accepts_as_on_type_aliases() {
    let output = expand_refieldable(
        quote! { as = Target, crate = ::refield },
        quote! {
            type Alias = <T as Trait>::Assoc;
        },
    )
    .expect("`as` is accepted on type aliases")
    .to_string();
    assert!(
        output.contains("@ __refield_alias"),
        "expected definition-site schema forwarding, got `{output}`"
    );
    assert!(output.contains("Target"), "got `{output}`");
}

#[test]
fn expand_item_accepts_empty_and_crate_only_arguments() {
    for args in [TokenStream::new(), quote! { crate = ::refield }] {
        let output = expand_refieldable(
            quote! { as = Target, #args },
            quote! {
                type Alias = <T as Trait>::Assoc;
            },
        )
        .expect("`as` is accepted on type aliases")
        .to_string();
        assert!(
            output.contains("@ __refield_alias"),
            "expected definition-site schema forwarding, got `{output}`"
        );
        assert!(output.contains("Target"), "got `{output}`");
    }
}

#[test]
fn expand_item_rejects_as_on_impls() {
    assert_error(
        expand_refieldable(
            quote! { crate = ::refield },
            quote! {
                #[refield(as = Target)]
                impl Sample { fn sample() {} }
            },
        ),
        "`refield` attributes are not supported on `impl` blocks; pass arguments to `#[refieldable(...)]`",
    );
}

#[test]
fn expand_item_rejects_impls_without_nominal_paths() {
    assert_error(
        expand_refieldable(
            quote! { crate = ::refield },
            quote! { impl Trait for () { fn sample() {} } },
        ),
        "require a nominal target `Self` type or associated type definitions",
    );
}

#[test]
fn expand_item_accepts_non_nominal_impl_with_associated_types() {
    let output = expand_refieldable(
        quote! { crate = ::refield },
        quote! {
            impl Trait for () {
                type Config = Target;
                fn build() -> Self::Config {
                    refield!(Self::Config { ..raw })
                }
            }
        },
    )
    .expect("associated type definitions provide the schema")
    .to_string();
    assert!(
        output.contains("macro_rules ! __refield_ctx__"),
        "expected a local context macro, got `{output}`"
    );
    assert!(
        output.contains("Target ! { @ __refield_call"),
        "the `@assoc` arm forwards to the schema's companion macro: `{output}`"
    );
    assert!(
        output.contains("const _ : () = {"),
        "expected the impl to be wrapped in an anonymous const: `{output}`"
    );
}

#[test]
fn expand_item_rewrites_impl_associated_type_forms() {
    let output = expand_refieldable(
        quote! { crate = ::refield },
        quote! {
            impl Trait for Service {
                type Config = Target;
                fn build() -> Self::Config {
                    let a = refield!(<Self as Trait>::Config { ..raw });
                    let b = refield!(Self::Config { ..raw });
                    refield!(Self::Config { ..raw })
                }
            }
        },
    )
    .expect("associated types are collected")
    .to_string();
    // The method body is not rewritten: `refield!` expands later and
    // requests context through the unqualified macro.
    assert!(
        output.contains("refield ! (< Self as Trait > :: Config {"),
        "the qualified invocation must stay intact: `{output}`"
    );
    assert!(
        output.contains("refield ! (Self :: Config {"),
        "the `Self::Config` invocation must stay intact: `{output}`"
    );
    assert!(
        output.contains("@ assoc Config"),
        "expected a single `@assoc` arm for both spellings: `{output}`"
    );
    assert!(
        !output.contains("qassoc"),
        "the trait name must not be matched by the context macro: `{output}`"
    );
    assert!(
        output.contains("Target ! { @ __refield_call"),
        "expected both arms to forward to `Target!`, got `{output}`"
    );
}

#[test]
fn strip_refield_attrs_clears_struct_and_field_attributes() {
    let output = strip(&quote! {
        #[refield(invalid)]
        #[derive(Debug)]
        struct Sample {
            #[refield(skip)]
            value: u8,
            #[derive(Clone)]
            other: u8,
        }
    })
    .to_string();
    assert!(
        !output.contains("refield"),
        "expected every `refield` attribute to be stripped, got `{output}`"
    );
    assert!(output.contains("derive"), "got `{output}`");
    assert!(output.contains("Debug"), "got `{output}`");
    assert!(output.contains("Clone"), "got `{output}`");
}

#[test]
fn proc_rejects_const_on_type_alias_and_impl() {
    assert_error(
        expand_refieldable(quote! { const }, quote! { type Alias = Target; }),
        "Unknown field: `const`. Available values: `as`, `crate`",
    );
    assert_error(
        expand_refieldable(quote! { const }, quote! { impl Target { fn sample() {} } }),
        "Unknown field: `const`. Available values: `crate`",
    );
}

#[test]
fn field_rejects_skip_combined_with_other_attributes() {
    for attribute in [
        quote! { skip, with = |value: u8| value },
        quote! { skip, name = "value" },
    ] {
        let input = quote! {
            struct Sample {
                #[refield(#attribute)]
                value: u8,
            }
        };
        assert_error(derive(input), "`skip` can only be combined with `default`");
    }
}

#[test]
fn field_rejects_empty_profile_list() {
    let input = quote! {
        struct Sample {
            #[refield(on())]
            value: u8,
        }
    };
    assert_error(derive(input), "`on` requires at least one profile name");

    let input_bare = quote! {
        struct Sample {
            #[refield(on)]
            value: u8,
        }
    };
    assert_error(derive(input_bare), "Unexpected meta-item format `word`");
}

#[test]
fn field_rejects_non_path_profiles() {
    for attribute in [
        quote! { on(1) },
        quote! { on("not a path") },
        quote! { on(a = 1) },
    ] {
        let input = quote! {
            struct Sample {
                #[refield(#attribute)]
                value: u8,
            }
        };
        assert_error(derive(input), "Unexpected type `non-word`");
    }
    assert_error(
        derive(quote! {
            struct Sample {
                #[refield(on = fast)]
                value: u8,
            }
        }),
        "Unexpected type `path`",
    );

    // Path-like profile names are accepted.
    derive(quote! {
        struct Sample {
            #[refield(on(some::path))]
            value: u8,
        }
    })
    .expect("path profiles are accepted");
}

#[test]
fn field_rejects_duplicate_profile_keys() {
    let input = quote! {
        struct Sample {
            #[refield(on(fast), name = "a")]
            #[refield(on(fast), name = "b")]
            value: u8,
        }
    };
    assert_error(derive(input), "Duplicate field `name`");
}

#[test]
fn multiple_on_attributes_with_distinct_keys_merge() {
    let file = derive_file(quote! {
        struct Sample {
            #[refield(on(fast), name = "a")]
            #[refield(on(fast), with = |v: u8| v + 1)]
            value: u8,
        }
    });
    let schema = companion_macro(&file, "Sample").mac.tokens.to_string();
    assert!(schema.contains("a , false"), "got `{schema}`");
    assert!(schema.contains("_with_value_1"), "got `{schema}`");
    let item_impl = derived_impl(&file);
    assert_eq!(helper_count(item_impl, "__refield_with_value_1"), 1);
}

#[test]
fn on_subset_merges_with_while_keeping_other_profile() {
    let file = derive_file(quote! {
        struct Sample {
            #[refield(on(fast, slow), name = "a")]
            #[refield(on(fast), with = |v: u8| v + 1)]
            value: u8,
        }
    });
    let schema = companion_macro(&file, "Sample").mac.tokens.to_string();
    assert!(schema.contains("[fast]"), "got `{schema}`");
    assert!(schema.contains("[slow]"), "got `{schema}`");
    assert!(schema.contains("a , false"), "got `{schema}`");
    assert!(schema.contains("_with_value_1"), "got `{schema}`");
    let item_impl = derived_impl(&file);
    assert_eq!(helper_count(item_impl, "__refield_with_value_1"), 1);
}

#[test]
fn field_accepts_profile_shorthands() {
    for attribute in [
        quote! { on(fast) },
        quote! { on(fast, thorough) },
        quote! { on(r#type) },
    ] {
        let input = quote! {
            struct Sample {
                #[refield(#attribute)]
                value: u8,
            }
        };
        derive(input).expect("every profile spelling should parse");
    }
}

#[test]
fn construct_rejects_unknown_profile() {
    assert_error(
        construct(construct_profile_input(
            quote! { fast, thorough },
            quote! { @unknown, ..raw },
        )),
        "unknown profile `unknown`",
    );
}

#[test]
fn construct_rejects_profile_after_fields() {
    assert_error(
        construct(construct_profile_input(
            quote! { fast },
            quote! { value: 1, @fast, ..raw },
        )),
        "`@profile` selector must be the first entry in the body",
    );
}

#[test]
fn construct_accepts_profile_as_first_body_entry() {
    for body in [
        quote! { @fast ..raw },
        quote! { @fast, ..raw },
        quote! { @fast value: 1, ..raw },
        quote! { @fast, value: 1, ..raw },
        quote! { @fast },
        quote! { @fast, },
    ] {
        construct(construct_profile_input(quote! { fast }, body))
            .expect("a leading profile selector should parse");
    }
}

#[test]
fn multi_segment_path_profile_works_end_to_end() {
    let file = derive_file(quote! {
        struct Sample {
            #[refield(on(mode::fast), name = "fast_v")]
            value: u8,
        }
    });
    let schema_macro = companion_macro(&file, "Sample");
    let schema_tokens = schema_macro.mac.tokens.to_string();
    assert!(
        schema_tokens.contains("mode :: fast"),
        "got `{schema_tokens}`"
    );

    let call_output = call(quote! { Sample { @mode::fast, ..raw } })
        .expect("call with multi-segment profile should succeed")
        .to_string();
    assert!(
        call_output.contains("@ mode :: fast"),
        "got `{call_output}`"
    );

    let construct_output = construct(quote! {
        (::refield),
        __refield_check_test,
        [
            {
                value,
                { value, false, (), () },
                [([mode::fast], { fast_v, false, (), () })],
                false,
            },
        ],
        (Sample),
        (Sample),
        { @mode::fast, ..raw }
    })
    .expect("construct with multi-segment profile should succeed")
    .to_string();
    assert!(
        construct_output.contains("(raw) . fast_v"),
        "got `{construct_output}`"
    );
}

#[test]
fn call_defers_bare_self_to_the_type_checker() {
    let output = call(quote! { Self { ..raw } })
        .expect("bare `Self` is deferred to the type checker")
        .to_string();
    assert!(
        output.contains("__refield_ctx__"),
        "expected the context macro, got `{output}`"
    );
    assert!(
        output.contains("@ self"),
        "expected the `@self` arm, got `{output}`"
    );
}

#[test]
fn call_rejects_bare_self_as_schema() {
    assert_error(
        call(quote! { Self as Self { ..raw } }),
        "cannot resolve struct schema from `Self` alone",
    );
}

#[test]
fn call_rejects_associated_types_without_schema() {
    assert_error(
        call(quote! { <T as Trait>::Assoc { ..raw } }),
        "associated types require an explicit schema",
    );
    assert_error(
        call(quote! { Type::<T>::Assoc { ..raw } }),
        "associated types require an explicit schema",
    );
}

#[test]
fn call_accepts_associated_types_with_schema() {
    for input in [
        quote! { <T as Trait>::Assoc as Schema { ..raw } },
        quote! { <Type<T> as Trait<U>>::Assoc as Schema { ..raw } },
        quote! { Type::<T>::Assoc as Schema { ..raw } },
    ] {
        let output = call(input).expect("an explicit schema permits associated types");
        let output = output.to_string();
        assert!(
            output.contains("Schema"),
            "expected `Schema`, got `{output}`"
        );
    }
}

// -----------------------------------------------------------------------
// REVIEW2 #6 & #7: one rule per profile attribute, one shared schema model
// -----------------------------------------------------------------------

#[test]
fn profile_rule_round_trips_through_the_schema_protocol() {
    use syn::parse::Parser as _;

    let file = derive_file(quote! {
        struct Sample {
            #[refield(on(fast, thorough), with = |v| v + 1)]
            value: u8,
        }
    });
    let schema = companion_macro(&file, "Sample").mac.tokens.to_string();
    // The schema carries one rule with two profile names, not one entry
    // per name.
    assert!(
        schema.contains("[fast , thorough]"),
        "one rule lists both names: `{schema}`"
    );

    // The same protocol parses both spellings back.
    let single =
        <syn::punctuated::Punctuated<field::ProfileRule, syn::Token![,]>>::parse_terminated
            .parse_str("([fast], { value, false, (), () })")
            .expect("`[fast]` parses");
    assert_eq!(single.len(), 1);

    let multiple =
        <syn::punctuated::Punctuated<field::ProfileRule, syn::Token![,]>>::parse_terminated
            .parse_str("([fast, thorough], { value, false, (), () })")
            .expect("`[fast, thorough]` parses");
    assert_eq!(multiple.len(), 1);
    let rendered = multiple[0].to_token_stream().to_string();
    assert!(rendered.contains("([fast , thorough] ,"), "`{rendered}`");
}

#[test]
fn one_profile_attribute_emits_one_helper() {
    let file = derive_file(quote! {
        struct Sample {
            #[refield(on(fast, thorough), with = |v| v + 1, default = 1)]
            value: u8,
        }
    });
    let item_impl = derived_impl(&file);
    let names: Vec<String> = impl_methods(item_impl)
        .iter()
        .map(|method| method.sig.ident.to_string())
        .collect();
    assert!(
        names.iter().any(|name| name.contains("_with_value_1")),
        "one `with` helper for the one profile rule: `{names:?}`"
    );
    assert!(
        names.iter().any(|name| name.contains("_default_value_1")),
        "one `default` helper for the one profile rule: `{names:?}`"
    );
    assert_eq!(
        names
            .iter()
            .filter(|name| name.contains("_with_value_"))
            .count(),
        1,
        "the default rule has no `with`, so only one helper exists: `{names:?}`"
    );
}

#[test]
fn duplicate_profiles_within_one_attribute_are_rejected() {
    let message = derive_field_error(quote! { on(fast, fast), with = |v: u8| v });
    assert!(
        message.contains("duplicate profile `fast`"),
        "got `{message}`"
    );
}

#[test]
fn schema_field_parses_back_into_the_shared_model() {
    let file = derive_file(quote! {
        struct Sample {
            #[refield(name = "raw", with = |v: u8| v + 1, default = 1)]
            value: u8,
            #[refield(on(fast))]
            other: u8,
        }
    });
    let schema = companion_macro(&file, "Sample").mac.tokens.to_string();
    // The schema is written by `FieldIr::to_tokens` itself; parsing the
    // rendered entries back proves both halves agree.
    let start = schema.find('[').expect("the schema has fields") + 1;
    // The list contains nested `[ ... ]` profile lists, so find the
    // matching bracket instead of the next one.
    let mut depth = 1_i32;
    let mut end = start;
    for (index, ch) in schema[start..].char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    end = start + index;
                    break;
                }
            }
            _ => {}
        }
    }
    let fields: TokenStream = schema[start..end]
        .parse()
        .expect("the rendered schema is a token stream");
    let parsed = syn::parse::Parser::parse2(
        |input: syn::parse::ParseStream<'_>| {
            let mut fields = Vec::new();
            while !input.is_empty() {
                fields.push(input.parse::<field::FieldIr>()?);
                let _ = input.parse::<syn::Token![,]>();
            }
            Ok(fields)
        },
        fields,
    )
    .expect("the rendered schema parses back");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].target.to_token_stream().to_string(), "value");
    assert!(!parsed[0].rule.skip);
    assert!(parsed[1].profile(&syn::parse_quote!(fast)).is_some());
    assert!(parsed[1].profile(&syn::parse_quote!(thorough)).is_none());
}

// -----------------------------------------------------------------------
// REVIEW2 #8: aliases skip the carrier, `impl` blocks keep it
// -----------------------------------------------------------------------

#[test]
fn type_aliases_are_expanded_without_the_carrier() {
    let output = proc(
        quote! { as = Target, crate = ::refield },
        &quote! {
            type Alias = <T as Trait>::Assoc;
        },
    )
    .expect("the alias expands directly")
    .to_string();
    assert!(
        !output.contains("__RefieldCarrier"),
        "an alias has no associated items to cfg-strip: `{output}`"
    );
    assert!(output.contains("@ __refield_alias"), "`{output}`");

    // The `impl` block still goes through the carrier.
    let carrier = proc(
        quote! { crate = ::refield },
        &quote! { impl Target { fn sample() {} } },
    )
    .expect("the impl block is wrapped")
    .to_string();
    assert!(carrier.contains("__RefieldCarrier"), "`{carrier}`");
}

// -----------------------------------------------------------------------
// REVIEW2 #9 & #10: one construct block, no `tuple` protocol field
// -----------------------------------------------------------------------

#[test]
fn construct_emits_exactly_one_marker_binding() {
    let output = construct(construct_input(quote! { ..raw }))
        .expect("a place source constructs")
        .to_string();
    assert_eq!(
        output.matches("let __refield_marker =").count(),
        1,
        "one marker binding per expansion: `{output}`"
    );
    assert_eq!(
        output.matches("PhantomData").count(),
        1,
        "one `PhantomData` construction per expansion: `{output}`"
    );

    // A value source binds its temporary in the same block.
    let output = construct(construct_input(quote! { ..make_raw() }))
        .expect("a value source constructs")
        .to_string();
    assert_eq!(output.matches("let __refield_marker =").count(), 1);
    assert!(
        output.contains("let __refield_source = make_raw ()"),
        "`{output}`"
    );
}

#[test]
fn construct_takes_its_braces_from_the_block_shim() {
    // The shim crate supplies Rust 2021 braces and allows the numbered
    // fields and the `..base` update that every expansion may contain.
    for body in [quote! { ..raw }, quote! { #[cfg(all())] value: 1 }] {
        let output = construct(construct_input(body))
            .expect("a construction expands")
            .to_string();
        assert!(
            output.starts_with(":: refield :: __private :: block ! {"),
            "`{output}`"
        );
    }
}

#[test]
fn construct_consumes_an_unused_place_once() {
    let input = quote! {
        (::refield),
        __refield_check_test,
        [
            {
                value,
                { value, true, (), (default) },
                [],
                false,
            },
        ],
        (Sample),
        (Sample),
        { ..raw }
    };
    let output = construct(input)
        .expect("a fully skipped schema constructs")
        .to_string();
    assert_eq!(output.matches("let _ = raw ;").count(), 1, "`{output}`");
    // The place is touched before the literal, which stays the tail.
    assert!(
        output.find("let _ = raw ;") < output.find("Sample {"),
        "`{output}`"
    );
}

#[test]
fn construct_rejects_tokens_after_rest() {
    assert_error(
        construct(construct_input(quote! { ..rest, })),
        "cannot have a comma or fields after `..`",
    );
    assert_error(
        construct(construct_input(quote! { ..rest, extra: 1 })),
        "cannot have a comma or fields after `..`",
    );
}

fn construct_pair_input(body: TokenStream) -> TokenStream {
    quote! {
        (::refield),
        __refield_check_test,
        [
            { value, { value, false, (), () }, [], false },
            { other, { other, false, (), () }, [], false },
        ],
        (Sample),
        (Sample),
        { #body }
    }
}

/// The positions of `needles` in the construction expanded from `body`.
fn expanded_position(body: TokenStream, needles: &[&str]) -> Vec<usize> {
    let output = construct(construct_pair_input(body))
        .expect("the construction expands")
        .to_string();
    needles
        .iter()
        .map(|needle| {
            output
                .find(needle)
                .unwrap_or_else(|| panic!("`{needle}` is missing from `{output}`"))
        })
        .collect()
}

#[test]
fn construct_orders_the_literal_by_the_written_source_position() {
    // A source written first is read before the written fields.
    let positions = expanded_position(
        quote! { raw.., other: 1 },
        &["value : (raw) . value", "other : 1"],
    );
    assert!(positions[0] < positions[1], "{positions:?}");

    // A place written last is read after them, like native update syntax.
    let positions = expanded_position(
        quote! { other: 1, ..raw },
        &["value : (raw) . value", "other : 1"],
    );
    assert!(positions[0] > positions[1], "{positions:?}");

    // A value written first is bound once before the literal.
    let positions = expanded_position(
        quote! { make_raw().., other: 1 },
        &[
            "let __refield_source = make_raw () ;",
            "value : (__refield_source) . value",
            "other : 1",
        ],
    );
    assert!(
        positions[0] < positions[1] && positions[1] < positions[2],
        "{positions:?}"
    );
}

#[test]
fn construct_accepts_every_source_position() {
    for body in [
        quote! { raw.. },
        quote! { raw.., },
        quote! { raw.., other: 1 },
        quote! { raw.., other: 1, },
        quote! { make_raw()?.., other: 1 },
        quote! { &wrapper as &Raw.., other: 1 },
        quote! { Raw { value: 1, other: 2 }.., other: 1 },
        quote! { other: 1, ..raw },
        quote! { other: 1, ..raw.inner },
        quote! { other: 1, ..*boxed },
        // Without written fields, a value written last has nothing to be
        // evaluated after, so it is read exactly like `make_raw()..`.
        quote! { ..make_raw() },
        quote! { ..Raw { value: 1, other: 2 } },
    ] {
        construct(construct_pair_input(body.clone()))
            .unwrap_or_else(|err| panic!("`{body}` should construct: {err}"));
    }
    for body in [
        quote! { @fast make_raw().., other: 1 },
        quote! { @fast, raw.. },
    ] {
        construct(construct_profile_input(quote! { fast }, body.clone()))
            .unwrap_or_else(|err| panic!("`{body}` should construct: {err}"));
    }
}

#[test]
fn construct_rejects_a_value_source_after_written_fields() {
    for body in [
        quote! { other: 1, ..make_raw() },
        quote! { other: 1, ..Raw { value: 1, other: 2 } },
        quote! { #[cfg(any())] other: 1, ..make_raw() },
    ] {
        assert_error(
            construct(construct_pair_input(body)),
            "a value source cannot follow written fields; write it first as `source..`",
        );
    }
}

#[test]
fn construct_rejects_misplaced_or_repeated_sources() {
    assert_error(
        construct(construct_pair_input(quote! { other: 1, raw.. })),
        "`source..` must be the first entry in the body",
    );
    assert_error(
        construct(construct_pair_input(quote! { raw.., ..raw })),
        "the source is already written first as `source..`",
    );
    assert_error(
        construct(construct_pair_input(quote! { raw.., other: 1, ..raw })),
        "the source is already written first as `source..`",
    );
}

#[test]
fn prepared_carriers_keep_the_source_position() {
    let output = construct(construct_pair_input(quote! {
        make_raw()..,
        #[cfg(all())]
        other: 1,
    }))
    .expect("an attributed construction expands")
    .to_string();
    assert!(
        output.contains("__construct (__refield_check_test__prepared_call , true ,"),
        "the carrier records the front position: `{output}`"
    );
    assert!(output.contains(".. make_raw ()"), "`{output}`");

    let carrier = parse_fixture(
        "#[derive(::refield::__private::Prepare)]\n#[refield(__construct(__cb, true, (::refield), __refield_check_test, [ { value, { value, false, (), () }, [], false }, { other, { other, false, (), () }, [], false } ], (Sample), (Sample), {}))]\n#[::refield::__private::discard]\nstruct __RefieldCarrier([(); { __RefieldCarrier { other: 2, ..make_raw() }; 0 }]);",
    );
    let rendered = prepare(&carrier)
        .expect("the carrier is prepared")
        .to_string();
    let source = rendered.find("let __refield_source = make_raw () ;");
    let auto = rendered.find("value : (__refield_source) . value");
    let written = rendered.find("other : 2");
    assert!(
        source.is_some() && source < auto && auto < written,
        "the rebuilt literal reads the source first: `{rendered}`"
    );
}

// -----------------------------------------------------------------------
// Definition-site helper methods (REVIEW_SUMMARY §2.2 #4, Appendix #4)
// -----------------------------------------------------------------------

#[test]
fn helper_is_shared_by_every_profile_of_one_rule() {
    // One attribute binds the same rule to two profiles, so both schema
    // entries reference one helper; it must be emitted exactly once.
    let file = derive_file(quote! {
        struct Sample {
            #[refield(on(fast, thorough), with = |v| v + 1)]
            value: u8,
        }
    });
    let item_impl = derived_impl(&file);

    assert_eq!(helper_count(item_impl, "__refield_with_value_1"), 1);
    let helper = helper_method(item_impl, "__refield_with_value_1");
    assert_eq!(helper.sig.inputs.len(), 2, "value plus `PhantomData<Self>`");

    // Both profile entries in one schema point at that same helper. The
    // companion macro repeats its schema in every arm, so restrict the
    // count to the `@__refield_call` arm.
    let schema = companion_macro(&file, "Sample").mac.tokens.to_string();
    let call_arm = schema
        .split("@ __refield_call")
        .nth(1)
        .and_then(|rest| rest.split("@ __refield_alias").next())
        .expect("the companion macro has a call arm");
    assert_eq!(
        call_arm.matches("_with_value_1").count(),
        1,
        "one helper, two profiles: `{call_arm}`"
    );
    assert!(
        call_arm.contains("[fast , thorough]"),
        "both profiles must be listed: `{call_arm}`"
    );
}

#[test]
fn separate_profile_rules_get_distinct_helpers() {
    let file = derive_file(quote! {
        struct Sample {
            #[refield(on(fast), with = |v: u8| v + 1)]
            #[refield(on(thorough), with = |v: u8| v + 2)]
            value: u8,
        }
    });
    let item_impl = derived_impl(&file);

    assert_eq!(helper_count(item_impl, "__refield_with_value_1"), 1);
    assert_eq!(helper_count(item_impl, "__refield_with_value_2"), 1);

    let fast = helper_method(item_impl, "__refield_with_value_1")
        .to_token_stream()
        .to_string();
    let thorough = helper_method(item_impl, "__refield_with_value_2")
        .to_token_stream()
        .to_string();
    assert!(fast.contains("v + 1"), "fast helper body: `{fast}`");
    assert!(
        thorough.contains("v + 2"),
        "thorough helper body: `{thorough}`"
    );
}

// -----------------------------------------------------------------------
// Helper visibility (REVIEW_SUMMARY Appendix #4)
// -----------------------------------------------------------------------

#[test]
fn helper_visibility_matches_the_field_visibility() {
    let file = derive_file(quote! {
        pub struct Sample {
            #[refield(with = |v| v, default = 1)]
            a: u8,
            #[refield(with = |v| v, default = 2)]
            pub(crate) b: u8,
            #[refield(with = |v| v, default = 3)]
            pub c: u8,
        }
    });
    let item_impl = derived_impl(&file);

    let vis_of = |name: &str| {
        helper_method(item_impl, name)
            .vis
            .to_token_stream()
            .to_string()
            .replace(' ', "")
    };
    assert_eq!(
        vis_of("__refield_with_a_0"),
        "",
        "private field, private fn"
    );
    assert_eq!(vis_of("__refield_default_a_0"), "");
    assert_eq!(vis_of("__refield_with_b_0"), "pub(crate)");
    assert_eq!(vis_of("__refield_default_b_0"), "pub(crate)");
    assert_eq!(vis_of("__refield_with_c_0"), "pub");
    assert_eq!(vis_of("__refield_default_c_0"), "pub");

    // The schema identity method is always callable from other crates.
    let check = check_method(item_impl);
    assert!(matches!(check.vis, syn::Visibility::Public(_)));
    assert!(
        check.sig.constness.is_some(),
        "`#check_ident` is `pub const fn`"
    );
}

#[test]
fn helper_visibility_covers_restricted_forms() {
    let file = derive_file(quote! {
        pub struct Sample {
            #[refield(with = |v| v)]
            pub(super) a: u8,
            #[refield(with = |v| v)]
            pub(in crate::nested) b: u8,
        }
    });
    let item_impl = derived_impl(&file);
    let vis_of = |name: &str| {
        helper_method(item_impl, name)
            .vis
            .to_token_stream()
            .to_string()
            .replace(' ', "")
    };
    assert_eq!(vis_of("__refield_with_a_0"), "pub(super)");
    assert_eq!(vis_of("__refield_with_b_0"), "pub(incrate::nested)");
}

#[test]
fn generated_helpers_allow_private_interface_lints() {
    // A field may mention a type that is only reachable inside the
    // defining module. The helper's visibility mirrors the field, so its
    // signature mentions that type too; the generated method therefore
    // carries `#[allow(private_interfaces, private_bounds)]` so the
    // expansion stays clean under a crate-level `deny`. The integration
    // test exercises the same shape. A helper's body is the user's `with`
    // or `default` expression, so it allows nothing else: the user's own
    // lints must still fire there.
    let file = derive_file(quote! {
        pub struct Sample {
            #[refield(with = |v| v, default = Default::default())]
            pub value: u8,
        }
    });
    let item_impl = derived_impl(&file);
    for method in impl_methods(item_impl) {
        let rendered = method.to_token_stream().to_string();
        assert!(
            rendered.contains("private_interfaces") && rendered.contains("private_bounds"),
            "every generated method must allow the private-interface lints: `{rendered}`"
        );
        let name = method.sig.ident.to_string();
        if name.contains("_with_") || name.contains("_default_") {
            assert!(
                !rendered.contains("clippy")
                    && !rendered.contains("unused_variables")
                    && !rendered.contains("non_snake_case")
                    && !rendered.contains("dead_code"),
                "a helper must not silence lints in the user's expression: `{rendered}`"
            );
        }
    }
}

// -----------------------------------------------------------------------
// Unique identifiers across definitions (REVIEW_SUMMARY Appendix #6)
// -----------------------------------------------------------------------

#[test]
fn companion_and_check_identifiers_are_unique_across_derives() {
    // Two identical derives at the same span share the same hash and
    // differ in `seq` (`0` vs `1`).
    let tokens = quote! {
        struct Duplicate { value: u8 }
    };
    let first = derive_file(tokens.clone());
    let second = derive_file(tokens);

    let first_check = check_method(derived_impl(&first)).sig.ident.to_string();
    let second_check = check_method(derived_impl(&second)).sig.ident.to_string();
    assert_ne!(first_check, second_check);

    let first_macro = companion_macro_ident(&first, "Duplicate");
    let second_macro = companion_macro_ident(&second, "Duplicate");
    assert_ne!(first_macro, second_macro);

    assert!(first_check.starts_with("__refield_check__Duplicate__IN__"));
    let suffix1 = first_check
        .strip_prefix("__refield_check__Duplicate__IN__")
        .expect("the schema ident embeds the struct name")
        .rsplit_once("__")
        .expect("the schema ident ends in `__<hash>_<seq>`")
        .1;
    let (hash1, seq1) = suffix1
        .rsplit_once('_')
        .expect("the schema ident ends in `<hash>_<seq>`");
    assert_eq!(hash1.len(), 16, "hash is 64-bit hex: `{hash1}`");
    assert!(hash1.chars().all(|c| c.is_ascii_hexdigit()));

    let suffix2 = second_check
        .strip_prefix("__refield_check__Duplicate__IN__")
        .expect("the schema ident embeds the struct name")
        .rsplit_once("__")
        .expect("the schema ident ends in `__<hash>_<seq>`")
        .1;
    let (hash2, seq2) = suffix2
        .rsplit_once('_')
        .expect("the schema ident ends in `<hash>_<seq>`");
    assert_eq!(
        hash2, hash1,
        "identical derives at the same span share hash"
    );
    assert_eq!(seq1, "0");
    assert_eq!(seq2, "1");

    // Distinct structs (or structs at different line/column spans via
    // `parse_fixture`) get distinct hashes and each starts with `seq == "0"`.
    let third = derive_file(parse_fixture("struct Duplicate { value: u8 }\n"));
    let fourth = derive_file(parse_fixture("\nstruct Duplicate { value: u8 }\n"));
    let third_check = check_method(derived_impl(&third)).sig.ident.to_string();
    let fourth_check = check_method(derived_impl(&fourth)).sig.ident.to_string();
    let (hash3, seq3) = third_check
        .strip_prefix("__refield_check__Duplicate__IN__")
        .unwrap()
        .rsplit_once("__")
        .unwrap()
        .1
        .rsplit_once('_')
        .unwrap();
    let (hash4, seq4) = fourth_check
        .strip_prefix("__refield_check__Duplicate__IN__")
        .unwrap()
        .rsplit_once("__")
        .unwrap()
        .1
        .rsplit_once('_')
        .unwrap();
    assert_ne!(hash3, hash4, "distinct spans yield distinct hashes");
    assert_eq!(seq3, "0");
    assert_eq!(seq4, "0");

    assert!(first_macro.starts_with("__refield__Duplicate__IN__"));
}

// -----------------------------------------------------------------------
// String-literal attribute parsing (REVIEW_SUMMARY §0.8 & §7)
// -----------------------------------------------------------------------

fn derive_field_error(attribute: TokenStream) -> String {
    error_message(derive(quote! {
        struct Sample {
            #[refield(#attribute)]
            value: u8,
        }
    }))
}

#[test]
fn default_preserves_string_literals_and_non_expr_keys_reject_strings() {
    let file = derive_file(quote! {
        struct Sample {
            #[refield(default = "hello")]
            greeting: &'static str,
        }
    });
    let item_impl = derived_impl(&file);
    let helper = helper_method(item_impl, "__refield_default_greeting_0");
    assert!(
        matches!(
            &helper.block.stmts[..],
            [syn::Stmt::Expr(
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(lit),
                    ..
                }),
                None,
            )] if lit.value() == "hello"
        ),
        "the default expression must remain the string literal \"hello\""
    );

    let file = derive_file(quote! {
        #[refield(crate = "::refield")]
        struct SampleWith {
            #[refield(with = "core::convert::identity")]
            value: u8,
        }
    });
    let item_impl = derived_impl(&file);
    assert_eq!(helper_count(item_impl, "__refield_with_value_0"), 1);

    let output = expand_refieldable(
        quote! { as = "Target" },
        quote! {
            type Alias = Target;
        },
    )
    .expect("`as = \"Target\"` parses via standard Path FromMeta")
    .to_string();
    assert!(output.contains("Target"));

    assert!(
        derive_field_error(quote! { bound = "T: Clone", default = 1 })
            .contains("expected `bound(...)`"),
        "`bound` rejects name-value syntax"
    );
}

// -----------------------------------------------------------------------
// Field attribute validation (REVIEW_SUMMARY Appendix #2)
// -----------------------------------------------------------------------

#[test]
fn with_closure_must_take_exactly_one_parameter() {
    for attribute in [quote! { with = || 1 }, quote! { with = |a, b| a }] {
        let message = derive_field_error(attribute);
        assert!(
            message.contains("`with` closure must take exactly one parameter"),
            "got `{message}`"
        );
    }
}

#[test]
fn duplicate_keys_within_one_attribute_are_rejected() {
    for (attribute, key) in [
        (quote! { name = "a", name = "b" }, "name"),
        (quote! { with = |v: u8| v, with = |v: u8| v }, "with"),
        (quote! { skip, skip }, "skip"),
        (quote! { default, default = 1 }, "default"),
        (quote! { const, const }, "const"),
        (quote! { on(fast), on(thorough) }, "on"),
    ] {
        let message = derive_field_error(attribute);
        assert!(
            message
                .to_lowercase()
                .contains(&format!("duplicate field `{key}`")),
            "expected a duplicate `{key}` diagnostic, got `{message}`"
        );
    }
}

#[test]
fn duplicate_keys_across_default_attributes_are_rejected() {
    // Attributes without `on` are merged into one rule; the merge sees the
    // duplicate exactly as if both keys had been written in one list.
    for (first, second, key) in [
        (quote! { name = "a" }, quote! { name = "b" }, "name"),
        (
            quote! { with = |v: u8| v },
            quote! { with = |v: u8| v },
            "with",
        ),
        (quote! { skip }, quote! { skip }, "skip"),
        (quote! { default }, quote! { default = 1 }, "default"),
        (quote! { const }, quote! { const }, "const"),
    ] {
        let message = error_message(derive(quote! {
            struct Sample {
                #[refield(#first)]
                #[refield(#second)]
                value: u8,
            }
        }));
        assert!(
            message
                .to_lowercase()
                .contains(&format!("duplicate field `{key}`")),
            "expected a duplicate `{key}` diagnostic, got `{message}`"
        );
    }
}

#[test]
fn unknown_field_attribute_is_rejected() {
    let message = derive_field_error(quote! { nonexistent });
    assert!(
        message.to_lowercase().contains("unknown field") && message.contains("nonexistent"),
        "got `{message}`"
    );
}

#[test]
fn bare_bound_is_rejected() {
    let message = derive_field_error(quote! { bound });
    assert!(message.contains("expected `bound(...)`"), "got `{message}`");
}

#[test]
fn every_bound_spelling_attaches_a_where_clause() {
    let cases = [
        (
            quote! { bound(T: Clone, U: Default), with = |v: T| v },
            vec!["T: Clone", "U: Default"],
        ),
        (
            quote! { bound(T: Clone), with = |v: T| v },
            vec!["T: Clone"],
        ),
        (
            quote! { bound(T: Clone), bound(U: Default), with = |v: T| v },
            vec!["T: Clone", "U: Default"],
        ),
    ];
    for (attribute, expected) in cases {
        let file = derive_file(quote! {
            struct Sample<T, U> {
                #[refield(#attribute)]
                value: T,
                #[allow(dead_code)]
                marker: U,
            }
        });
        let helper = helper_method(derived_impl(&file), "__refield_with_value_0");
        let where_clause = helper
            .sig
            .generics
            .where_clause
            .as_ref()
            .expect("`bound` must emit a where clause");
        let rendered = where_clause.to_token_stream().to_string();
        let normalized = rendered.replace(' ', "");
        for predicate in expected {
            let predicate = predicate.replace(' ', "");
            assert!(
                normalized.contains(&predicate),
                "expected `{predicate}` in `{rendered}`"
            );
        }
    }
}

// -----------------------------------------------------------------------
// `CallInput` validation (REVIEW_SUMMARY §0.8)
// -----------------------------------------------------------------------

#[test]
fn call_requires_a_brace_after_the_type_path() {
    assert_error(call(quote! { Target }), "expected `{` after type path");
    assert_error(
        call(quote! { Target as Schema }),
        "expected `{` after type path",
    );
}

#[test]
fn call_rejects_trailing_tokens_after_the_body() {
    assert_error(
        call(quote! { Target { ..raw } extra }),
        "unexpected tokens after struct initialization body",
    );
    assert_error(
        call(quote! { Target as Schema { ..raw } extra }),
        "unexpected tokens after struct initialization body",
    );
}

#[test]
fn call_rejects_associated_types_in_the_schema_position() {
    let message = error_message(call(quote! { Alias as Type::<T>::Assoc { ..raw } }));
    assert!(
        message.contains("schema path after `as` cannot be an associated type"),
        "got `{message}`"
    );
}

// -----------------------------------------------------------------------
// Struct-level `name`, `with`, `from = Type`, and `const`
// -----------------------------------------------------------------------

/// The field's rendered schema entry, from the `@__refield_call` arm.
fn schema_field_rule(file: &syn::File, struct_name: &str, target: &str) -> String {
    let schema = companion_macro(file, struct_name).mac.tokens.to_string();
    let call_arm = schema
        .split("@ __refield_call")
        .nth(1)
        .and_then(|rest| rest.split("@ __refield_alias").next())
        .expect("the companion macro has a call arm");
    let marker = format!("{{ {target} , ");
    let start = call_arm
        .find(&marker)
        .unwrap_or_else(|| panic!("expected a schema entry for `{target}`: `{call_arm}`"));
    let rest = &call_arm[start + marker.len()..];
    let mut depth = 0_i32;
    let mut end = 0;
    for (index, ch) in rest.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = index + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    rest[..end].to_string()
}

#[test]
fn struct_level_name_renames_every_automatic_source() {
    let file = derive_file(quote! {
        #[refield(name = "raw_{}")]
        struct Sample {
            port: u16,
            #[refield(name = "special")]
            name: String,
            #[refield(skip, default = 1)]
            skipped: u8,
            #[refield(name = "own")]
            explicit: u8,
        }
    });

    assert!(schema_field_rule(&file, "Sample", "port").contains("raw_port"));
    // The field-level `name` wins.
    assert!(schema_field_rule(&file, "Sample", "name").contains("special"));
    // `skip` never reads the source, so the struct-level pattern does not
    // apply to it.
    assert!(schema_field_rule(&file, "Sample", "skipped").contains("skipped"));
    assert!(schema_field_rule(&file, "Sample", "explicit").contains("own"));
}

#[test]
fn struct_level_with_generates_one_helper_per_automatic_field() {
    let file = derive_file(quote! {
        #[refield(with = |v| v + 1)]
        struct Sample {
            a: u8,
            b: u8,
            #[refield(with = |v| v + 2)]
            c: u8,
            #[refield(skip, default = 0)]
            d: u8,
        }
    });
    let item_impl = derived_impl(&file);
    assert_eq!(helper_count(item_impl, "__refield_with_a_0"), 1);
    assert_eq!(helper_count(item_impl, "__refield_with_b_0"), 1);
    assert_eq!(helper_count(item_impl, "__refield_with_c_0"), 1);
    // Skipped fields do not get a conversion helper.
    assert_eq!(helper_count(item_impl, "__refield_with_d_0"), 0);

    let a = helper_method(item_impl, "__refield_with_a_0")
        .to_token_stream()
        .to_string();
    let c = helper_method(item_impl, "__refield_with_c_0")
        .to_token_stream()
        .to_string();
    assert!(a.contains("v + 1"), "struct-level body: `{a}`");
    assert!(c.contains("v + 2"), "field-level override: `{c}`");
}

#[test]
fn struct_level_with_closure_type_uses_the_declared_input_type() {
    let file = derive_file(quote! {
        #[refield(with = |s: &'a str| parse(s))]
        struct Sample<'a> {
            label: String,
            marker: &'a str,
        }
    });
    let helper = helper_method(derived_impl(&file), "__refield_with_label_0");
    let rendered = helper.to_token_stream().to_string();
    assert!(
        rendered.contains(": & 'a str"),
        "the struct-level input type must be used: `{rendered}`"
    );
    let marker = helper_method(derived_impl(&file), "__refield_with_marker_0")
        .to_token_stream()
        .to_string();
    assert!(
        marker.contains(": & 'a str"),
        "every field shares the declared input type: `{marker}`"
    );
}

#[test]
fn struct_level_const_promotes_every_helper() {
    let file = derive_file(quote! {
        #[refield(const, with = double)]
        struct Sample {
            a: u8,
            #[refield(default = 1)]
            b: u8,
        }
    });
    let item_impl = derived_impl(&file);
    for method in impl_methods(item_impl) {
        let rendered = method.to_token_stream().to_string();
        assert!(
            rendered.contains("const fn"),
            "struct-level `const` must promote every helper: `{rendered}`"
        );
    }
}

#[test]
fn struct_level_with_applies_to_profile_rules_too() {
    // A profile rule only overrides `name`; the struct-level conversion is
    // still inherited, and each rule gets its own helper method.
    let file = derive_file(quote! {
        #[refield(with = |v| v + 1)]
        struct Sample {
            #[refield(on(fast), name = "fast_value")]
            value: u8,
        }
    });
    let item_impl = derived_impl(&file);
    assert_eq!(helper_count(item_impl, "__refield_with_value_0"), 1);
    assert_eq!(helper_count(item_impl, "__refield_with_value_1"), 1);
    let schema = companion_macro(&file, "Sample").mac.tokens.to_string();
    assert!(
        schema.contains("fast_value"),
        "the profile source comes from the profile rule: `{schema}`"
    );
}

#[test]
fn struct_level_name_is_validated_on_unit_structs() {
    // There is no field to format, but a pattern that can never name a
    // field is still a user error.
    assert_error(
        derive(quote! {
            #[refield(name = "not an ident")]
            struct Sample;
        }),
        "invalid source field name `not an ident`",
    );
    derive(quote! {
        #[refield(name = "raw_{}")]
        struct Sample;
    })
    .expect("a well-formed pattern is accepted on a unit struct");
}

#[test]
fn struct_level_keys_are_rejected_on_type_aliases() {
    for (args, expected) in [
        (
            quote! { name = "raw_{}" },
            "Unknown field: `name`. Available values: `as`, `crate`",
        ),
        (
            quote! { with = parse },
            "Unknown field: `with`. Available values: `as`, `crate`",
        ),
        (
            quote! { const },
            "Unknown field: `const`. Available values: `as`, `crate`",
        ),
    ] {
        assert_error(
            expand_refieldable(args, quote! { type Alias = Target; }),
            expected,
        );
    }
    assert_error(
        expand_refieldable(
            TokenStream::new(),
            quote! {
                #[refield(as = Target)]
                type Alias = Target;
            },
        ),
        "`refield` attributes are not supported on type aliases; pass arguments to `#[refieldable(...)]`",
    );
}

#[test]
fn struct_level_with_is_validated_like_field_with() {
    assert_error(
        derive(quote! {
            #[refield(with = |a, b| a)]
            struct Sample { value: u8 }
        }),
        "`with` closure must take exactly one parameter",
    );
}

#[test]
fn struct_level_duplicate_and_unknown_keys_are_rejected() {
    for (attribute, expected) in [
        (quote! { name = "a", name = "b" }, "Duplicate field `name`"),
        (quote! { with = f, with = g }, "Duplicate field `with`"),
        (quote! { const, const }, "Duplicate field `const`"),
        (quote! { on(fast) }, "Unknown field: `on`"),
        (quote! { skip }, "Unknown field: `skip`"),
        (quote! { default }, "Unknown field: `default`"),
        (quote! { bound(T: Clone) }, "Unknown field: `bound`"),
        (quote! { nonexistent }, "Unknown field: `nonexistent`"),
    ] {
        let message = error_message(derive(quote! {
            #[refield(#attribute)]
            struct Sample { value: u8 }
        }));
        assert!(
            message.contains(expected),
            "expected `{expected}`, got `{message}`"
        );
    }
}

// -----------------------------------------------------------------------
// Diagnostic count and span attribution
// -----------------------------------------------------------------------

/// Parse a fixture exactly the way `rustc` hands it to the macro.
///
/// `proc-macro2` records a span only for tokens that were parsed from a
/// string; tokens built with `quote!` carry a call-site span that has no
/// line or column. Every span test therefore starts from source text, and
/// `///` doc comments become `#[doc = "..."]` attributes exactly as the
/// compiler does it.
fn parse_fixture(source: &str) -> TokenStream {
    TokenStream::from_str(source).unwrap_or_else(|error| panic!("fixture should tokenize: {error}"))
}

/// One `(line, column)` position, zero-based as `span-locations` reports it.
type Position = (usize, usize);

/// The `(start, end)` range of a token stream's first and last token.
///
/// `syn::Error::to_compile_error` renders each diagnostic as
/// `::core::compile_error!{...}` whose punctuation and group carry the
/// error's start span and whose closing group carries its end span, so the
/// first and last tokens recover both halves of the range.
fn token_span(tokens: &TokenStream) -> (Position, Position) {
    let all: Vec<_> = tokens.clone().into_iter().collect();
    let first = all.first().expect("a compile_error stream is never empty");
    let last = all.last().expect("a compile_error stream is never empty");
    let start = first.span().start();
    let end = last.span().end();
    ((start.line, start.column), (end.line, end.column))
}

/// Number of diagnostics packed into one `syn::Error`.
fn error_count(error: &syn::Error) -> usize {
    error.clone().into_iter().count()
}

/// Flatten an error (including combined errors) into one message per
/// diagnostic.
fn error_messages(error: &syn::Error) -> Vec<String> {
    error
        .clone()
        .into_iter()
        .map(|error| error.to_string())
        .collect()
}

use std::str::FromStr;

#[test]
fn diagnostic_spans_point_at_the_offending_tokens() {
    // Every case is `(fixture, expected message, expected (start, end))`.
    // Columns are zero-based and the end is exclusive, exactly as
    // `proc-macro2`'s `span-locations` feature reports them.
    let struct_cases: &[(&str, &str, (Position, Position))] = &[
        (
            "#[refield(unknown)]\nstruct Sample { value: u8 }\n",
            "Unknown field: `unknown`. Available values: `const`, `crate`, `name`, `with`",
            ((1, 10), (1, 17)),
        ),
        // Struct-level keys that only exist per field point at the key.
        (
            "#[refield(skip)]\nstruct Sample { value: u8 }\n",
            "Unknown field: `skip`. Available values: `const`, `crate`, `name`, `with`",
            ((1, 10), (1, 14)),
        ),
        (
            "#[refield(bound(T: Clone))]\nstruct Sample { value: u8 }\n",
            "Unknown field: `bound`. Available values: `const`, `crate`, `name`, `with`",
            ((1, 10), (1, 25)),
        ),
        // A duplicate key points at the second occurrence, on line 2.
        (
            "#[refield(const)]\n#[refield(const)]\nstruct Sample { value: u8 }\n",
            "Duplicate field `const`",
            ((2, 10), (2, 15)),
        ),
        // The `with` closure span covers the whole closure expression.
        (
            "#[refield(with = |a, b| a)]\nstruct Sample { value: u8 }\n",
            "`with` closure must take exactly one parameter",
            ((1, 18), (1, 22)),
        ),
    ];
    for (source, expected, span) in struct_cases {
        let error = derive(parse_fixture(source)).unwrap_err();
        assert_eq!(error_count(&error), 1, "one diagnostic for `{source}`");
        assert_eq!(error.to_string(), *expected);
        assert_eq!(token_span(&error.to_compile_error()), *span, "`{source}`");
    }

    let field_cases: &[(&str, &str, (Position, Position))] = &[
        // The error points at the key inside the field attribute.
        (
            "struct Sample {\n    #[refield(nonexistent)]\n    value: u8,\n}\n",
            "Unknown field: `nonexistent`. Available values: `bound`, `const`, `default`, `name`, `skip`, `with`",
            ((2, 14), (2, 25)),
        ),
        // `skip` combined with `with` points at `skip`.
        (
            "struct Sample {\n    #[refield(skip, with = |x| x)]\n    value: u8,\n}\n",
            "`skip` can only be combined with `default`",
            ((2, 14), (2, 18)),
        ),
        // A profile name that is not a path points at the
        // literal that failed to parse.
        (
            "struct Sample {\n    #[refield(on(\"not an ident\"))]\n    value: u8,\n}\n",
            "Unexpected type `non-word`",
            ((2, 17), (2, 31)),
        ),
        // `bound` without parentheses points at the key.
        (
            "struct Sample {\n    #[refield(bound)]\n    value: u8,\n}\n",
            "expected `bound(...)`",
            ((2, 14), (2, 19)),
        ),
        // An invalid `name` pattern points at the literal, not the field.
        (
            "struct Sample {\n    #[refield(name = \"not an ident\")]\n    value: u8,\n}\n",
            "invalid source field name `not an ident`; expected a field identifier or tuple index",
            ((2, 21), (2, 35)),
        ),
    ];
    for (source, expected, span) in field_cases {
        let error = derive(parse_fixture(source)).unwrap_err();
        assert_eq!(error_count(&error), 1, "one diagnostic for `{source}`");
        assert_eq!(error.to_string(), *expected, "`{source}`");
        assert_eq!(token_span(&error.to_compile_error()), *span, "`{source}`");
    }
}

#[test]
fn item_level_diagnostic_spans_skip_doc_comments() {
    // A leading doc comment is an attribute too. The diagnostic must still
    // point at `fn` / `enum` / `union` on the following line, which is what
    // `item_without_attrs` exists for.
    let error = proc(
        TokenStream::new(),
        &parse_fixture("/// Docs for a function.\nfn sample() {}\n"),
    )
    .unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert_eq!(
        error.to_string(),
        "`refieldable` can only be applied to structs, type aliases, or `impl` blocks"
    );
    assert_eq!(token_span(&error.to_compile_error()), ((2, 0), (2, 14)));

    let error = derive(parse_fixture("/// Docs for an enum.\nenum Sample { A }\n")).unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert_eq!(
        error.to_string(),
        "`Refield` can only be derived for structs"
    );
    assert_eq!(token_span(&error.to_compile_error()), ((2, 0), (2, 4)));

    let error = derive(parse_fixture(
        "/// Docs for a union.\nunion Sample { a: u8 }\n",
    ))
    .unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert_eq!(
        token_span(&error.to_compile_error()),
        ((2, 0), (2, 5)),
        "the `union` keyword is highlighted, not the doc comment"
    );
}

#[test]
fn type_alias_and_impl_diagnostic_spans() {
    let error = proc(
        TokenStream::new(),
        &parse_fixture("#[refield(name = \"raw_{}\")]\ntype Alias = Target;\n"),
    )
    .unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert_eq!(
        error.to_string(),
        "`refield` attributes are not supported on type aliases; pass arguments to `#[refieldable(...)]`"
    );
    assert_eq!(token_span(&error.to_compile_error()), ((1, 0), (1, 27)));

    let error = proc(
        TokenStream::new(),
        &parse_fixture("#[refield(with = parse)]\ntype Alias = Target;\n"),
    )
    .unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert_eq!(
        error.to_string(),
        "`refield` attributes are not supported on type aliases; pass arguments to `#[refieldable(...)]`"
    );
    assert_eq!(token_span(&error.to_compile_error()), ((1, 0), (1, 24)));

    // The offending attribute inside the `impl` block is highlighted, not
    // the whole block.
    let error = expand_refieldable(
        parse_fixture("crate = ::refield"),
        parse_fixture("impl Target {\n    #[refield(skip)]\n    fn sample() {}\n}\n"),
    )
    .unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert_eq!(
        error.to_string(),
        "`refield` attributes are not supported on items inside an `impl` block"
    );
    assert_eq!(token_span(&error.to_compile_error()), ((2, 4), (2, 20)));
}

#[test]
fn type_alias_as_path_is_validated_at_the_definition_site() {
    for (args, expected) in [
        (
            "as = Self",
            "cannot resolve struct schema from `Self` alone; use a concrete schema path after `as`",
        ),
        (
            "as = Type::<T>::Assoc",
            "schema path after `as` cannot be an associated type",
        ),
    ] {
        let error = proc(
            parse_fixture(args),
            &parse_fixture("type Alias = <T as Trait>::Assoc;\n"),
        )
        .unwrap_err();
        assert_eq!(error_count(&error), 1);
        assert_eq!(error.to_string(), expected);
    }

    // A generic schema path keeps its arguments for the literal while the
    // forwarded macro path has them stripped.
    let output = proc(
        parse_fixture("as = Target::<u32>"),
        &parse_fixture("type Alias = Target<u32>;\n"),
    )
    .expect("`as = Target::<u32>` is a valid schema path")
    .to_string();
    assert!(
        output.contains("type Alias = Target < u32 > ;"),
        "the alias keeps its own generic arguments: `{output}`"
    );
    assert!(
        output.contains("Target ! { @ __refield_alias"),
        "the companion macro path drops the generic arguments: `{output}`"
    );
}

#[test]
fn every_struct_and_field_error_is_reported_in_one_pass() {
    // The struct attribute fails *and* two fields fail. `derive` combines
    // the struct-level diagnostic with the accumulated field diagnostics,
    // producing exactly three errors in source order, each with its own
    // span and no duplicates.
    let source = "#[refield(unknown_struct)]\nstruct Sample {\n    #[refield(unknown_field)]\n    a: u8,\n    #[refield(skip, with = |x| x)]\n    b: u8,\n}\n";
    let error = derive(parse_fixture(source)).unwrap_err();
    assert_eq!(error_count(&error), 3);

    let messages = error_messages(&error);
    assert_eq!(
        messages,
        [
            "Unknown field: `unknown_struct`. Available values: `const`, `crate`, `name`, `with`",
            "Unknown field: `unknown_field`. Available values: `bound`, `const`, `default`, `name`, `skip`, `with`",
            "`skip` can only be combined with `default`",
        ]
    );

    let spans: Vec<_> = error
        .clone()
        .into_iter()
        .map(|error| token_span(&error.to_compile_error()))
        .collect();
    assert_eq!(
        spans,
        [((1, 10), (1, 24)), ((3, 14), (3, 27)), ((5, 14), (5, 18)),],
        "each diagnostic keeps the span of its own tokens"
    );
}

#[test]
fn multiple_field_errors_accumulate_without_duplicates() {
    // Two bad fields and one bad profile rule: two errors, in field
    // order, and no spurious duplicate for the default rule.
    let source = "struct Sample {\n    #[refield(on(fast), unknown_a)]\n    a: u8,\n    #[refield(untyped)]\n    b: u8,\n}\n";
    let error = derive(parse_fixture(source)).unwrap_err();
    assert_eq!(error_count(&error), 2);
    let messages = error_messages(&error);
    assert_eq!(
        messages,
        [
            "Unknown field: `unknown_a`. Available values: `bound`, `const`, `default`, `name`, `skip`, `with`",
            "Unknown field: `untyped`. Available values: `bound`, `const`, `default`, `name`, `skip`, `with`",
        ]
    );
    let spans: Vec<_> = error
        .clone()
        .into_iter()
        .map(|error| token_span(&error.to_compile_error()))
        .collect();
    // The profile rule reports the key after `on(fast),`; the default
    // rule reports its own key on the next field.
    assert_eq!(spans, [((2, 24), (2, 33)), ((4, 14), (4, 21))]);
}

#[test]
fn impl_attributes_report_every_offending_site() {
    // One error on the `impl` block itself and one on an associated item:
    // both are combined rather than the second being lost.
    let source = "#[refield(name = \"raw_{}\")]\nimpl Target {\n    #[refield(skip)]\n    fn sample() {}\n}\n";
    let error = expand_refieldable(TokenStream::new(), parse_fixture(source)).unwrap_err();
    assert_eq!(error_count(&error), 2);
    assert_eq!(
        error_messages(&error),
        [
            "`refield` attributes are not supported on `impl` blocks; pass arguments to `#[refieldable(...)]`",
            "`refield` attributes are not supported on items inside an `impl` block",
        ]
    );
    let spans: Vec<_> = error
        .clone()
        .into_iter()
        .map(|error| token_span(&error.to_compile_error()))
        .collect();
    assert_eq!(spans, [((1, 0), (1, 27)), ((3, 4), (3, 20))]);
}

#[test]
fn unparsable_attributes_do_not_consume_following_attributes() {
    // A syntax error inside one attribute leaves the next attribute on the
    // following line untouched: the parser reports one error for the broken
    // expression, not at line 2.
    let error = derive(parse_fixture(
        "#[refield(with = |x| x +)]\n#[refield(const)]\nstruct Sample { value: u8 }\n",
    ))
    .unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert_eq!(
        error.to_string(),
        "unexpected end of input, expected an expression"
    );
    assert_eq!(token_span(&error.to_compile_error()), ((1, 0), (1, 0)));

    let error = derive(parse_fixture(
            "struct Sample {\n    #[refield(with = |x| x +)]\n    #[refield(const)]\n    value: u8,\n}\n",
        ))
        .unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert_eq!(token_span(&error.to_compile_error()), ((1, 0), (1, 0)));
}

#[test]
fn call_and_construct_diagnostic_spans() {
    let error = call(parse_fixture("Target")).unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert_eq!(
        error.to_string(),
        "unexpected end of input, expected `{` after type path"
    );
    assert_eq!(token_span(&error.to_compile_error()), ((1, 0), (1, 0)));

    let error = call(parse_fixture("Target { ..raw } extra")).unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert_eq!(
        error.to_string(),
        "unexpected tokens after struct initialization body"
    );
    assert_eq!(token_span(&error.to_compile_error()), ((1, 17), (1, 22)));

    let error = call(parse_fixture("<T as Trait>::Assoc { ..raw }")).unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert!(
        error
            .to_string()
            .starts_with("associated types require an explicit schema"),
    );
    assert_eq!(
        token_span(&error.to_compile_error()),
        ((1, 0), (1, 19)),
        "the whole constructor path is highlighted"
    );

    let error = call(parse_fixture("Self as Self { ..raw }")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "cannot resolve struct schema from `Self` alone; use a concrete schema path after `as`"
    );
    assert_eq!(token_span(&error.to_compile_error()), ((1, 8), (1, 12)));

    let error = call(parse_fixture("Alias as Type::<T>::Assoc { ..raw }")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "schema path after `as` cannot be an associated type"
    );
    assert_eq!(token_span(&error.to_compile_error()), ((1, 9), (1, 18)));
}

#[test]
fn construct_diagnostic_spans() {
    let unknown_profile =
        "(::refield),\n__refield_check_test,\n[],\n(Sample),\n(Sample),\n{ @unknown, ..raw }\n";
    let error = construct(parse_fixture(unknown_profile)).unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert_eq!(error.to_string(), "unknown profile `unknown`");
    assert_eq!(token_span(&error.to_compile_error()), ((6, 3), (6, 10)));

    let profile_after_fields = "(::refield),\n__refield_check_test,\n[\n    {\n        value,\n        { value, false, (), () },\n        [([fast], { value, false, (), () })],\n        false,\n    },\n],\n(Sample),\n(Sample),\n{ value: 1, @fast, ..raw }\n";
    let error = construct(parse_fixture(profile_after_fields)).unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert_eq!(
        error.to_string(),
        "`@profile` selector must be the first entry in the body"
    );
    assert_eq!(
        token_span(&error.to_compile_error()),
        ((13, 12), (13, 13)),
        "the `@` token is highlighted"
    );

    let rest_trailing =
        "(::refield),\n__refield_check_test,\n[],\n(Sample),\n(Sample),\n{ ..rest, }\n";
    let error = construct(parse_fixture(rest_trailing)).unwrap_err();
    assert_eq!(error_count(&error), 1);
    assert_eq!(
        error.to_string(),
        "cannot have a comma or fields after `..`"
    );
    assert_eq!(token_span(&error.to_compile_error()), ((6, 8), (6, 9)));
}

#[test]
fn strip_refield_attrs_clears_nested_items() {
    // Struct, field, enum variant, variant field, and `impl` item
    // attributes are all removed; unrelated attributes survive.
    let stripped = strip(&parse_fixture(
            "#[refield(unknown)]\nstruct Sample {\n    #[refield(from)]\n    a: u8,\n    #[derive(Clone)]\n    b: u8,\n}\n",
        ))
        .to_string();
    assert!(
        !stripped.contains("refield"),
        "every struct and field attribute is stripped: `{stripped}`"
    );
    assert!(stripped.contains("derive"), "`{stripped}`");
    assert!(stripped.contains("Clone"), "`{stripped}`");

    let stripped = strip(&parse_fixture(
            "#[refield(unknown)]\nenum Sample {\n    #[refield(as = X)]\n    A(#[refield(skip)] u8),\n    B,\n}\n",
        ))
        .to_string();
    assert!(
        !stripped.contains("refield"),
        "enum, variant, and variant-field attributes are stripped: `{stripped}`"
    );

    let stripped = strip(&parse_fixture(
            "#[refield(unknown)]\nimpl Target {\n    #[refield(skip)]\n    fn sample() {}\n    #[refield(x)]\n    type Assoc = u8;\n}\n",
        ))
        .to_string();
    assert!(
        !stripped.contains("refield"),
        "attributes on `impl` items are stripped: `{stripped}`"
    );
    assert!(stripped.contains("fn sample"), "`{stripped}`");
    assert!(stripped.contains("type Assoc"), "`{stripped}`");
}

#[test]
fn strip_prepared_attrs_recovers_the_original_item() {
    // On the error path, `prepare` emits the cfg-stripped item without its
    // carrier and without any `#[refield(...)]` attribute, so the compiler
    // does not report a secondary "cannot find attribute" diagnostic.
    let carrier = proc(
        parse_fixture("crate = ::refield"),
        &parse_fixture("impl Target { fn sample() {} }\n"),
    )
    .expect("`proc` produces the carrier");
    let stripped = recover(&carrier).to_string();
    assert!(!stripped.contains("__RefieldCarrier"), "`{stripped}`");
    assert!(!stripped.contains("refield"), "`{stripped}`");
    assert!(stripped.contains("impl Target"), "`{stripped}`");

    // A malformed carrier contributes nothing rather than panicking.
    assert_eq!(
        recover(&parse_fixture("struct NotACarrier;")).to_string(),
        ""
    );
}

#[test]
fn prepared_carriers_rebuild_the_construction_from_the_stripped_fields() {
    let carrier = parse_fixture(
        "#[derive(::refield::__private::Prepare)]\n#[refield(__construct(__cb, false, (::refield), __refield_check_test, [ { value, { value, false, (), () }, [], false } ], (Sample), (Sample), {}))]\n#[::refield::__private::discard]\nstruct __RefieldCarrier([(); { __RefieldCarrier { other: 2 }; 0 }]);",
    );
    let output = prepare(&carrier).expect("the carrier is prepared");
    let rendered = output.to_token_stream().to_string();
    assert!(
        rendered.contains("macro_rules ! __cb"),
        "the callback is defined locally: `{rendered}`"
    );
    assert!(
        rendered.contains("Sample { other : 2 ,"),
        "the surviving field is written verbatim into the literal: `{rendered}`"
    );
    assert!(
        rendered.contains("Sample :: __refield_check_test"),
        "the requested type is carried through: `{rendered}`"
    );

    // The cleanup path restores only the callback, never the placeholder
    // expression, so the caller sees one error instead of a missing-macro
    // cascade.
    let callback = recover(&carrier).to_string();
    assert!(!callback.contains("__RefieldCarrier"), "`{callback}`");
    assert!(callback.contains("macro_rules ! __cb"), "`{callback}`");
    assert!(
        callback.contains("($ __refield_dollar : tt) =>"),
        "the error callback accepts the `$` argument: `{callback}`"
    );
    assert!(callback.contains("compile_error"), "`{callback}`");
}

#[test]
fn prepared_callbacks_escape_user_dollars_through_a_token_argument() {
    // A user `macro_rules!` in a kept field expression keeps its own
    // `$` tokens: each one is rewritten to the callback's `$` metavariable,
    // which the caller restores by passing a single `$`.
    let carrier = parse_fixture(
        "#[derive(::refield::__private::Prepare)]\n#[refield(__construct(__cb, false, (::refield), __refield_check_test, [ { value, { value, false, (), () }, [], false } ], (Sample), (Sample), {}))]\n#[::refield::__private::discard]\nstruct __RefieldCarrier([(); { __RefieldCarrier { other: { macro_rules! pair { ($($a:expr),*) => { 0 $(+ $a)* }; } pair!(1, 2) } }; 0 }]);",
    );
    let rendered = prepare(&carrier)
        .expect("the carrier is prepared")
        .to_string();
    assert!(rendered.contains("macro_rules ! __cb"), "`{rendered}`");
    assert!(
        rendered.contains("($ __refield_dollar : tt) =>"),
        "the callback matcher takes the `$` token: `{rendered}`"
    );
    assert!(
        rendered.contains(
            "macro_rules ! pair { ($ __refield_dollar ($ __refield_dollar a : expr) ,*) =>"
        ),
        "the user's `macro_rules!` is escaped token by token: `{rendered}`"
    );
    assert!(
        rendered.contains("0 $ __refield_dollar (+ $ __refield_dollar a) *"),
        "the user's repetition is escaped too: `{rendered}`"
    );
    assert!(
        !rendered.contains("$ ("),
        "no bare user dollar survives: `{rendered}`"
    );
}

#[test]
fn malformed_carriers_are_reported_by_prepare_but_tolerated_on_cleanup() {
    // The normal path only parses the carrier once and reports a carrier
    // that does not wrap an `impl` block instead of expanding to nothing.
    let error = prepare(&parse_fixture("struct NotACarrier;")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "internal `refieldable` carrier does not contain an item"
    );

    let wrong_item = quote! {
        #[derive(::refield::__private::Prepare)]
        #[refield(crate = ::refield)]
        struct __RefieldCarrier([(); {
            struct NotAnImpl;
            0
        }]);
    };
    let error = prepare(&wrong_item).unwrap_err();
    assert_eq!(
        error.to_string(),
        "internal `refieldable` carrier does not contain an `impl` block"
    );

    // The cleanup path stays tolerant so the original diagnostic is not
    // followed by secondary errors; a carrier wrapping another item is
    // still unwrapped and emitted without its attributes.
    let stripped = recover(&wrong_item).to_string();
    assert!(!stripped.contains("__RefieldCarrier"), "`{stripped}`");
    assert!(stripped.contains("struct NotAnImpl"), "`{stripped}`");
}

#[test]
fn proc_forwards_every_struct_level_argument() {
    let carrier = proc(
        parse_fixture("crate = ::refield,\nname = \"raw_{}\",\nwith = |v| v as u32,\nconst"),
        &parse_fixture("struct Sample { value: u8 }\n"),
    )
    .expect("`proc` forwards the arguments")
    .to_string();
    assert!(carrier.contains("name = \"raw_{}\""), "`{carrier}`");
    assert!(carrier.contains("with = | v | v as u32"), "`{carrier}`");
    assert!(carrier.contains("const"), "`{carrier}`");
    // The forwarded attributes keep the order they were declared in.
    let name_pos = carrier.find("name = ").expect("name");
    let with_pos = carrier.find("with = ").expect("with");
    let const_pos = carrier.rfind("const").expect("const");
    assert!(name_pos < with_pos && with_pos < const_pos);
}

// -----------------------------------------------------------------------
// REVIEW2 #1: attributes are classified by syntax, not by token scanning
// -----------------------------------------------------------------------

#[test]
fn with_expression_mentioning_on_is_not_a_profile_rule() {
    // `on` is a variant name inside the generic arguments of `bump`, so it
    // belongs to the `with` expression. A raw-token scan mistook it for a
    // top-level `on` key and dropped the whole rule.
    // The expansion is not type-checked here, so `bump` and `On` only have
    // to parse the way a generic call would.
    let file = derive_file(quote! {
        struct Sample {
            #[refield(with = bump::<(), On>)]
            value: u8,
        }
    });
    let item_impl = derived_impl(&file);
    assert_eq!(
        helper_count(item_impl, "__refield_with_value_0"),
        1,
        "the conversion must stay on the default rule"
    );
    let schema = companion_macro(&file, "Sample").mac.tokens.to_string();
    assert!(
        !schema.contains("(["),
        "no profile rule may be created: `{schema}`"
    );
}

#[test]
fn lowercase_on_type_argument_is_not_a_profile_rule() {
    // The exact spelling from the review: the bare type name `on` inside
    // the turbofish. It is parsed as part of the expression, so the whole
    // attribute is a plain default rule.
    let file = derive_file(quote! {
        struct Sample {
            #[refield(with = bump::<(), on>, default = 1)]
            value: u8,
        }
    });
    let item_impl = derived_impl(&file);
    assert_eq!(helper_count(item_impl, "__refield_with_value_0"), 1);
    assert_eq!(helper_count(item_impl, "__refield_default_value_0"), 1);
    let schema = companion_macro(&file, "Sample").mac.tokens.to_string();
    assert!(
        !schema.contains("(["),
        "no profile rule may be created: `{schema}`"
    );
}

// -----------------------------------------------------------------------
// `with`: pseudo-closure shorthand and function paths
// -----------------------------------------------------------------------

fn parse_function_arg(source: &str) -> FunctionArg {
    syn::parse_str::<FunctionArg>(source)
        .unwrap_or_else(|error| panic!("`{source}` should parse: {error}"))
}

#[test]
fn function_arg_parses_pseudo_closures() {
    let typed = parse_function_arg("|v: u8| v + 1");
    let FunctionArg::PseudoClosure(closure) = typed else {
        panic!("a leading `|` selects a pseudo-closure");
    };
    assert_eq!(closure.inputs.len(), 1);
    assert!(matches!(closure.output, syn::ReturnType::Default));

    let untyped = parse_function_arg("|v| v + 1");
    assert!(matches!(untyped, FunctionArg::PseudoClosure(_)));

    let infer = parse_function_arg("|v: _| v + 1");
    assert!(matches!(infer, FunctionArg::PseudoClosure(_)));

    let returned = parse_function_arg("|v: u8| -> u16 { u16::from(v) }");
    let FunctionArg::PseudoClosure(returned) = returned else {
        panic!("a return type belongs to the pseudo-closure");
    };
    assert!(matches!(returned.output, syn::ReturnType::Type(..)));
}

#[test]
fn function_arg_parses_function_paths() {
    for source in [
        "negate",
        "core::convert::identity",
        "crate::module::convert",
        "bump::<(), on>",
        "<Target as Trait>::convert",
        "<Target as Trait>::convert::<u8>",
    ] {
        let parsed = parse_function_arg(source);
        assert!(
            matches!(parsed, FunctionArg::Function(_)),
            "`{source}` is a function path"
        );
        assert_eq!(
            parsed.to_token_stream().to_string().replace(' ', ""),
            source.replace(' ', ""),
            "the written path keeps its shape"
        );
    }

    let from_string = FunctionArg::from_meta(&syn::parse_quote!(with = "core::convert::identity"))
        .expect("the historic string spelling stays supported");
    assert!(matches!(from_string, FunctionArg::Function(_)));
    assert!(
        from_string
            .to_token_stream()
            .to_string()
            .contains("identity"),
        "the string contents become the path"
    );

    let bad_string = FunctionArg::from_meta(&syn::parse_quote!(with = "|a, b| a"));
    assert!(
        bad_string.is_err(),
        "a string must satisfy the same grammar as the bare value"
    );
}

#[test]
fn function_arg_rejects_every_other_shape() {
    for source in [
        "(|v: u8| v)",
        "async |v: u8| v",
        "move |v: u8| v",
        "for<'a> |v: &'a u8| *v",
        "const |v: u8| v",
        "make_converter()",
        "{ |v: u8| v }",
        "1 + 2",
    ] {
        assert!(
            syn::parse_str::<FunctionArg>(source).is_err(),
            "`{source}` is outside the `with` grammar"
        );
    }

    for source in ["|| 1", "|a, b| a"] {
        let error = syn::parse_str::<FunctionArg>(source).unwrap_err();
        assert_eq!(
            error.to_string(),
            "`with` closure must take exactly one parameter",
            "`{source}`"
        );
    }
}

#[test]
fn underscore_closure_inputs_take_the_field_type() {
    let file = derive_file(quote! {
        struct Sample {
            #[refield(with = |v: _| v + 1)]
            value: u8,
        }
    });
    let helper = helper_method(derived_impl(&file), "__refield_with_value_0");
    let rendered = helper.to_token_stream().to_string();
    assert!(
        rendered.contains("v : u8"),
        "`_` input falls back to the field type: `{rendered}`"
    );
    assert!(
        !rendered.contains("v : _"),
        "`_` must not reach the method signature: `{rendered}`"
    );
    let sized = helper
        .sig
        .generics
        .where_clause
        .as_ref()
        .map(|clause| clause.to_token_stream().to_string())
        .unwrap_or_default();
    assert!(
        !sized.contains("_ : :: core :: marker :: Sized"),
        "no `_` predicate is emitted: `{sized}`"
    );
}

#[test]
fn explicit_closure_return_type_is_preserved() {
    let file = derive_file(quote! {
        struct Sample {
            #[refield(with = |v: u8| -> u16 { u16::from(v) })]
            value: u16,
        }
    });
    let helper = helper_method(derived_impl(&file), "__refield_with_value_0");
    assert_eq!(
        helper
            .sig
            .output
            .to_token_stream()
            .to_string()
            .replace(' ', ""),
        "->u16",
        "the closure's declared return type becomes the helper's return type"
    );

    // `-> _` keeps the field type.
    let file = derive_file(quote! {
        struct Sample {
            #[refield(with = |v: u8| -> _ { v })]
            value: u8,
        }
    });
    let helper = helper_method(derived_impl(&file), "__refield_with_value_0");
    assert_eq!(
        helper
            .sig
            .output
            .to_token_stream()
            .to_string()
            .replace(' ', ""),
        "->u8"
    );
}

#[test]
fn block_expressions_are_not_wrapped_in_extra_braces() {
    let file = derive_file(quote! {
        struct Sample {
            #[refield(with = |v: u8| { v + 1 }, default = { 1 + 1 })]
            block_val: u8,
            #[refield(with = |v: u8| v + 1, default = 1 + 1)]
            plain_val: u8,
        }
    });
    let item_impl = derived_impl(&file);
    for name in [
        "__refield_with_block_val_0",
        "__refield_default_block_val_0",
        "__refield_with_plain_val_0",
        "__refield_default_plain_val_0",
    ] {
        let helper = helper_method(item_impl, name);
        let block = match &helper.block.stmts[..] {
            [syn::Stmt::Expr(syn::Expr::Block(_), None)] => {
                panic!("the body must be one block, not a block containing a block: `{name}`")
            }
            _ => helper.block.to_token_stream().to_string(),
        };
        assert!(
            !block.contains("{ {"),
            "the expression block must be spliced in directly: `{block}`"
        );
    }
}

// -----------------------------------------------------------------------
// REVIEW2 #3: one context macro, uniquely named helpers
// -----------------------------------------------------------------------

#[test]
fn derive_does_not_define_a_fallback_context_macro() {
    let file = derive_file(quote! { struct Sample { value: u8 } });
    assert!(
        !file.items.iter().any(|item| matches!(
            item,
            Item::Macro(item_macro)
                if item_macro
                    .ident
                    .as_ref()
                    .is_some_and(|ident| ident == "__refield_ctx__")
        )),
        "only `#[refieldable] impl` may define `__refield_ctx__`"
    );
}

#[test]
fn generated_helpers_use_the_schema_identity_as_prefix() {
    let file = derive_file(quote! {
        struct Sample {
            #[refield(with = |v| v, default = 1)]
            a: u8,
        }
    });
    let item_impl = derived_impl(&file);
    let check = check_method(item_impl).sig.ident.to_string();
    for method in impl_methods(item_impl) {
        let name = method.sig.ident.to_string();
        if name == check {
            continue;
        }
        assert!(
            name.starts_with(&format!("{check}_")),
            "`{name}` must be namespaced by `{check}`"
        );
    }
}

#[test]
fn user_methods_named_like_old_helpers_do_not_collide() {
    // The generated names are prefixed with the schema identity, so a user
    // method that happens to use the old fixed pattern is untouched.
    let file = derive_file(quote! {
        struct Sample {
            #[refield(with = |v| v + 1, default = 1)]
            a: u8,
        }
    });
    let generated = derived_impl(&file);
    // The user's own methods keep the old fixed names; the generated ones
    // are namespaced by the schema identity, so the two never collide.
    let names: Vec<String> = impl_methods(generated)
        .iter()
        .map(|method| method.sig.ident.to_string())
        .collect();
    assert!(
        !names.iter().any(|name| name == "__refield_with_a_0"),
        "the generated helper must not use the user-facing name: `{names:?}`"
    );
    assert!(
        !names.iter().any(|name| name == "__refield_default_a_0"),
        "the generated helper must not use the user-facing name: `{names:?}`"
    );
    assert_eq!(helper_count(generated, "__refield_with_a_0"), 1);
    assert_eq!(helper_count(generated, "__refield_default_a_0"), 1);
}

// -----------------------------------------------------------------------
// REVIEW2 #4: `Sized` lives on the methods that need it
// -----------------------------------------------------------------------

#[test]
fn identity_check_is_sized_but_the_struct_need_not_be() {
    let file = derive_file(quote! {
        struct Sample<T: ?Sized> {
            head: u8,
            #[refield(with = |v| v)]
            tail: T,
        }
    });
    let item_impl = derived_impl(&file);
    let check = check_method(item_impl);
    let where_clause = check
        .sig
        .generics
        .where_clause
        .as_ref()
        .expect("the identity check requires `Self: Sized`");
    assert!(
        where_clause
            .to_token_stream()
            .to_string()
            .replace(' ', "")
            .contains("Self:::core::marker::Sized"),
        "got `{where_clause}`",
        where_clause = where_clause.to_token_stream()
    );

    let helper = helper_method(item_impl, "__refield_with_tail_0");
    let where_clause = helper
        .sig
        .generics
        .where_clause
        .as_ref()
        .expect("every helper requires `Self: Sized`");
    let rendered = where_clause.to_token_stream().to_string().replace(' ', "");
    assert!(
        rendered.contains("Self:::core::marker::Sized"),
        "`{rendered}`"
    );
    assert!(
        rendered.contains("T:::core::marker::Sized"),
        "the value positions need `T: Sized`: `{rendered}`"
    );
}

#[test]
fn reference_helpers_do_not_add_a_sized_bound_for_the_referent() {
    let file = derive_file(quote! {
        struct Sample<'a, T: ?Sized> {
            #[refield(with = |v: &'a T| v)]
            value: &'a T,
        }
    });
    let helper = helper_method(derived_impl(&file), "__refield_with_value_0");
    let rendered = helper
        .sig
        .generics
        .where_clause
        .as_ref()
        .expect("`Self: Sized` is always present")
        .to_token_stream()
        .to_string()
        .replace(' ', "");
    assert!(
        rendered.contains("Self:::core::marker::Sized"),
        "`{rendered}`"
    );
    assert!(
        !rendered.contains("&'aT:::core::marker::Sized"),
        "a reference output is already `Sized`: `{rendered}`"
    );
}

// -----------------------------------------------------------------------
// REVIEW2 #5: the `impl` block's `crate` is the one that is used
// -----------------------------------------------------------------------

#[test]
fn impl_context_forwards_the_impl_blocks_crate() {
    let output = expand_refieldable(
        quote! { crate = patches },
        quote! {
            impl Target {
                fn build() {}
            }
        },
    )
    .expect("the impl block expands")
    .to_string();
    assert!(
        output.contains("(patches)"),
        "the `impl`-level crate must be forwarded to the companion macro: `{output}`"
    );
    assert!(
        !output.contains("($ ($ krate : tt) +)"),
        "the context macro arms no longer forward a caller-supplied crate: `{output}`"
    );
}

#[test]
fn non_nominal_self_reports_a_missing_context_rather_than_resolving() {
    let output = expand_refieldable(
        quote! { crate = ::refield },
        quote! {
            impl Trait for () {
                type Config = Target;
                fn build() -> Self::Config {
                    refield!(Self::Config { ..raw })
                }
            }
        },
    )
    .expect("associated type definitions are enough")
    .to_string();
    assert!(
        output.contains("cannot resolve struct schema from `Self` when the `impl` target type"),
        "the `@self` arm must degrade to a clear diagnostic: `{output}`"
    );
}
