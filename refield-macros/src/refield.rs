mod args;
mod codegen;
mod field;
mod utils;

use darling::ast::NestedMeta;
use darling::{FromAttributes, FromMeta};
use proc_macro2::{Delimiter, Group, Span, TokenStream, TokenTree};
use quote::{ToTokens, format_ident, quote};
use syn::ext::IdentExt;
use syn::spanned::Spanned;
use syn::{
    Data, DeriveInput, Error, Expr, ImplItem, Item, Result, Stmt, Type, TypePath, parse_quote,
    parse2,
};

use args::{AliasArgs, Crate, ImplArgs, StructArgs};
use codegen::{CallInput, ConstructInput, PreparedConstruct, SchemaMacro};
use field::FieldIr;
use utils::{
    UniqueId, associated, bare, carried, combine, concrete, escape, is_refield, is_self, metadata,
    nominal, retain, span, turbofish,
};

pub fn proc(args: TokenStream, input: &TokenStream) -> Result<TokenStream> {
    match parse2::<Item>(input.clone())? {
        Item::Struct(mut item) => {
            if !args.is_empty() {
                item.attrs.push(parse_quote! { #[refield(#args)] });
            }
            let krate = StructArgs::from_attributes(&item.attrs)?
                .krate
                .or_else(Crate::infer)
                .unwrap_or_default();
            item.attrs
                .insert(0, parse_quote! { #[derive(#krate::Refield)] });
            Ok(quote! { #item })
        }
        Item::Type(item) => {
            let args = AliasArgs::from_list(&NestedMeta::parse_meta_list(args)?)?;
            if let Some(attr) = item.attrs.iter().find(|attr| is_refield(attr)) {
                return Err(Error::new_spanned(
                    attr,
                    "`refield` attributes are not supported on type aliases; pass arguments to `#[refieldable(...)]`",
                ));
            }
            let krate = args.krate.or_else(Crate::infer).unwrap_or_default();
            let target = match args.r#as {
                // `as = Self` / `as = Type::<T>::Assoc` / `as = Target<u32>` are
                // validated here, at the alias definition site. Only the bare path
                // is kept: it names the companion macro, while the literal path is
                // the alias itself.
                Some(path) => bare(&concrete(path)?),
                None => nominal(
                    &item.ty,
                    "`refieldable` type aliases must alias a struct or type alias path, or specify `#[refieldable(as = Schema)]`",
                )?,
            };
            // Ask the target's companion macro for its schema *at the alias
            // definition site*, so every path in the schema resolves where the alias
            // was written. The target then calls `__private::alias!`, which
            // generates an independent companion macro for the alias itself.
            let (vis, ident) = (&item.vis, &item.ident);
            Ok(quote! {
                #item
                #target! {
                    @__refield_alias
                    (#krate),
                    (#vis),
                    #ident
                }
            })
        }
        Item::Impl(item) => {
            let krate = ImplArgs::from_list(&NestedMeta::parse_meta_list(args)?)?
                .krate
                .or_else(Crate::infer)
                .unwrap_or_default();
            Ok(quote! {
                #[derive(#krate::__private::Prepare)]
                #[refield(crate = #krate)]
                #[#krate::__private::discard]
                struct __RefieldCarrier([(); {
                    #item
                    0
                }]);
            })
        }
        mut item => {
            retain(&mut item, |_| false);
            Err(Error::new_spanned(
                item,
                "`refieldable` can only be applied to structs, type aliases, or `impl` blocks",
            ))
        }
    }
}

/// Remove every `#[refield(...)]` attribute from the item and everything it
/// contains, returning the cleaned tokens.
pub fn strip(input: &TokenStream) -> TokenStream {
    match parse2::<Item>(input.clone()) {
        Ok(mut item) => {
            retain(&mut item, |attr| !is_refield(attr));
            quote! { #item }
        }
        Err(_) => input.clone(),
    }
}

pub fn derive(input: TokenStream) -> Result<TokenStream> {
    let raw = input.clone();
    let input = parse2::<DeriveInput>(input)?;

    // Reject non-structs before anything else, so an `enum` gets one clear
    // diagnostic instead of an unrelated attribute error.
    let data = match &input.data {
        Data::Struct(data) => data,
        Data::Enum(data) => {
            return Err(Error::new(
                data.enum_token.span,
                "`Refield` can only be derived for structs",
            ));
        }
        Data::Union(data) => {
            return Err(Error::new(
                data.union_token.span,
                "`Refield` can only be derived for structs",
            ));
        }
    };

    let check = {
        let id = UniqueId::new(&input.ident, &raw);
        format_ident!("__refield_check__{id}", span = Span::mixed_site())
    };
    // Parse the struct-level arguments and the fields in one pass, combining
    // both failures so a single compilation reports every offending attribute.
    let args = match StructArgs::from_attributes(&input.attrs) {
        Ok(args) => args,
        Err(error) => {
            let mut error = Error::from(error);
            if let Err(fields) = FieldIr::extract(&data.fields, &StructArgs::default(), &check) {
                error.combine(fields);
            }
            return Err(error);
        }
    };
    let (fields, helpers) = FieldIr::extract(&data.fields, &args, &check)?;
    let krate = args.krate.or_else(Crate::infer).unwrap_or_default();

    // The schema identity method and every field helper live on the struct
    // itself, so custom `with` / `default` expressions evaluate in the
    // definition site's generic scope and can reach private items. Every
    // schema name maps to exactly one such method, so `construct!` can require
    // that the constructed value belongs to the requested type before
    // returning it.
    let helpers = {
        // `construct!` passes the same `PhantomData<Self>` marker to this
        // check and to every helper call, so they all infer the same generic
        // arguments. The value is either a dead placeholder whose type
        // `..base` gives the literal, or the literal itself when a field is
        // missing.
        let marker = format_ident!("__refield_marker", span = Span::mixed_site());
        let value = format_ident!("__refield_value", span = Span::mixed_site());
        let ident = &input.ident;
        let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
        quote! {
            #[automatically_derived]
            impl #impl_generics #ident #type_generics #where_clause {
                #[doc(hidden)]
                #[inline(always)]
                #[allow(private_interfaces, private_bounds)]
                pub const fn #check(
                    #marker: ::core::marker::PhantomData<Self>,
                    #value: Self,
                ) -> Self
                where
                    Self: ::core::marker::Sized,
                {
                    #value
                }
                #(#helpers)*
            }
        }
    };
    let companion = SchemaMacro {
        krate,
        vis: input.vis,
        ident: input.ident,
        check,
        schema: quote! { #(#fields,)* },
    };

    Ok(quote! {
        #helpers
        #companion
    })
}

pub fn call(input: TokenStream) -> Result<TokenStream> {
    let CallInput {
        constructor,
        r#as,
        span,
        body,
    } = parse2(input)?;

    // Every deferred-context arm forwards the caller's full request (with
    // generic arguments) plus the untouched body tokens.
    let requested = turbofish(constructor.clone());
    let body = {
        let mut body = Group::new(Delimiter::Brace, body);
        body.set_span(span);
        body
    };

    let (path, literal) = match r#as {
        // A qualified path (`<Ty as Trait>::Assoc`) is rejected in
        // struct-expression position on stable Rust (E0658), and a `Ty::Assoc`
        // head is ambiguous (E0223). The explicit schema names the same type,
        // so it is used for the literal; `requested` keeps the caller's path
        // (and its generic arguments) for the schema identity check.
        //
        // The companion macro itself is named by the bare path; only the
        // literal keeps the generic arguments.
        Some(path) => {
            let path = concrete(path)?;
            (bare(&path), path)
        }
        // Without an explicit schema, `Self` and the associated types of
        // `Self` are resolved by the enclosing `#[refieldable] impl` context,
        // if any. The request is emitted as a call to the unqualified
        // `__refield_ctx__` macro, which is defined lexically inside such an
        // `impl` block. The context macro carries the `crate` path resolved
        // from the `impl` block's own attributes.
        None => {
            let segments = &constructor.path.segments;
            // `Self::Assoc`, `Self::Assoc::<Args>`, and `<Self as Trait>::Assoc`
            // name the same candidate and share one forwarding: the trait is
            // not matched by name here. The candidate schema of the enclosing
            // `impl` block is validated at compile time by the schema identity
            // check on the *requested* type, so a renamed trait import resolves
            // exactly like the written one, while `<Self as OtherTrait>::Assoc`
            // is rejected unless its schema is named explicitly.
            let assoc = match &constructor.qself {
                None => (segments.len() == 2
                    && segments[0].ident == "Self"
                    && segments[0].arguments.is_none())
                .then(|| &segments[1]),
                Some(qself) => match &*qself.ty {
                    Type::Path(ty) => (ty.qself.is_none()
                        && ty.path.is_ident("Self")
                        && qself.position >= 1
                        && segments.len() == qself.position + 1)
                        .then(|| &segments[qself.position]),
                    _ => None,
                },
            };
            if let Some(assoc) = assoc {
                let key = assoc.ident.unraw();
                span!(key.span());
                return Ok(q! {
                    __refield_ctx__! {
                        @assoc
                        #key,
                        (#requested),
                        #body
                    }
                });
            }
            if constructor.qself.is_some() {
                return Err(Error::new(
                    constructor.span(),
                    "associated types require an explicit schema `refield!(<T as Trait>::Assoc as Schema { ... })` or an enclosing `#[refieldable] impl` block",
                ));
            }
            if let Some(segment) = associated(&constructor.path) {
                return Err(Error::new(
                    segment.span(),
                    "associated types require an explicit schema `refield!(Type::Assoc as Schema { ... })` or an enclosing `#[refieldable] impl` block",
                ));
            }
            if is_self(&constructor.path) {
                let ident = &segments[0].ident;
                span!(ident.span());
                return Ok(q! {
                    __refield_ctx__! {
                        @self
                        #ident,
                        (#requested),
                        #body
                    }
                });
            }
            (bare(&constructor.path), constructor.path)
        }
    };

    // The requested type keeps every generic argument and qualifier: the
    // schema identity check runs on it. The literal path uses the explicit
    // schema instead when the requested type is not a struct literal.
    let literal = turbofish(TypePath {
        attrs: Vec::new(),
        qself: None,
        path: literal,
    });

    // The runtime path is resolved where `refield!` is written, if this
    // compilation names the `refield` crate at all: a definition-site
    // `crate = patches` alias may only be in scope in the defining module.
    // An empty group leaves the companion macro's definition-site `crate`
    // as the fallback for callers that reach `refield!` through a facade
    // without a direct dependency entry.
    let entry = match Crate::infer() {
        Some(krate) => quote! { (#krate) },
        None => quote! { () },
    };
    Ok(quote! {
        #path! {
            @__refield_call
            #entry,
            (#requested),
            (#literal),
            #body
        }
    })
}

pub fn construct(input: TokenStream) -> Result<TokenStream> {
    parse2::<ConstructInput>(input).map(ToTokens::into_token_stream)
}

pub fn alias(input: TokenStream) -> Result<TokenStream> {
    parse2::<SchemaMacro>(input).map(ToTokens::into_token_stream)
}

/// Recover what a carrier wraps once `rustc` has processed its attributes: a
/// construction with attributed fields, or a `#[refieldable] impl` block.
///
/// A token stream that does not unwrap to the expected item is reported
/// instead of silently expanding to nothing.
pub fn prepare(input: &TokenStream) -> Result<TokenStream> {
    let input = parse2::<DeriveInput>(input.clone())?;

    if let Some(metadata) = metadata(&input) {
        // The carried expression has already been stripped and expanded by
        // `rustc`, so only the fields that really survive are present. They
        // replace the empty metadata body, and the result is emitted as a
        // callback macro the caller expands at the original position.
        let PreparedConstruct {
            callback,
            front,
            mut construct,
        } = parse2(metadata)?;
        let expr = carried(&input).iter().find_map(|stmt| match stmt {
            Stmt::Expr(Expr::Struct(expr), _) => Some(expr),
            _ => None,
        });
        // Attributes on the expression belong to the whole construction and
        // were already handled by the compiler.
        let expr = match expr {
            Some(expr) if expr.attrs.is_empty() && expr.qself.is_none() => expr,
            Some(_) => {
                return Err(Error::new_spanned(
                    &input.ident,
                    "internal `refield` carrier field expression has an unexpected shape",
                ));
            }
            None => {
                return Err(Error::new_spanned(
                    &input.ident,
                    "internal `refield` carrier does not contain a field expression",
                ));
            }
        };
        construct.body.fields = expr.fields.iter().cloned().collect();
        construct.body.rest = expr.rest.as_deref().cloned();
        construct.body.front = front;
        // The callback receives one `$` token and expands it in place of every
        // dollar written by the user. `macro_rules!` would otherwise interpret a
        // `$(` repeat in the user expression as part of its own right-hand side.
        let dollar = format_ident!("__refield_dollar", span = Span::mixed_site());
        let expansion = escape(construct.expand(), &dollar);
        return Ok(quote! {
            #[allow(unused_macros)]
            macro_rules! #callback {
                ($#dollar:tt) => {
                    #expansion
                };
            }
        });
    }

    let krate = StructArgs::from_attributes(&input.attrs)?
        .krate
        .or_else(Crate::infer)
        .unwrap_or_default();
    let item = match carried(&input).iter().find_map(|stmt| match stmt {
        Stmt::Item(item) => Some(item),
        _ => None,
    }) {
        Some(Item::Impl(item)) => item,
        Some(_) => {
            return Err(Error::new_spanned(
                &input.ident,
                "internal `refieldable` carrier does not contain an `impl` block",
            ));
        }
        None => {
            return Err(Error::new_spanned(
                &input.ident,
                "internal `refieldable` carrier does not contain an item",
            ));
        }
    };

    {
        let mut errors = None;
        for attr in item.attrs.iter().filter(|attr| is_refield(attr)) {
            combine(
                &mut errors,
                Error::new_spanned(
                    attr,
                    "`refield` attributes are not supported on `impl` blocks; pass arguments to `#[refieldable(...)]`",
                ),
            );
        }
        for item in &item.items {
            let attrs = match item {
                ImplItem::Const(item) => &item.attrs,
                ImplItem::Fn(item) => &item.attrs,
                ImplItem::Type(item) => &item.attrs,
                ImplItem::Macro(item) => &item.attrs,
                _ => continue,
            };
            for attr in attrs.iter().filter(|attr| is_refield(attr)) {
                combine(
                    &mut errors,
                    Error::new_spanned(
                        attr,
                        "`refield` attributes are not supported on items inside an `impl` block",
                    ),
                );
            }
        }
        if let Some(errors) = errors {
            return Err(errors);
        }
    }

    let target = nominal(&item.self_ty, "").ok();
    // Associated types are keyed by name only. Whether `<Self as Trait>::Assoc`
    // really is the type this block defines is decided by the schema identity
    // check on the requested type, not by matching the trait's spelling here.
    let assoc = item
        .items
        .iter()
        .filter_map(|item| match item {
            ImplItem::Type(item) => Some((item.ident.unraw(), nominal(&item.ty, "").ok()?)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if target.is_none() && assoc.is_empty() {
        return Err(Error::new_spanned(
            &item.self_ty,
            "`refieldable` `impl` blocks require a nominal target `Self` type or associated type definitions with nominal paths",
        ));
    }

    let arms = {
        let target = match &target {
            Some(target) => quote! {
                (@self $self_ident:ident, ($($requested:tt)+), { $($body:tt)* }) => {
                    #target! {
                        @__refield_call
                        (#krate),
                        ($($requested)+),
                        ($self_ident),
                        { $($body)* }
                    }
                };
            },
            None => quote! {
                (@self $($rest:tt)*) => {
                    ::core::compile_error!(
                        "cannot resolve struct schema from `Self` when the `impl` target type is not a struct or type alias path"
                    )
                };
            },
        };
        let assoc = assoc.iter().map(|(key, path)| {
            quote! {
                (@assoc #key, ($($requested:tt)+), { $($body:tt)* }) => {
                    #path! {
                        @__refield_call
                        (#krate),
                        ($($requested)+),
                        (#path),
                        { $($body)* }
                    }
                };
            }
        });
        quote! { #target #(#assoc)* }
    };
    Ok(quote! {
        const _: () = {
            #[allow(unused_macros)]
            macro_rules! __refield_ctx__ {
                #arms
                (@assoc $($rest:tt)*) => {
                    ::core::compile_error!(
                        "cannot resolve associated type schema from `Self::Assoc` or `<Self as Trait>::Assoc`; define the associated type in this `#[refieldable] impl` block or specify `as Schema`"
                    )
                };
            }
            #item
        };
    })
}

/// Emit placeholder tokens for a carrier whose `Prepare` failed.
///
/// This path deliberately tolerates a malformed carrier: the real diagnostic
/// has already been produced, and the compiler must not add a cascade of
/// follow-up errors.
///
/// A construction carrier is never recovered as code: its array-length block
/// is a placeholder expression that only existed for the compiler's attribute
/// pass. Only the callback macro the caller invokes is restored, with a body
/// that reports an error instead of a "cannot find macro" cascade. The `impl`
/// carrier keeps emitting its original item.
pub fn recover(input: &TokenStream) -> TokenStream {
    let Ok(input) = parse2::<DeriveInput>(input.clone()) else {
        return TokenStream::new();
    };
    if let Some(metadata) = metadata(&input) {
        let callback = metadata
            .into_iter()
            .find_map(|tree| match tree {
                TokenTree::Ident(ident) => Some(ident),
                _ => None,
            })
            .unwrap_or_else(|| format_ident!("__refield_prepared_call"));
        // The caller always invokes the callback with the `$` token that the
        // prepared body uses to escape user-written dollars; accept it and
        // ignore it, because this body only reports the error.
        let dollar = format_ident!("__refield_dollar", span = Span::mixed_site());
        return quote! {
            #[allow(unused_macros)]
            macro_rules! #callback {
                ($#dollar:tt) => {
                    ::core::compile_error!("`refield` could not prepare this construction")
                };
            }
        };
    }
    match carried(&input).iter().find_map(|stmt| match stmt {
        Stmt::Item(item) => Some(item),
        _ => None,
    }) {
        Some(item) => {
            let mut item = item.clone();
            retain(&mut item, |attr| !is_refield(attr));
            quote! { #item }
        }
        None => TokenStream::new(),
    }
}

#[cfg(test)]
mod tests;
