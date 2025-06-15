use proc_macro::TokenStream;
use quote::ToTokens;
use syn::parse::{Parse, ParseStream};
use syn::visit_mut::visit_item_fn_mut;
use syn::{
    parse_macro_input, parse_quote, visit_mut::VisitMut, Attribute, Block, Item, ItemFn, ItemMod,
    Path,
};

/// Represents a memory region enum variant.
struct RegionExpr {
    path: Path,
}

impl Parse for RegionExpr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Ok(RegionExpr {
            path: input.parse()?,
        })
    }
}

/// Visitor that wraps function bodies with region guards.
struct RegionVisitor {
    region: Path,
}

impl VisitMut for RegionVisitor {
    fn visit_item_fn_mut(&mut self, func: &mut ItemFn) {
        // Skip if function already has a memory_region attribute.
        if !has_memory_region_attr(&func.attrs) {
            let original_body = &func.block;
            let region = &self.region;

            let new_body: Block = parse_quote!({
                let _region_guard = ::flux_engine_memory::RegionGuard::new(#region);
                #original_body
            });

            func.block = Box::new(new_body);
        }

        // Continue visiting the function body.
        visit_item_fn_mut(self, func);
    }

    fn visit_item_mod_mut(&mut self, module: &mut ItemMod) {
        if let Some((_, items)) = &mut module.content {
            for item in items.iter_mut() {
                match item {
                    Item::Fn(f) => self.visit_item_fn_mut(f),
                    Item::Mod(m) => self.visit_item_mod_mut(m),
                    _ => {}
                }
            }
        }
    }
}

fn has_memory_region_attr(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("memory_region"))
}

#[proc_macro_attribute]
pub fn memory_region(attr: TokenStream, item: TokenStream) -> TokenStream {
    let region = parse_macro_input!(attr as RegionExpr);

    if let Ok(mut module) = syn::parse::<ItemMod>(item.clone()) {
        if let Some((_, ref mut items)) = module.content {
            let mut visitor = RegionVisitor {
                region: region.path.clone(),
            };

            for item in items.iter_mut() {
                match item {
                    Item::Fn(f) => visitor.visit_item_fn_mut(f),
                    Item::Mod(m) => visitor.visit_item_mod_mut(m),
                    _ => {}
                }
            }
        }

        module.to_token_stream().into()
    } else if let Ok(mut function) = syn::parse::<ItemFn>(item.clone()) {
        let region_path = region.path;
        let original_body = &function.block;

        let new_body: Block = parse_quote!({
            let _region_guard = ::flux_engine_memory::RegionGuard::new(#region_path);
            #original_body
        });

        function.block = Box::new(new_body);
        function.to_token_stream().into()
    } else {
        syn::Error::new_spanned(
            proc_macro2::TokenStream::from(item),
            "memory_region attribute can only be applied to modules or functions",
        )
        .to_compile_error()
        .into()
    }
}

#[proc_macro_attribute]
pub fn override_region(attr: TokenStream, item: TokenStream) -> TokenStream {
    let region = parse_macro_input!(attr as RegionExpr);
    let mut function = parse_macro_input!(item as ItemFn);

    let region_path = region.path;
    let original_body = &function.block;

    let new_body: Block = parse_quote!({
        let _region_guard = ::flux_engine_memory::RegionGuard::new(#region_path);
        #original_body
    });

    function.block = Box::new(new_body);
    function.to_token_stream().into()
}
