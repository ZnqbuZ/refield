use darling::util::{Flag, Override, PreservedStrExpr, SpannedValue};
use darling::{FromAttributes, FromMeta};
use derive_more::Deref;
use proc_macro_crate::{FoundCrate, crate_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{
    Error, Expr, ExprBlock, ExprLit, Generics, Lit, LitStr, Meta, Pat, PatType, Path, Result,
    Token, WherePredicate, parse_quote, parse2,
};

use super::utils::pqs;

#[derive(Debug, FromMeta)]
#[darling(default)]
pub(super) struct Crate(Path);

impl Crate {
    /// The runtime path resolved from the *current* compilation's manifest.
    ///
    /// `None` means this crate has no usable `refield` dependency entry, so a
    /// caller that needs to tell a real call-site entry from the definition-site
    /// default can fall back to [`Default::default`] explicitly.
    pub(super) fn infer() -> Option<Self> {
        match crate_name("refield") {
            Ok(FoundCrate::Name(name)) => {
                let name = format_ident!("{}", name);
                Some(Self(parse_quote! { ::#name }))
            }
            // Inside the runtime crate itself (including its doctests), the
            // `extern crate self as refield` alias makes `::refield` valid.
            Ok(FoundCrate::Itself) => Some(Self::default()),
            Err(_) => None,
        }
    }
}

impl Default for Crate {
    fn default() -> Self {
        Self(parse_quote! { ::refield })
    }
}

impl Parse for Crate {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        input.parse().map(Self)
    }
}

impl ToTokens for Crate {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        self.0.to_tokens(tokens);
    }
}

#[derive(Debug, Deref)]
pub(super) struct Bounds(Punctuated<WherePredicate, Token![,]>);

impl FromMeta for Bounds {
    fn from_meta(meta: &Meta) -> darling::Result<Self> {
        match meta {
            Meta::List(list) => Ok(Self(list.parse_args_with(Punctuated::parse_terminated)?)),
            _ => Err(darling::Error::custom("expected `bound(...)`").with_span(meta)),
        }
    }
}

#[derive(Debug, Default, FromMeta)]
#[darling(default)]
pub(super) struct GeneralArgs {
    pub(super) name: Option<LitStr>,
    pub(super) with: Option<PseudoClosure>,
    #[darling(rename = "const")]
    pub(super) konst: Flag,
}

/// The conversion written as `with = ...` or `with(...)`.
///
/// Only pseudo-closure shorthand is accepted: `|pattern: Type| body`,
/// optionally preceded by generic parameters and followed by a `where` clause
/// and a block body (`<T> |pattern: T| where T: Trait { body }`); those two
/// only fit in `with(...)`. Any other expression is a syntax error at the
/// point where the value starts, which keeps unsupported closure modifiers
/// (`async`, `move`, `const`, `for<...>`), return types, and bare function
/// paths out of the grammar.
#[derive(Debug)]
pub(super) struct PseudoClosure {
    /// The generic parameters and the `where` clause.
    pub(super) generics: Generics,
    open: Token![|],
    /// The parameter; an untyped one is read as `pattern: _`.
    pub(super) param: PatType,
    close: Token![|],
    pub(super) body: Expr,
}

impl Parse for PseudoClosure {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut generics = input.parse::<Generics>()?;
        if !input.peek(Token![|]) {
            return Err(input.error("expected pseudo-closure `|pattern: Type| body`"));
        }
        let open = input.parse::<Token![|]>()?;
        let mut params = Punctuated::<PatType, Token![,]>::new();
        while !input.peek(Token![|]) {
            let pat = Pat::parse_single(input)?;
            params.push_value(if input.peek(Token![:]) {
                PatType {
                    attrs: Vec::new(),
                    pat: Box::new(pat),
                    colon_token: input.parse()?,
                    ty: input.parse()?,
                }
            } else {
                pqs! { pat.span() => #pat: _ }
            });
            if input.peek(Token![|]) {
                break;
            }
            params.push_punct(input.parse()?);
        }
        let close = input.parse::<Token![|]>()?;
        if params.len() != 1 {
            // With at least one parameter the diagnostic points at the
            // parameter list; `||` has none, so both bars are highlighted.
            let span = if params.is_empty() {
                quote! { #open #close }
            } else {
                params.to_token_stream()
            };
            return Err(Error::new_spanned(
                span,
                "`with` closure must take exactly one parameter",
            ));
        }
        let param = params
            .into_iter()
            .next()
            .expect("the parameter count is checked above");
        if input.peek(Token![->]) {
            return Err(input.error(
                "`with` closure cannot declare a return type; it always returns the field type",
            ));
        }
        generics.where_clause = input.parse()?;
        // An unbraced body could not be told apart from the end of the
        // `where` clause.
        let body = if generics.where_clause.is_some() {
            Expr::Block(ExprBlock {
                attrs: Vec::new(),
                label: None,
                block: input.parse()?,
            })
        } else {
            input.parse()?
        };
        Ok(Self {
            generics,
            open,
            param,
            close,
            body,
        })
    }
}

impl FromMeta for PseudoClosure {
    fn from_meta(meta: &Meta) -> darling::Result<Self> {
        match meta {
            // Neither generic parameters nor a `where` clause can appear in
            // the expression after `=`, so they are written as `with(...)`.
            Meta::List(list) => Ok(list.parse_args()?),
            Meta::NameValue(value) => Self::from_expr(&value.value),
            Meta::Path(_) => Self::from_word(),
        }
        .map_err(|error| error.with_span(meta))
    }

    fn from_expr(expr: &Expr) -> darling::Result<Self> {
        match expr {
            // The historic `with = "|x| ..."` string spelling stays supported;
            // the string contents must satisfy the same grammar.
            Expr::Lit(ExprLit {
                lit: Lit::Str(lit), ..
            }) => Ok(lit.parse()?),
            // Invisible groups appear when the attribute is forwarded through
            // a `macro_rules!` fragment.
            Expr::Group(group) => Self::from_expr(&group.expr),
            _ => Ok(parse2(expr.to_token_stream())?),
        }
    }
}

impl ToTokens for PseudoClosure {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            generics,
            open,
            param,
            close,
            body,
        } = self;
        let where_clause = &generics.where_clause;
        tokens.extend(quote! { #generics #open #param #close #where_clause #body });
    }
}

/// A type written directly or as a string, read with the `Parsed` grammar.
#[derive(Debug)]
pub(super) struct TypeArg<Parsed>(pub(super) Parsed);

impl<Parsed: Parse> FromMeta for TypeArg<Parsed> {
    fn from_expr(expr: &Expr) -> darling::Result<Self> {
        match expr {
            // The contents of a string must satisfy the same grammar.
            Expr::Lit(ExprLit {
                lit: Lit::Str(lit), ..
            }) => Ok(Self(lit.parse()?)),
            // Invisible groups appear when the attribute is forwarded through
            // a `macro_rules!` fragment.
            Expr::Group(group) => Self::from_expr(&group.expr),
            // Attribute values parse as expressions; read their tokens again
            // with the grammar this argument expects.
            _ => Ok(Self(parse2(expr.to_token_stream())?)),
        }
    }
}

#[derive(Debug, Default, Deref, FromAttributes)]
#[darling(default, attributes(refield))]
pub(super) struct StructArgs {
    #[deref]
    #[darling(flatten)]
    pub(super) general: GeneralArgs,
    #[darling(rename = "crate")]
    pub(super) krate: Option<Crate>,
}

#[derive(Debug, Default, FromMeta)]
#[darling(default)]
pub(super) struct AliasArgs {
    #[darling(rename = "crate")]
    pub(super) krate: Option<Crate>,
    #[darling(rename = "as")]
    pub(super) r#as: Option<TypeArg<Path>>,
}

#[derive(Debug, Default, FromMeta)]
#[darling(default)]
pub(super) struct ImplArgs {
    #[darling(rename = "crate")]
    pub(super) krate: Option<Crate>,
}

#[derive(Debug, Default, Deref, FromMeta)]
#[darling(default, and_then = Self::finalize)]
pub(super) struct FieldArgs {
    #[deref]
    #[darling(flatten)]
    pub(super) general: GeneralArgs,
    pub(super) skip: Flag,
    pub(super) default: Option<SpannedValue<Override<PreservedStrExpr>>>,
    #[darling(multiple, rename = "bound")]
    pub(super) bounds: Vec<Bounds>,
}

impl FieldArgs {
    fn finalize(self) -> darling::Result<Self> {
        if self.skip.is_present() && (self.name.is_some() || self.with.is_some()) {
            return Err(
                darling::Error::custom("`skip` can only be combined with `default`")
                    .with_span(&self.skip.span()),
            );
        }
        Ok(self)
    }
}
