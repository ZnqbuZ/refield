use proc_macro::TokenStream;
use quote::quote;

mod refield;

#[proc_macro_attribute]
pub fn refieldable(args: TokenStream, input: TokenStream) -> TokenStream {
    let input = input.into();
    refield::proc(args.into(), &input)
        .unwrap_or_else(|error| {
            let input = refield::strip(&input);
            let error = error.to_compile_error();
            quote! {
                #input
                #error
            }
        })
        .into()
}

#[proc_macro_derive(Refield, attributes(refield))]
pub fn derive(input: TokenStream) -> TokenStream {
    refield::derive(input.into())
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

#[proc_macro]
pub fn refield(input: TokenStream) -> TokenStream {
    refield::call(input.into())
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

#[doc(hidden)]
#[proc_macro]
pub fn construct(input: TokenStream) -> TokenStream {
    refield::construct(input.into())
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

#[doc(hidden)]
#[proc_macro]
pub fn alias(input: TokenStream) -> TokenStream {
    refield::alias(input.into())
        .unwrap_or_else(|error| error.to_compile_error())
        .into()
}

/// Second half of the carrier pipeline.
///
/// An `#[refieldable] impl` block and a construction with attributed fields
/// are placed in a carrier struct's derive input, so `rustc`'s built-in
/// cfg-stripping runs over them before this macro inspects them. What they
/// carry is emitted here; `discard` then removes the carrier.
#[doc(hidden)]
#[proc_macro_derive(Prepare, attributes(refield))]
pub fn prepare(input: TokenStream) -> TokenStream {
    let input = input.into();
    refield::prepare(&input)
        .unwrap_or_else(|error| {
            let input = refield::recover(&input);
            let error = error.to_compile_error();
            quote! {
                #input
                #error
            }
        })
        .into()
}

#[doc(hidden)]
#[proc_macro_attribute]
pub fn discard(_: TokenStream, _: TokenStream) -> TokenStream {
    TokenStream::new()
}
