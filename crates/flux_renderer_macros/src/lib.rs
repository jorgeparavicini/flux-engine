use proc_macro::TokenStream;
use syn::__private::quote;
use syn::{parse_macro_input, ItemStruct};

#[proc_macro_derive(Vertex)]
pub fn derive_vertex(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as ItemStruct);

    let mut vertex_attributes = Vec::with_capacity(input.fields.len());
    let mut current_offset = 0;

    for (i, field) in input.fields.iter().enumerate() {
        let field_type = &field.ty;
        let format = match quote::quote! {#field_type}.to_string().as_str() {
            "f32" => "Float32x1",
            "[f32; 2]" => "Float32x2",
            "[f32; 3]" => "Float32x3",
            "[f32; 4]" => "Float32x4",
            invalid_field => panic!("Unsupported field {:?} in Vertex struct", invalid_field),
        };

        let attribute = quote::quote! {
            VertexAttribute {
                location: #i as u32,
                format: VertexFormat::#format,
                offset: #current_offset,
            }
        };

        vertex_attributes.push(attribute);
        current_offset += match format {
            "Float32x1" => 4,
            "Float32x2" => 8,
            "Float32x3" => 12,
            "Float32x4" => 16,
            _ => 0,
        };
    }

    let output = quote::quote! {
        impl Vertex for #input {
            fn layout() -> VertexLayout {
                VertexLayout {
                    attributes: vec![
                        #(#vertex_attributes),*
                    ],
                    stride: #current_offset,
                }
            }
        }
    };

    panic!("Output {:?}", output);

    TokenStream::from(output)
}
