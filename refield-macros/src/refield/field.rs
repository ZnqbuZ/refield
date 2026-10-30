use darling::FromMeta;
use darling::ast::NestedMeta;
use darling::util::{Override, PathList, parse_attribute_to_meta_list};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{ToTokens, format_ident, quote};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{
    Error, Expr, Field, Fields, Index, Member, Pat, Path, Result, ReturnType, Token, Type, UnOp,
};

use super::args::{FieldArgs, FunctionArg, StructArgs};
use super::codegen::Helper;
use super::utils::{
    block, combine, format, is_refield, is_same, member_to_string, parse, path_to_string, qs, span,
};

/// How a missing field value is produced.
#[derive(Clone)]
pub(super) enum DefaultRule {
    /// No default value.
    None,
    /// Bare `default` or `skip` without an explicit expression: expands inline to `Default::default()`.
    Trait(Span),
    /// Custom `default = expr`: names the generated helper method.
    Method(Ident),
}

impl Parse for DefaultRule {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        if input.is_empty() {
            return Ok(Self::None);
        }
        if input.peek(Token![fn]) {
            input.parse::<Token![fn]>()?;
            return Ok(Self::Method(Ident::parse_any(input)?));
        }
        let keyword = input.parse::<Ident>()?;
        if keyword != "default" {
            return Err(Error::new(keyword.span(), "expected `default` or `fn`"));
        }
        Ok(Self::Trait(keyword.span()))
    }
}

impl ToTokens for DefaultRule {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Self::None => {}
            Self::Trait(span) => tokens.extend(qs! { *span => default }),
            Self::Method(ident) => tokens.extend(qs! { ident.span() => fn #ident }),
        }
    }
}

/// Extraction and conversion rule for one target field.
#[derive(Clone)]
pub(super) struct FieldRule {
    pub(super) source: Member,
    pub(super) skip: bool,
    pub(super) with: Option<Ident>,
    pub(super) default: DefaultRule,
}

impl FieldRule {
    /// Build a rule from one merged [`FieldArgs`] group.
    ///
    /// `base` supplies the struct-level defaults: `name` renames every
    /// automatic source that has no field-level `name`, and `with` describes
    /// the conversion of every automatic field that does not override it.
    ///
    /// A `with` pseudo-closure or function path and a custom `default`
    /// expression each become an inherent helper method on the target struct,
    /// pushed to `helpers`, so they evaluate in the definition site's scope.
    fn new(
        args: FieldArgs,
        target: &Member,
        field: &Field,
        index: usize,
        base: &StructArgs,
        check: &Ident,
        helpers: &mut Vec<TokenStream>,
    ) -> Result<Self> {
        let skip = args.skip.is_present();
        let source = match (&args.name, &base.name) {
            (Some(name), _) => format(name, target)?,
            (None, Some(name)) if !skip => format(name, target)?,
            _ => target.clone(),
        };
        let konst = args.konst.is_present() || base.konst.is_present();
        let name = member_to_string(target);
        let bounds = args
            .bounds
            .iter()
            .flat_map(|bounds| bounds.iter().cloned())
            .collect::<Vec<_>>();

        // A field-level `with` replaces the struct-level one. The effective
        // expression and the helper name are decided once for both spellings.
        let with = if skip {
            None
        } else {
            args.with.as_ref().or(base.with.as_ref())
        };
        let with = with.map(|with| {
            // The helper's name points at the user's expression in diagnostics,
            // but carries the macro's hygiene, so lints on the name itself are
            // not reported against user code.
            let ident = format_ident!(
                "{check}_with_{name}_{index}",
                span = Span::mixed_site().located_at(with.span())
            );
            let (pat, input, output, body) = match with {
                FunctionArg::PseudoClosure(closure) => {
                    let param = closure
                        .inputs
                        .first()
                        .expect("closure parameter count is validated while parsing `with`");
                    // An untyped parameter, or `_` as its type, takes the field type.
                    let (pat, input) = match param {
                        Pat::Type(param) => {
                            let input = match &*param.ty {
                                Type::Infer(_) => field.ty.clone(),
                                ty => ty.clone(),
                            };
                            ((*param.pat).clone(), input)
                        }
                        pat => (pat.clone(), field.ty.clone()),
                    };
                    // The closure's declared return type is part of its signature,
                    // so it becomes the helper's return type. A mismatch with the
                    // field is then reported by the compiler instead of being
                    // discarded.
                    let output = match &closure.output {
                        ReturnType::Type(_, ty) if !matches!(**ty, Type::Infer(_)) => {
                            (**ty).clone()
                        }
                        _ => field.ty.clone(),
                    };
                    (pat, input, output, block(&closure.body))
                }
                FunctionArg::Function(path) => {
                    // A function path is called with the extracted value; its
                    // signature is not guessed, so both ends use the field type.
                    span!(path.span());
                    let value = format_ident!("__refield_value", span = Span::mixed_site());
                    (
                        pq! { #value },
                        field.ty.clone(),
                        field.ty.clone(),
                        q! { (#path)(#value) },
                    )
                }
            };
            helpers.push(
                Helper {
                    vis: &field.vis,
                    ident: &ident,
                    input: Some((&pat, &input)),
                    output: &output,
                    body,
                    bounds: &bounds,
                    konst,
                }
                .into_token_stream(),
            );
            ident
        });

        let default = match &args.default {
            Some(default) => match &**default {
                Override::Inherit => DefaultRule::Trait(default.span()),
                Override::Explicit(expr) => {
                    let expr = &expr.0;
                    let ident = format_ident!(
                        "{check}_default_{name}_{index}",
                        span = Span::mixed_site().located_at(expr.span())
                    );
                    // A literal, possibly negated, can always be `const`.
                    let konst = konst
                        || match expr {
                            Expr::Lit(_) => true,
                            Expr::Unary(unary) => {
                                matches!(unary.op, UnOp::Neg(_) | UnOp::Not(_))
                                    && matches!(&*unary.expr, Expr::Lit(_))
                            }
                            _ => false,
                        };
                    helpers.push(
                        Helper {
                            vis: &field.vis,
                            ident: &ident,
                            input: None,
                            output: &field.ty,
                            body: block(expr),
                            bounds: &bounds,
                            konst,
                        }
                        .into_token_stream(),
                    );
                    DefaultRule::Method(ident)
                }
            },
            None if skip => DefaultRule::Trait(args.skip.span()),
            None => DefaultRule::None,
        };

        Ok(Self {
            source,
            skip,
            with,
            default,
        })
    }
}

impl Parse for FieldRule {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        parse!(input => {
            source: Member,
            skip: bool,
            (with: Option<Ident>),
            (default: DefaultRule),
        })
    }
}

impl ToTokens for FieldRule {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            source,
            skip,
            with,
            default,
        } = self;
        tokens.extend(quote! { { #source, #skip, (#with), (#default) } });
    }
}

/// A list of profile names bound to the rule declared by one
/// `#[refield(...)]` attribute.
///
/// One attribute produces exactly one rule, no matter how many profiles it
/// lists, so the rule — and the helper method generated for it — is never
/// duplicated.
pub(super) struct ProfileRule {
    profiles: Vec<Path>,
    rule: FieldRule,
}

impl Parse for ProfileRule {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        parse!(input => (
            [profiles: Vec<Path>],
            rule: FieldRule,
        ))
    }
}

impl ToTokens for ProfileRule {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self { profiles, rule } = self;
        tokens.extend(quote! { ([ #(#profiles),* ], #rule) });
    }
}

/// One target field as carried by the companion macro's schema.
///
/// The schema only needs the target member, the default rule, and the profile
/// rules; the field's type, visibility, and span were used while the helper
/// methods were generated and are not retained.
pub(super) struct FieldIr {
    pub(super) target: Member,
    pub(super) rule: FieldRule,
    profiles: Vec<ProfileRule>,
    /// Whether the target field carries `#[deprecated]`. The definition site is
    /// the only place this can be observed: at the call site the macro would
    /// have to write the field name to read it, which is exactly the lint.
    pub(super) deprecated: bool,
}

impl FieldIr {
    /// The rule declared for `profile`, if the field has one.
    pub(super) fn profile(&self, profile: &Path) -> Option<&FieldRule> {
        self.profiles
            .iter()
            .find(|rule| {
                rule.profiles
                    .iter()
                    .any(|candidate| is_same(candidate, profile))
            })
            .map(|rule| &rule.rule)
    }

    /// Extract the schema of every field and the helper methods their rules
    /// generate.
    ///
    /// A custom `with` / `default` expression becomes an inherent helper
    /// method on the target struct, so it evaluates in the definition site's
    /// scope. The helper tokens are emitted once per rule, right where the
    /// rule is built.
    pub(super) fn extract(
        fields: &Fields,
        base: &StructArgs,
        check: &Ident,
    ) -> Result<(Vec<Self>, Vec<TokenStream>)> {
        let mut result = Vec::new();
        let mut helpers = Vec::new();
        let mut errors = None;

        // A unit struct has no field to validate the struct-level `name`
        // pattern against, but an invalid pattern is still a user error.
        if fields.is_empty()
            && let Some(name) = &base.name
            && let Err(error) = format(
                name,
                &Member::Named(format_ident!("__refield", span = name.span())),
            )
        {
            combine(&mut errors, error);
        }

        for (index, field) in fields.iter().enumerate() {
            let ir = (|| -> Result<Self> {
                let target = field.ident.clone().map(Member::Named).unwrap_or_else(|| {
                    Index {
                        index: index as u32,
                        span: field.ty.span(),
                    }
                    .into()
                });

                // Every `#[refield(...)]` attribute contributes its non-`on`
                // items at most once. The attributes without `on` make up the
                // default rule; a profile only records which attributes name
                // it, and the profiles named by exactly the same attributes
                // share one rule.
                let (defaults, profiles) = {
                    let mut errors = None;
                    let mut items = Vec::new();
                    let mut defaults = Vec::new();
                    let mut buckets = Vec::<(Path, Vec<usize>)>::new();
                    for attr in field.attrs.iter().filter(|attr| is_refield(attr)) {
                        let list = match parse_attribute_to_meta_list(attr) {
                            Ok(list) => list,
                            Err(error) => {
                                combine(&mut errors, error.into());
                                continue;
                            }
                        };
                        if list.tokens.is_empty() {
                            continue;
                        }
                        let metas = match NestedMeta::parse_meta_list(list.tokens) {
                            Ok(metas) => metas,
                            Err(error) => {
                                combine(&mut errors, error);
                                continue;
                            }
                        };
                        let mut on = Vec::new();
                        let mut rest = Vec::new();
                        for meta in metas {
                            match meta {
                                NestedMeta::Meta(meta) if meta.path().is_ident("on") => {
                                    on.push(meta);
                                }
                                meta => rest.push(meta),
                            }
                        }

                        let index = items.len();
                        items.push(rest);
                        let Some(first) = on.first() else {
                            defaults.push(index);
                            continue;
                        };
                        if let Some(second) = on.get(1) {
                            combine(
                                &mut errors,
                                darling::Error::duplicate_field("on")
                                    .with_span(second)
                                    .into(),
                            );
                        }
                        let paths = match PathList::from_meta(first) {
                            Ok(paths) => paths,
                            Err(error) => {
                                combine(&mut errors, error.into());
                                continue;
                            }
                        };
                        if paths.is_empty() {
                            combine(
                                &mut errors,
                                darling::Error::custom("`on` requires at least one profile name")
                                    .with_span(first)
                                    .into(),
                            );
                            continue;
                        }
                        let mut seen = Vec::<&Path>::new();
                        for path in paths.iter() {
                            if seen.iter().any(|seen| is_same(seen, path)) {
                                combine(
                                    &mut errors,
                                    darling::Error::custom(format!(
                                        "duplicate profile `{}`",
                                        path_to_string(path)
                                    ))
                                    .with_span(path)
                                    .into(),
                                );
                                continue;
                            }
                            seen.push(path);
                            match buckets.iter_mut().find(|(bucket, _)| is_same(bucket, path)) {
                                Some((_, indices)) => indices.push(index),
                                None => buckets.push((path.clone(), vec![index])),
                            }
                        }
                    }

                    // The items of the attributes at `indices`, merged.
                    let merge = |indices: &[usize]| {
                        indices
                            .iter()
                            .flat_map(|&index| items[index].iter().cloned())
                            .collect::<Vec<_>>()
                    };
                    let defaults = match FieldArgs::from_list(&merge(&defaults)) {
                        Ok(defaults) => Some(defaults),
                        Err(error) => {
                            combine(&mut errors, error.into());
                            None
                        }
                    };
                    let mut groups = Vec::<(&[usize], Vec<Path>)>::new();
                    for (path, indices) in &buckets {
                        match groups
                            .iter_mut()
                            .find(|(group, _)| *group == indices.as_slice())
                        {
                            Some((_, paths)) => paths.push(path.clone()),
                            None => groups.push((indices, vec![path.clone()])),
                        }
                    }
                    let mut profiles = Vec::new();
                    for (indices, paths) in groups {
                        match FieldArgs::from_list(&merge(indices)) {
                            Ok(args) => profiles.push((paths, args)),
                            Err(error) => combine(&mut errors, error.into()),
                        }
                    }

                    if let Some(errors) = errors {
                        return Err(errors);
                    }
                    (defaults.unwrap_or_default(), profiles)
                };

                let rule = FieldRule::new(defaults, &target, field, 0, base, check, &mut helpers)?;
                let profiles = profiles
                    .into_iter()
                    .enumerate()
                    .map(|(index, (profiles, args))| {
                        Ok(ProfileRule {
                            profiles,
                            rule: FieldRule::new(
                                args,
                                &target,
                                field,
                                index + 1,
                                base,
                                check,
                                &mut helpers,
                            )?,
                        })
                    })
                    .collect::<Result<_>>()?;
                // Only the path's last segment is compared, so renamed imports
                // of `deprecated` from `core` cannot hide it; a procedural macro
                // that injects the attribute downstream is still invisible here,
                // which conservatively means no `allow`.
                let deprecated = field.attrs.iter().any(|attr| {
                    attr.path()
                        .segments
                        .last()
                        .is_some_and(|segment| segment.ident == "deprecated")
                });

                Ok(Self {
                    target,
                    rule,
                    profiles,
                    deprecated,
                })
            })();

            match ir {
                Ok(ir) => result.push(ir),
                Err(error) => combine(&mut errors, error),
            }
        }

        match errors {
            Some(errors) => Err(errors),
            None => Ok((result, helpers)),
        }
    }
}

impl Parse for FieldIr {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        parse!(input => {
            target: Member,
            rule: FieldRule,
            [profiles: Vec<ProfileRule>],
            deprecated: bool,
        })
    }
}

impl ToTokens for FieldIr {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            target,
            rule,
            profiles,
            deprecated,
        } = self;
        tokens.extend(quote! { { #target, #rule, [ #(#profiles),* ], #deprecated } });
    }
}
