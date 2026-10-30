use proc_macro2::{Delimiter, Group, Ident, Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use std::collections::HashSet;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::token::Brace;
use syn::{
    Error, Expr, FieldValue, Member, Pat, Path, RangeLimits, Result, Token, Type, TypePath,
    Visibility, WherePredicate, braced,
};

use super::args::Crate;
use super::field::{DefaultRule, FieldIr};
use super::utils::{UniqueId, is_place, member_to_string, parse, path_to_string, qs, sized};

/// Emit the companion macro shared by `#[derive(Refield)]` structs and
/// `#[refieldable] type` aliases.
///
/// Both spellings carry the same schema; the only difference is which
/// constructor the direct (body-only) arm uses. An alias calls its target's
/// `@__refield_alias` arm at the alias definition site, which hands the schema
/// to `alias!` to generate an alias companion macro with its own identity.
pub(super) struct SchemaMacro {
    pub(super) krate: Crate,
    pub(super) vis: Visibility,
    pub(super) ident: Ident,
    pub(super) check: Ident,
    pub(super) schema: TokenStream,
}

impl Parse for SchemaMacro {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        parse!(input =>
            (krate: Crate),
            (vis: Visibility),
            ident: Ident,
            check: Ident,
            [schema: TokenStream],
        )
    }
}

impl ToTokens for SchemaMacro {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            krate,
            vis,
            ident,
            check,
            schema,
        } = self;

        let arms = quote! {
            // A call-site provider is used whenever one exists: an annotated
            // `impl` block, an alias, or `refield!` itself resolved `refield`
            // at its own location, where a definition-site `crate = ...` alias
            // may not be in scope at all.
            (@__refield_call ($($krate:tt)+), ($($requested:tt)+), ($($path:tt)+), { $($body:tt)* }) => {
                $($krate)+::__private::construct! {
                    ($($krate)+),
                    #check,
                    [ #schema ],
                    ($($requested)+),
                    ($($path)+),
                    { $($body)* }
                }
            };
            // No provider: the call site cannot name the runtime crate (for
            // example it reaches `refield!` only through a facade), so the
            // definition-site `crate` configuration is the only usable path.
            (@__refield_call (), ($($requested:tt)+), ($($path:tt)+), { $($body:tt)* }) => {
                #krate::__private::construct! {
                    (#krate),
                    #check,
                    [ #schema ],
                    ($($requested)+),
                    ($($path)+),
                    { $($body)* }
                }
            };
            (@__refield_alias ($($krate:tt)+), ($($vis:tt)*), $ident:tt) => {
                $($krate)+::__private::alias! {
                    ($($krate)+),
                    ($($vis)*),
                    $ident,
                    #check,
                    [ #schema ]
                }
            };
            ($($body:tt)*) => {
                #krate::__private::construct! {
                    (#krate),
                    #check,
                    [ #schema ],
                    (#ident),
                    (#ident),
                    { $($body)* }
                }
            };
        };

        let name = {
            let id = UniqueId::new(ident, &arms);
            format_ident!("__refield__{id}", span = Span::mixed_site())
        };
        // Only an `#[refieldable] impl` defines a context macro. There is no
        // per-type fallback: redefining the same `macro_rules!` name inside a
        // method body would shadow the enclosing `impl` context and make
        // `refield!(Self { ... })` ambiguous.
        tokens.extend(if matches!(vis, Visibility::Public(_)) {
            quote! {
                #[doc(hidden)]
                #[allow(non_local_definitions)]
                #[macro_export]
                macro_rules! #name {
                    #arms
                }
                #[doc(hidden)]
                #[allow(unused_imports)]
                pub use #name as #ident;
            }
        } else {
            quote! {
                #[doc(hidden)]
                #[allow(unused_macros)]
                macro_rules! #name {
                    #arms
                }
                #[doc(hidden)]
                #[allow(unused_imports)]
                #vis use #name as #ident;
            }
        });
    }
}

/// One helper method generated on the target struct for a `with` or
/// `default` expression.
pub(super) struct Helper<'f> {
    pub(super) vis: &'f Visibility,
    pub(super) ident: &'f Ident,
    /// The parameter pattern and type, or `None` for a helper that takes no
    /// value (a `default` expression).
    pub(super) input: Option<(&'f Pat, &'f Type)>,
    pub(super) output: &'f Type,
    pub(super) body: TokenStream,
    pub(super) bounds: &'f [WherePredicate],
    pub(super) konst: bool,
}

impl ToTokens for Helper<'_> {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            vis,
            ident,
            input,
            output,
            body,
            bounds,
            konst,
        } = self;
        let marker = format_ident!("__refield_marker", span = Span::mixed_site());
        let konst = konst.then(|| quote! { const });
        let param = input.map(|(pat, ty)| quote! { #pat: #ty, });
        let predicates = {
            let mut predicates = vec![quote! { Self: ::core::marker::Sized }];
            predicates.extend(bounds.iter().map(|bound| quote! { #bound }));
            predicates.extend(input.and_then(|(_, ty)| sized(ty)));
            predicates.extend(sized(output));
            predicates
        };
        // The body is the user's `with` or `default` expression, so only the
        // lints of the generated signature are allowed. Lints on the generated
        // name (`non_snake_case`, `dead_code`) never reach the user: the name
        // carries the macro's hygiene.
        tokens.extend(quote! {
            #[doc(hidden)]
            #[inline(always)]
            #[allow(private_interfaces, private_bounds)]
            #vis #konst fn #ident(
                #param
                #marker: ::core::marker::PhantomData<Self>,
            ) -> #output
            where
                #(#predicates,)*
            {
                #body
            }
        });
    }
}

/// A `refield!(Constructor as Schema { ... })` request.
pub(super) struct CallInput {
    pub(super) constructor: TypePath,
    /// The schema written after `as`, if any.
    pub(super) r#as: Option<Path>,
    /// The span of the body's braces.
    pub(super) span: Span,
    pub(super) body: TokenStream,
}

impl Parse for CallInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let constructor = input.parse()?;
        let r#as = if input.peek(Token![as]) {
            input.parse::<Token![as]>()?;
            Some(input.parse()?)
        } else {
            None
        };
        if !input.peek(Brace) {
            return Err(input.error("expected `{` after type path"));
        }
        let content;
        let span = braced!(content in input).span.join();
        let body = content.parse()?;
        if !input.is_empty() {
            return Err(input.error("unexpected tokens after struct initialization body"));
        }
        Ok(Self {
            constructor,
            r#as,
            span,
            body,
        })
    }
}

pub(super) struct ConstructBody {
    /// The profile selected by an `@name` selector at the start of the body.
    pub(super) profile: Option<Path>,
    pub(super) fields: Vec<FieldValue>,
    /// The source, written either first as `source..` or last as `..source`.
    pub(super) rest: Option<Expr>,
    /// Whether the source was written first. A source written first is read
    /// before the written fields; one written last is read after them.
    pub(super) front: bool,
}

impl ConstructBody {
    /// Parse a source written first, `source..`, if the next entry is one.
    ///
    /// No field can be mistaken for it: a field starts with attributes or a
    /// member, and neither parses as a range with a start and no end.
    fn source(input: ParseStream<'_>) -> Result<Option<Expr>> {
        let fork = input.fork();
        let front = fork.parse::<Expr>().is_ok_and(|expr| {
            matches!(
                expr,
                Expr::Range(range)
                    if range.attrs.is_empty()
                        && range.start.is_some()
                        && range.end.is_none()
                        && matches!(range.limits, RangeLimits::HalfOpen(_))
            )
        }) && (fork.is_empty() || fork.peek(Token![,]));
        if !front {
            return Ok(None);
        }
        let Expr::Range(range) = input.parse::<Expr>()? else {
            unreachable!("the fork has just parsed a range");
        };
        Ok(range.start.map(|start| *start))
    }
}

impl Parse for ConstructBody {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        // `@name` is only meaningful as the first entry of the body. The
        // comma after it is optional, so `{ @alt ..raw }`, `{ @alt, ..raw }`,
        // and `{ @alt }` all parse.
        let profile = if input.peek(Token![@]) {
            input.parse::<Token![@]>()?;
            let profile = Path::parse_mod_style(input)?;
            let _ = input.parse::<Token![,]>();
            Some(profile)
        } else {
            None
        };

        // `source..` is the next entry after the selector, if any.
        let mut rest = Self::source(input)?;
        let front = rest.is_some();
        if front && !input.is_empty() {
            input.parse::<Token![,]>()?;
        }

        let mut fields = Vec::new();
        while !input.is_empty() {
            if input.peek(Token![..]) {
                let dots = input.parse::<Token![..]>()?;
                if front {
                    return Err(Error::new_spanned(
                        dots,
                        "the source is already written first as `source..`",
                    ));
                }
                rest = Some(input.parse()?);
                if !input.is_empty() {
                    return Err(input.error("cannot have a comma or fields after `..`"));
                }
                break;
            }
            if input.peek(Token![@]) {
                let at = input.parse::<Token![@]>()?;
                return Err(Error::new(
                    at.span,
                    "`@profile` selector must be the first entry in the body",
                ));
            }
            if let Some(source) = Self::source(input)? {
                return Err(Error::new_spanned(
                    source,
                    "`source..` must be the first entry in the body",
                ));
            }
            fields.push(input.parse()?);
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        // A value written last would have to be evaluated after the written
        // fields, yet bound before the automatic fields read it. Without
        // written fields, there is nothing to evaluate before it.
        if !front
            && !fields.is_empty()
            && let Some(rest) = rest.as_ref().filter(|rest| !is_place(rest))
        {
            return Err(Error::new_spanned(
                rest,
                "a value source cannot follow written fields; write it first as `source..`, or bind it to a local and spread the local",
            ));
        }
        Ok(Self {
            profile,
            fields,
            rest,
            front,
        })
    }
}

pub(super) struct ConstructInput {
    krate: Crate,
    check: Ident,
    schema: Vec<FieldIr>,
    requested: TokenStream,
    constructor: TokenStream,
    pub(super) body: ConstructBody,
}

impl ConstructInput {
    /// Render the final construction expression.
    ///
    /// Every written field present here has already survived the compiler's
    /// attribute processing, so the automatic fields are excluded by member
    /// alone.
    pub(super) fn expand(&self) -> TokenStream {
        let Self {
            krate,
            check,
            schema,
            requested,
            constructor,
            body,
        } = self;

        // Every helper method generated on the target struct takes the same
        // `PhantomData<Self>` argument. It is the first argument of the
        // identity check and of every helper call, which binds it to the
        // requested type, so a turbofish-free call still infers the struct's
        // generic arguments.
        let marker = format_ident!("__refield_marker", span = Span::mixed_site());
        let base = format_ident!("__refield_base", span = Span::mixed_site());
        let source = format_ident!("__refield_source", span = Span::mixed_site());

        // Where automatic fields read their values from. A place is read
        // directly per field; a value source is bound once below.
        let place = body.rest.as_ref().is_some_and(is_place);
        // A read that fails (the source lacks the field, holds another type,
        // or is borrowed or moved) is an error in the source the user wrote,
        // so every read spans that source, from its first token to its last.
        // Only the location is borrowed: the tokens keep the macro's hygiene,
        // so they still count as generated code.
        let (open, close) = {
            let mut spans = body
                .rest
                .to_token_stream()
                .into_iter()
                .map(|tree| tree.span());
            let first = spans.next().unwrap_or_else(Span::call_site);
            let last = spans.last().unwrap_or(first);
            (
                Span::mixed_site().located_at(first),
                Span::mixed_site().located_at(last),
            )
        };
        let rest = body.rest.as_ref().map(|rest| {
            if place {
                rest.to_token_stream()
            } else {
                let mut source = source.clone();
                source.set_span(open);
                quote! { #source }
            }
        });

        // The number of written schema fields, and every other field with the
        // rule it uses for the selected profile, falling back to its default
        // rule. Automatic fields follow the schema's declaration order.
        let (written, automatic) = {
            let members = body
                .fields
                .iter()
                .map(|field| member_to_string(&field.member))
                .collect::<HashSet<_>>();
            let (written, automatic) = schema
                .iter()
                .partition::<Vec<_>, _>(|field| members.contains(&member_to_string(&field.target)));
            let automatic = automatic
                .into_iter()
                .map(|field| {
                    let rule = body
                        .profile
                        .as_ref()
                        .and_then(|profile| field.profile(profile))
                        .unwrap_or(&field.rule);
                    (field, rule)
                })
                .collect::<Vec<_>>();
            (written.len(), automatic)
        };
        // Whether an automatic field reads the source outside the
        // `#[allow(deprecated)]` of a deprecated target field.
        let reads = body.rest.is_some()
            && automatic
                .iter()
                .any(|(field, rule)| !rule.skip && !field.deprecated);

        // Each automatic field's initializer, keeping the `with` / `default` /
        // `skip` semantics. A field with neither a source nor a default gets
        // none and stays missing.
        let fills = automatic
            .into_iter()
            .filter_map(|(field, rule)| {
                let default = match &rule.default {
                    DefaultRule::None => None,
                    DefaultRule::Trait(span) => {
                        Some((*span, qs! { *span => ::core::default::Default::default() }))
                    }
                    DefaultRule::Method(helper) => Some((
                        helper.span(),
                        qs! { helper.span() => #requested::#helper(#marker) },
                    )),
                };
                // A skipped field takes its default, which it always has. So
                // does every field without `..rest`, which leaves nothing to
                // extract from; a field without a default stays missing.
                let rest = match &rest {
                    Some(rest) if !rule.skip => rest,
                    _ => return default.map(|(_, default)| (field, default)),
                };
                let read = {
                    let mut member = rule.source.clone();
                    match &mut member {
                        Member::Named(ident) => ident.set_span(close),
                        Member::Unnamed(index) => index.span = close,
                    }
                    let mut group = Group::new(Delimiter::Parenthesis, rest.clone());
                    group.set_span(open);
                    qs! { close => #group.#member }
                };
                let convert = |value| match &rule.with {
                    None => value,
                    Some(helper) => qs! { helper.span() => #requested::#helper(#value, #marker) },
                };
                let fill = match default {
                    Some((span, default)) => {
                        let value = format_ident!("__refield_value", span = Span::mixed_site());
                        let converted = convert(quote! { #value });
                        qs! { span =>
                            match #read {
                                ::core::option::Option::Some(#value) => #converted,
                                ::core::option::Option::None => #default,
                            }
                        }
                    }
                    None => convert(read),
                };
                Some((field, fill))
            })
            .collect::<Vec<_>>();

        // Every schema field is either written or filled automatically. Only
        // then may the literal carry `..base`: a missing field must still be
        // reported by `rustc` as E0063 rather than read from the placeholder.
        let complete = written + fills.len() == schema.len();

        // Each automatic field reads the source right in the literal. Whether
        // the target is deprecated is known from the schema: emitting
        // `#[allow(deprecated)]` unconditionally would turn a call inside a
        // `#[forbid(deprecated)]` scope into an E0453 hard error.
        let fills = fills.into_iter().map(|(field, fill)| {
            let target = &field.target;
            let allow = field.deprecated.then(|| quote! { #[allow(deprecated)] });
            qs! { target.span() =>
                #allow
                #target: #fill
            }
        });
        // Written fields stay untouched in the literal: attributes, spans,
        // parentheses, expected types, and temporary lifetime extension are
        // the ones of a native struct literal. Fields are evaluated in literal
        // order, so a source written first is read before the written fields
        // and a place written last after them.
        let written = body.fields.iter().map(ToTokens::to_token_stream);
        let fills = if body.front {
            fills.chain(written).collect::<Vec<_>>()
        } else {
            written.chain(fills).collect::<Vec<_>>()
        };

        // A value source is evaluated exactly once, before every field. It is
        // bound even when no automatic field reads it, so whatever is left of
        // it is always dropped after the construction, at the end of the
        // expansion.
        let bind = body
            .rest
            .as_ref()
            .filter(|_| !place)
            .map(|rest| quote! { let #source = #rest; });
        // A place that no automatic field reads is still touched, so it counts
        // as used; `let _ = place;` never moves or reads it. A place read only
        // under `#[allow(deprecated)]` is touched as well, so its own lints (a
        // deprecated static, say) are still reported.
        let touch = body
            .rest
            .as_ref()
            .filter(|_| place && !reads)
            .map(|rest| quote! { let _ = #rest; });

        let (tie, tail) = if complete {
            // `..base` ties the literal's type to the requested type (with
            // every generic argument) without wrapping the literal in a call,
            // so the literal stays an extending tail expression. `base` is
            // only assigned in dead code and moved out right away: it is never
            // read (every field is listed), never dropped, and needs no
            // destructor in `const fn`. The identity method is only defined on
            // the type the schema was derived for, so a mismatched
            // `as Schema` is rejected even when both types have identical
            // fields.
            (
                Some(quote! {
                    let #base;
                    if false {
                        #base = #requested::#check(#marker, #krate::__private::fake());
                        #krate::__private::sink(#base);
                    }
                }),
                quote! {
                    #constructor {
                        #(#fills,)*
                        ..#base
                    }
                },
            )
        } else {
            // A field is missing: keep the identity check around the literal
            // and let `rustc` report E0063.
            (
                None,
                quote! {
                    #requested::#check(#marker, #constructor {
                        #(#fills,)*
                    })
                },
            )
        };

        // The block's braces come from a Rust 2021 crate, so temporaries in
        // the literal live to the end of the caller's statement, as they do
        // for a plain struct literal in every edition. Only the tail is exempt
        // from `clippy::needless_update`, which its `..base` always triggers.
        quote! {
            #krate::__private::block! {
                {
                    let #marker = ::core::marker::PhantomData;
                    #tie
                    #bind
                    #touch
                }
                { #tail }
            }
        }
    }
}

impl Parse for ConstructInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let construct = parse!(input =>
            (krate: Crate),
            check: Ident,
            [schema: Vec<FieldIr>],
            (requested: TokenStream),
            (constructor: TokenStream),
            {body: ConstructBody},
        )?;
        // Reject a selected profile that no field of the schema declares.
        if let Some(profile) = &construct.body.profile
            && !construct
                .schema
                .iter()
                .any(|field| field.profile(profile).is_some())
        {
            return Err(Error::new_spanned(
                profile,
                format!("unknown profile `{}`", path_to_string(profile)),
            ));
        }
        Ok(construct)
    }
}

impl ToTokens for ConstructInput {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        // Fields without attributes are emitted directly. As soon as any
        // written field carries an attribute, the expression is routed through
        // the compiler's preparation stage instead: `cfg` and `cfg_attr` are
        // applied by `rustc` to a carrier before this macro decides which
        // fields survive, so no `cfg` predicate is evaluated (or even
        // interpreted) here.
        if self.body.fields.iter().all(|field| field.attrs.is_empty()) {
            tokens.extend(self.expand());
            return;
        }

        // The carrier hands the written fields to `rustc` first and receives
        // them back through the `Prepare` derive.
        let Self {
            krate,
            check,
            schema,
            requested,
            constructor,
            body,
        } = self;
        let callback = format_ident!("{check}__prepared_call", span = Span::mixed_site());
        // The `@profile` selector is not Rust syntax, so it travels in the
        // metadata rather than in the placeholder expression. The metadata is
        // the ordinary `construct!` protocol with an empty body, so the same
        // parser reads it back. `source..` is not Rust syntax either: the
        // placeholder always spells the source `..rest`, and its position
        // travels in the metadata.
        let profile = body.profile.as_ref().map(|profile| quote! { @ #profile });
        let front = body.front;
        let fields = &body.fields;
        let rest = body.rest.as_ref().map(|rest| quote! { ..#rest });
        tokens.extend(quote! {
            #krate::__private::block! {
                #[derive(#krate::__private::Prepare)]
                #[refield(__construct(
                    #callback,
                    #front,
                    (#krate),
                    #check,
                    [ #(#schema,)* ],
                    (#requested),
                    (#constructor),
                    { #profile }
                ))]
                #[#krate::__private::discard]
                struct __RefieldCarrier([(); {
                    // `rustc` strips disabled fields and expands `cfg_attr`
                    // before `Prepare` sees this expression. It is never type
                    // checked: `discard` removes the carrier first.
                    __RefieldCarrier {
                        #(#fields,)*
                        #rest
                    };
                    0
                }]);
                #callback!($)
            }
        });
    }
}

/// The `__construct(...)` metadata a prepared construction carrier carries:
/// the callback name, the source position, and one ordinary `construct!`
/// payload.
pub(super) struct PreparedConstruct {
    pub(super) callback: Ident,
    /// Whether the source was written first, as `source..`.
    pub(super) front: bool,
    pub(super) construct: ConstructInput,
}

impl Parse for PreparedConstruct {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        parse!(input => callback: Ident, front: bool, construct: ConstructInput)
    }
}
