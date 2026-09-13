use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

/// Derives `Component`, giving the type a stable identity derived from its
/// module path and name.
///
/// The type must be `Send + Sync` unless it opts out:
///
/// - `#[component(non_send)]` — the component is confined to the thread that
///   created it (for types like window or GPU handles).
///
/// Generic types are not supported: every instantiation would share one
/// identity.
#[proc_macro_derive(Component, attributes(component))]
pub fn derive_component(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    if !input.generics.params.is_empty() {
        return syn::Error::new_spanned(&input.generics, "#[derive(Component)] does not support generic types: the component key is derived from the type name, so every instantiation would share one key")
            .to_compile_error()
            .into();
    }

    let mut non_send = false;
    let mut toggleable = false;
    for attr in input.attrs {
        if !attr.path().is_ident("component") {
            continue;
        }
        let result = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("non_send") {
                non_send = true;
                Ok(())
            } else if meta.path.is_ident("toggleable") {
                toggleable = true;
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
                const fn assert_send_sync<T: ::flux_ecs::ThreadSafeComponent>() {}
                assert_send_sync::<#name>();
            };
        }
    };

    quote! {
        #[automatically_derived]
        impl ::flux_ecs::Component for #name {
            const KEY: ::flux_ecs::ComponentKey = ::flux_ecs::ComponentKey::from_path(concat!(module_path!(), "::", #name_str));
            const NON_SEND: bool = #non_send;
            const TOGGLEABLE: bool = #toggleable;
        }
        #assertion
    }
        .into()
}

/// Derives `SystemParam` for a struct of system parameters, so related
/// requests can be grouped under named fields.
///
/// The struct's lifetimes must be named `'w` (world data) and/or `'s`
/// (system state), in that order, and every field must itself be a system
/// parameter written with those lifetimes:
///
/// ```ignore
/// #[derive(SystemParam)]
/// struct Gpu<'w> {
///     device: Single<'w, &'w Device>,
///     swapchain: Single<'w, &'w Swapchain>,
/// }
/// ```
///
/// Generic type parameters are not supported.
#[proc_macro_derive(SystemParam)]
pub fn derive_system_param(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let mut lifetimes: Vec<String> = Vec::new();
    for param in &input.generics.params {
        match param {
            syn::GenericParam::Lifetime(l) => lifetimes.push(l.lifetime.ident.to_string()),
            other => {
                return syn::Error::new_spanned(
                    other,
                    "`#[derive(SystemParam)]` supports lifetime parameters only",
                )
                .to_compile_error()
                .into();
            }
        }
    }
    let allowed = [
        vec![],
        vec!["w".to_string()],
        vec!["s".to_string()],
        vec!["w".to_string(), "s".to_string()],
    ];
    if !allowed.contains(&lifetimes) {
        return syn::Error::new_spanned(
            &input.generics,
            "`#[derive(SystemParam)]` requires lifetimes named 'w and/or 's, in that order",
        )
        .to_compile_error()
        .into();
    }

    let syn::Data::Struct(data) = &input.data else {
        return syn::Error::new_spanned(&input, "`#[derive(SystemParam)]` requires a struct")
            .to_compile_error()
            .into();
    };
    let syn::Fields::Named(fields) = &data.fields else {
        return syn::Error::new_spanned(&input, "`#[derive(SystemParam)]` requires named fields")
            .to_compile_error()
            .into();
    };

    /// Rewrites every lifetime in a type to `'static`: parameter access,
    /// state, and initialization are lifetime-independent.
    struct Erase;
    impl syn::visit_mut::VisitMut for Erase {
        fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
            lifetime.ident = syn::Ident::new("static", lifetime.ident.span());
        }
    }

    let field_names: Vec<_> = fields
        .named
        .iter()
        .map(|f| f.ident.clone().unwrap())
        .collect();
    let erased: Vec<syn::Type> = fields
        .named
        .iter()
        .map(|f| {
            let mut ty = f.ty.clone();
            syn::visit_mut::VisitMut::visit_type_mut(&mut Erase, &mut ty);
            ty
        })
        .collect();

    // Self, lifetime-erased, for the impl header; Item substitutes fresh ones.
    let erased_self_args = lifetimes.iter().map(|_| quote!('_));
    let item_args = lifetimes
        .iter()
        .map(|l| if l == "w" { quote!('w2) } else { quote!('s2) });
    let state_indices = (0..field_names.len())
        .map(syn::Index::from)
        .collect::<Vec<_>>();

    quote! {
        unsafe impl ::flux_ecs::SystemParam for #name<#(#erased_self_args),*> {
            const ACCESS: ::flux_ecs::AccessList = {
                let list = ::flux_ecs::AccessList::EMPTY;
                #( let list = list.concat(<#erased as ::flux_ecs::SystemParam>::ACCESS); )*
                list
            };
            type State = (#(<#erased as ::flux_ecs::SystemParam>::State,)*);
            type Item<'w2, 's2> = #name<#(#item_args),*>;

            fn init(world: &mut ::flux_ecs::World) -> Self::State {
                (#(<#erased as ::flux_ecs::SystemParam>::init(world),)*)
            }

            unsafe fn fetch<'w2, 's2>(
                state: &'s2 mut Self::State,
                cells: &::flux_ecs::WorldCells<'w2>,
                version: u64,
            ) -> Self::Item<'w2, 's2> {
                #name {
                    #(#field_names: unsafe {
                        <#erased as ::flux_ecs::SystemParam>::fetch(&mut state.#state_indices, cells, version)
                    },)*
                }
            }

            fn apply(state: &mut Self::State, world: &mut ::flux_ecs::World) {
                #(<#erased as ::flux_ecs::SystemParam>::apply(&mut state.#state_indices, world);)*
            }
        }
    }
    .into()
}
