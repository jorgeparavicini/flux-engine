use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

#[proc_macro_derive(Component, attributes(component))]
pub fn derive_component(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    if !input.generics.params.is_empty() {
        return syn::Error::new_spanned(&input.generics, "#[derive(Component)] does not support generic types: the component key is derived from the type name, so every instantiation would share one key")
            .to_compile_error()
            .into();
    }

    let mut non_send = false;
    for attr in input.attrs {
        if !attr.path().is_ident("component") {
            continue;
        }
        let result = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("non_send") {
                non_send = true;
                Ok(())
            } else {
                Err(meta.error("unknown `component` attribute; expected `non_send`"))
            }
        });

        if let Err(e) = result {
            return e.to_compile_error().into();
        }
    }

    let name = &input.ident;
    let name_str = name.to_string();
    let assertion = if non_send {
        quote!()
    } else {
        quote! {
            const _: () = {
                const fn assert_send_sync<T: ::flux_ecs2::ThreadSafeComponent>() {}
                assert_send_sync::<#name>();
            };
        }
    };

    quote! {
        #[automatically_derived]
        impl ::flux_ecs2::Component for #name {
            const KEY: ::flux_ecs2::ComponentKey = ::flux_ecs2::ComponentKey::from_path(concat!(module_path!(), "::", #name_str));
            const NON_SEND: bool = #non_send;
        }
        #assertion
    }
        .into()
}
