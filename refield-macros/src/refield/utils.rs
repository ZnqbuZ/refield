use proc_macro2::{Delimiter, Group, Ident, Spacing, Span, TokenStream, TokenTree};
use quote::quote;
use std::collections::BTreeMap;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Mutex;
use syn::ext::IdentExt;
use syn::visit::{self, Visit};
use syn::{
    Attribute, Data, DeriveInput, Error, Expr, Fields, ImplItem, Item, Lifetime, LitStr, Member,
    Meta, Path, PathArguments, PathSegment, Result, Stmt, Type, TypeImplTrait, TypeInfer, TypePath,
    TypeReference, UnOp,
};
use unicode_ident::is_xid_continue;

macro_rules! parse {
    ($input:ident => { $($rest:tt)* }) => {{
        let outer;
        syn::braced!(outer in $input);
        parse!(@body outer () $($rest)*)
    }};
    ($input:ident => ( $($rest:tt)* )) => {{
        let outer;
        syn::parenthesized!(outer in $input);
        parse!(@body outer () $($rest)*)
    }};
    ($input:ident => $($rest:tt)*) => {{
        parse!(@body $input () $($rest)*)
    }};

    (@body $stream:ident ($($names:ident)*) $(,)?) => {{
        let _ = $stream.parse::<syn::Token![,]>();
        Ok::<Self, syn::Error>(Self { $($names),* })
    }};
    (@body $stream:ident ($($names:ident)*) , $($rest:tt)+) => {{
        $stream.parse::<syn::Token![,]>()?;
        parse!(@body $stream ($($names)*) $($rest)+)
    }};
    (@body $stream:ident ($($names:ident)*) ($name:ident : $($ty:tt)+) $($rest:tt)*) => {{
        let content;
        syn::parenthesized!(content in $stream);
        let $name = parse!(@val content, $($ty)+);
        parse!(@body $stream ($($names)* $name) $($rest)*)
    }};
    (@body $stream:ident ($($names:ident)*) [$name:ident : $($ty:tt)+] $($rest:tt)*) => {{
        let content;
        syn::bracketed!(content in $stream);
        let $name = parse!(@val content, $($ty)+);
        parse!(@body $stream ($($names)* $name) $($rest)*)
    }};
    (@body $stream:ident ($($names:ident)*) {$name:ident : $($ty:tt)+} $($rest:tt)*) => {{
        let content;
        syn::braced!(content in $stream);
        let $name = parse!(@val content, $($ty)+);
        parse!(@body $stream ($($names)* $name) $($rest)*)
    }};
    (@body $stream:ident ($($names:ident)*) $name:ident : $ty:ident $(<$inner:ident>)? $($rest:tt)*) => {{
        let $name = parse!(@val $stream, $ty $(<$inner>)?);
        parse!(@body $stream ($($names)* $name) $($rest)*)
    }};

    (@val $stream:ident, bool) => {
        $stream.parse::<syn::LitBool>()?.value
    };
    (@val $stream:ident, Ident) => {
        <proc_macro2::Ident as syn::ext::IdentExt>::parse_any(&$stream)?
    };
    (@val $stream:ident, Option<$ty:ident>) => {
        if $stream.is_empty() {
            None
        } else {
            Some(parse!(@val $stream, $ty))
        }
    };
    (@val $stream:ident, Vec<Path>) => {
        syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated_with(
            &$stream,
            syn::Path::parse_mod_style,
        )?
        .into_iter()
        .collect()
    };
    (@val $stream:ident, Vec<$ty:ident>) => {
        syn::punctuated::Punctuated::<$ty, syn::Token![,]>::parse_terminated(&$stream)?
            .into_iter()
            .collect()
    };
    (@val $stream:ident, $ty:ident) => {
        $stream.parse::<$ty>()?
    };
}

pub(super) use parse;

// Tokens written by this macro only borrow the location of `$span`, so
// diagnostics still point at the user's code. They keep the macro's context:
// they follow this crate's edition whatever the caller's is, and lints treat
// them as generated code. Interpolated user tokens keep their own spans.
macro_rules! qs {
    ($span:expr => $($tt:tt)*) => {
        ::quote::quote_spanned! { ::proc_macro2::Span::mixed_site().located_at($span) => $($tt)* }
    };
}

macro_rules! pqs {
    ($span:expr => $($tt:tt)*) => {
        ::syn::parse_quote_spanned! { ::proc_macro2::Span::mixed_site().located_at($span) => $($tt)* }
    };
}

macro_rules! span {
    ($span:expr) => {
        $crate::refield::utils::span!(@impl $span, $)
    };

    (@impl $span:expr, $_:tt) => {
        #[allow(unused_macros)]
        macro_rules! q {
            ($_($_ tt:tt)*) => {
                $crate::refield::utils::qs! { $span => $_($_ tt)* }
            };
        }
        #[allow(unused_macros)]
        macro_rules! pq {
            ($_($_ tt:tt)*) => {
                $crate::refield::utils::pqs! { $span => $_($_ tt)* }
            };
        }
    };
}

pub(super) use {pqs, qs, span};

/// Accumulate `error` into `errors`, so a single pass reports every
/// offending attribute.
pub(super) fn combine(errors: &mut Option<Error>, error: Error) {
    match errors {
        Some(errors) => errors.combine(error),
        None => *errors = Some(error),
    }
}

pub(super) fn is_refield(attr: &Attribute) -> bool {
    attr.path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "refield")
}

/// Whether `path` is exactly `Self`, which only an annotated `impl` context
/// can resolve.
pub(super) fn is_self(path: &Path) -> bool {
    path.segments.len() == 1 && path.segments[0].ident == "Self"
}

pub(super) fn is_place(expr: &Expr) -> bool {
    match expr {
        Expr::Path(path) => {
            path.qself.is_none()
                && path.path.leading_colon.is_none()
                && path.path.segments.len() == 1
                && path.path.segments[0].arguments.is_none()
        }
        Expr::Field(field) => is_place(&field.base),
        Expr::Unary(unary) if matches!(unary.op, UnOp::Deref(_)) => is_place(&unary.expr),
        Expr::Paren(paren) => is_place(&paren.expr),
        Expr::Group(group) => is_place(&group.expr),
        _ => false,
    }
}

/// Whether two profile paths are the same, ignoring raw-identifier prefixes.
pub(super) fn is_same(a: &Path, b: &Path) -> bool {
    a.leading_colon.is_some() == b.leading_colon.is_some()
        && a.segments.len() == b.segments.len()
        && a.segments.iter().zip(&b.segments).all(|(a, b)| {
            a.ident.unraw() == b.ident.unraw() && a.arguments.is_none() && b.arguments.is_none()
        })
}

pub(super) fn member_to_string(member: &Member) -> String {
    match member {
        Member::Named(ident) => ident.unraw().to_string(),
        Member::Unnamed(index) => index.index.to_string(),
    }
}

pub(super) fn path_to_string(path: &Path) -> String {
    let mut string = String::new();
    if path.leading_colon.is_some() {
        string.push_str("::");
    }
    for (index, segment) in path.segments.iter().enumerate() {
        if index > 0 {
            string.push_str("::");
        }
        string.push_str(&segment.ident.unraw().to_string());
    }
    string
}

/// The member a `name` pattern gives for `target`, with `{}` replaced by the
/// target's name.
pub(super) fn format(pattern: &LitStr, target: &Member) -> Result<Member> {
    let value = pattern.value().replace("{}", &member_to_string(target));
    LitStr::new(&value, pattern.span()).parse().map_err(|_| {
        Error::new(
            pattern.span(),
            format!(
                "invalid source field name `{value}`; expected a field identifier or tuple index"
            ),
        )
    })
}

/// The companion macro path of a nominal type, or `message` as the error if
/// the type does not name a struct or type alias by path.
pub(super) fn nominal(ty: &Type, message: &str) -> Result<Path> {
    let Type::Path(TypePath {
        qself: None, path, ..
    }) = ty
    else {
        return Err(Error::new_spanned(ty, message));
    };
    if let Some(segment) = associated(path) {
        return Err(Error::new_spanned(segment, message));
    }
    if is_self(path) {
        return Err(Error::new(path.segments[0].ident.span(), message));
    }
    Ok(bare(path))
}

/// Validate a schema path written after `as`: it must name a concrete schema,
/// neither `Self` nor an associated type. The path keeps its generic
/// arguments for the struct literal; [`bare`] gives its companion macro.
pub(super) fn concrete(path: Path) -> Result<Path> {
    if is_self(&path) {
        return Err(Error::new(
            path.segments[0].ident.span(),
            "cannot resolve struct schema from `Self` alone; use a concrete schema path after `as`",
        ));
    }
    if let Some(segment) = associated(&path) {
        return Err(Error::new_spanned(
            segment,
            "schema path after `as` cannot be an associated type",
        ));
    }
    Ok(path)
}

/// The first segment before the final one that carries generic arguments.
///
/// Such a path names an associated type rather than a type that owns a
/// companion macro, so no caller may forward it unchanged.
pub(super) fn associated(path: &Path) -> Option<&PathSegment> {
    path.segments
        .iter()
        .take(path.segments.len().saturating_sub(1))
        .find(|segment| !segment.arguments.is_none())
}

/// The companion macro path for a nominal path: the final segment's generic
/// arguments are stripped, everything else is kept.
pub(super) fn bare(path: &Path) -> Path {
    let mut path = path.clone();
    if let Some(last) = path.segments.last_mut() {
        last.arguments = PathArguments::None;
    }
    path
}

/// Add the turbofish `::<>` that generic arguments need in expression
/// position.
///
/// Only segments after a qualified path's `as` boundary are touched: the
/// `Type<T>` inside `<Type<T> as Trait>` is type syntax and must stay as is.
pub(super) fn turbofish(mut ty: TypePath) -> TypePath {
    let start = ty.qself.as_ref().map_or(0, |qself| qself.position);
    for segment in ty.path.segments.iter_mut().skip(start) {
        if let PathArguments::AngleBracketed(arguments) = &mut segment.arguments {
            arguments.colon2_token.get_or_insert_with(Default::default);
        }
    }
    ty
}

/// Turn an expression into the body block of a generated method.
///
/// The statements of an unlabelled block are spliced into the method body
/// instead of the block itself, which would otherwise nest one block inside
/// another and trip `unused_braces`.
pub(super) fn block(expr: &Expr) -> TokenStream {
    match expr {
        Expr::Block(block) if block.attrs.is_empty() && block.label.is_none() => {
            let stmts = &block.block.stmts;
            quote! { #(#stmts)* }
        }
        _ => quote! { #expr },
    }
}

/// Whether a type mentions a lifetime that cannot be written in a `where`
/// predicate at the helper's position: an elided lifetime (`&T`, `&'_ T`, and
/// the same behind any path generic argument such as `Cow<'_, str>`), an
/// inferred type (`_`), or an `impl Trait`/anonymous parameter.
///
/// A `Sized` predicate that mentions one of those cannot be spelled in the
/// helper's `where` clause, so such types are left unconstrained.
struct UnnameableTypes {
    found: bool,
}

impl<'ast> Visit<'ast> for UnnameableTypes {
    fn visit_lifetime(&mut self, lifetime: &'ast Lifetime) {
        if lifetime.ident == "_" {
            self.found = true;
        }
        visit::visit_lifetime(self, lifetime);
    }

    fn visit_type_impl_trait(&mut self, ty: &'ast TypeImplTrait) {
        self.found = true;
        visit::visit_type_impl_trait(self, ty);
    }

    fn visit_type_infer(&mut self, _: &'ast TypeInfer) {
        self.found = true;
    }

    fn visit_type_reference(&mut self, ty: &'ast TypeReference) {
        if ty
            .lifetime
            .as_ref()
            .is_none_or(|lifetime| lifetime.ident == "_")
        {
            self.found = true;
        }
        visit::visit_type_reference(self, ty);
    }
}

/// The `Sized` predicate a helper needs for one of its value positions.
///
/// The bound is attached to the generated method rather than to the struct,
/// so `struct Target<T: ?Sized> { value: T }` still derives cleanly.
pub(super) fn sized(ty: &Type) -> Option<TokenStream> {
    if matches!(ty, Type::Reference(_)) {
        return None;
    }
    let mut visitor = UnnameableTypes { found: false };
    visitor.visit_type(ty);
    (!visitor.found).then(|| quote! { #ty: ::core::marker::Sized })
}

/// Replace every `$` in `tokens` by the callback's `$` metavariable.
///
/// The whole final expression is rewritten, including `..rest` and every
/// field initializer, so user-written `macro_rules!` definitions and
/// invocations keep their dollars once the callback expands. Groups keep
/// their delimiter and span; every other token is cloned unchanged.
pub(super) fn escape(tokens: TokenStream, dollar: &Ident) -> TokenStream {
    tokens
        .into_iter()
        .map(|tree| match tree {
            TokenTree::Punct(punct) if punct.as_char() == '$' => {
                [TokenTree::Punct(punct), TokenTree::Ident(dollar.clone())]
                    .into_iter()
                    .collect()
            }
            TokenTree::Group(group) => {
                let mut escaped = Group::new(group.delimiter(), escape(group.stream(), dollar));
                escaped.set_span(group.span());
                TokenStream::from(TokenTree::Group(escaped))
            }
            tree => TokenStream::from(tree),
        })
        .collect()
}

/// Keep only the attributes `keep` accepts, on the item, its fields, and its
/// nested items.
pub(super) fn retain(item: &mut Item, mut keep: impl FnMut(&Attribute) -> bool) {
    let attrs = match item {
        Item::Const(item) => &mut item.attrs,
        Item::Enum(item) => {
            for variant in &mut item.variants {
                variant.attrs.retain(&mut keep);
                for field in variant.fields.iter_mut() {
                    field.attrs.retain(&mut keep);
                }
            }
            &mut item.attrs
        }
        Item::ExternCrate(item) => &mut item.attrs,
        Item::Fn(item) => &mut item.attrs,
        Item::ForeignMod(item) => &mut item.attrs,
        Item::Impl(item) => {
            for item in &mut item.items {
                let attrs = match item {
                    ImplItem::Const(item) => &mut item.attrs,
                    ImplItem::Fn(item) => &mut item.attrs,
                    ImplItem::Type(item) => &mut item.attrs,
                    ImplItem::Macro(item) => &mut item.attrs,
                    _ => continue,
                };
                attrs.retain(&mut keep);
            }
            &mut item.attrs
        }
        Item::Macro(item) => &mut item.attrs,
        Item::Mod(item) => &mut item.attrs,
        Item::Static(item) => &mut item.attrs,
        Item::Struct(item) => {
            for field in item.fields.iter_mut() {
                field.attrs.retain(&mut keep);
            }
            &mut item.attrs
        }
        Item::Trait(item) => &mut item.attrs,
        Item::TraitAlias(item) => &mut item.attrs,
        Item::Type(item) => &mut item.attrs,
        Item::Union(item) => {
            for field in &mut item.fields.named {
                field.attrs.retain(&mut keep);
            }
            &mut item.attrs
        }
        Item::Use(item) => &mut item.attrs,
        _ => return,
    };
    attrs.retain(keep);
}

/// The statements in a carrier struct's array-length block; none if the
/// input is not shaped like a carrier.
pub(super) fn carried(input: &DeriveInput) -> &[Stmt] {
    if let Data::Struct(data) = &input.data
        && let Fields::Unnamed(fields) = &data.fields
        && let Some(field) = fields.unnamed.first()
        && let Type::Array(array) = &field.ty
        && let Expr::Block(block) = &array.len
    {
        &block.block.stmts
    } else {
        &[]
    }
}

/// The tokens inside the `__construct(...)` group of a construction carrier,
/// if any.
pub(super) fn metadata(input: &DeriveInput) -> Option<TokenStream> {
    input.attrs.iter().find_map(|attr| {
        if is_refield(attr)
            && let Meta::List(list) = &attr.meta
            && let mut tokens = list.tokens.clone().into_iter()
            && let Some(TokenTree::Ident(ident)) = tokens.next()
            && ident == "__construct"
            && let Some(TokenTree::Group(group)) = tokens.next()
            && group.delimiter() == Delimiter::Parenthesis
            && tokens.next().is_none()
        {
            Some(group.stream())
        } else {
            None
        }
    })
}

// region id

static COUNTERS: Mutex<BTreeMap<u64, usize>> = Mutex::new(BTreeMap::new());

trait HashExt {
    fn hash(&self, hasher: &mut impl Hasher);
}

impl HashExt for Span {
    fn hash(&self, hasher: &mut impl Hasher) {
        self.file().hash(hasher);
        self.start().hash(hasher);
        self.end().hash(hasher);
    }
}

impl HashExt for Delimiter {
    fn hash(&self, hasher: &mut impl Hasher) {
        let tag = match self {
            Delimiter::Parenthesis => 0u8,
            Delimiter::Brace => 1,
            Delimiter::Bracket => 2,
            Delimiter::None => 3,
        };
        tag.hash(hasher);
    }
}

impl HashExt for Spacing {
    fn hash(&self, hasher: &mut impl Hasher) {
        let tag = match self {
            Spacing::Alone => 0u8,
            Spacing::Joint => 1,
        };
        tag.hash(hasher);
    }
}

impl HashExt for TokenTree {
    fn hash(&self, hasher: &mut impl Hasher) {
        self.span().hash(hasher);
        match self {
            TokenTree::Group(group) => {
                0u8.hash(hasher);
                group.delimiter().hash(hasher);
                group.span_open().hash(hasher);
                group.span_close().hash(hasher);
                group.stream().hash(hasher);
            }
            TokenTree::Ident(ident) => {
                1u8.hash(hasher);
                ident.to_string().hash(hasher);
            }
            TokenTree::Punct(punct) => {
                2u8.hash(hasher);
                punct.as_char().hash(hasher);
                punct.spacing().hash(hasher);
            }
            TokenTree::Literal(literal) => {
                3u8.hash(hasher);
                literal.to_string().hash(hasher);
            }
        }
    }
}

impl HashExt for Ident {
    fn hash(&self, hasher: &mut impl Hasher) {
        self.to_string().hash(hasher);
        self.span().hash(hasher);
    }
}

impl HashExt for TokenStream {
    fn hash(&self, hasher: &mut impl Hasher) {
        for tree in self.clone() {
            1u8.hash(hasher);
            tree.hash(hasher);
        }
        0u8.hash(hasher);
    }
}

pub(super) struct UniqueId {
    name: Ident,
    hash: u64,
    seq: usize,
    file: String,
    line: usize,
    column: usize,
}

impl UniqueId {
    pub(super) fn new(ident: &Ident, tokens: &TokenStream) -> Self {
        let span = Span::call_site();
        let hash = {
            let mut hasher = DefaultHasher::new();
            HashExt::hash(ident, &mut hasher);
            HashExt::hash(tokens, &mut hasher);
            HashExt::hash(&span, &mut hasher);
            hasher.finish()
        };
        let seq = {
            let mut counters = COUNTERS.lock().unwrap();
            *counters
                .entry(hash)
                .and_modify(|seq| *seq += 1)
                .or_insert(0)
        };
        let start = span.start();
        Self {
            name: ident.unraw(),
            hash,
            seq,
            file: span.file(),
            line: start.line,
            column: start.column,
        }
    }
}

impl Display for UniqueId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}__IN__{}__AT__L{}_C{}__{:016x}_{:x}",
            self.name,
            self.file
                .chars()
                .map(|c| if is_xid_continue(c) { c } else { '_' })
                .collect::<String>(),
            self.line,
            self.column,
            self.hash,
            self.seq
        )
    }
}

// endregion
