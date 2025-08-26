use flux_renderer_macros::Vertex;

#[repr(C)]
#[derive(Vertex)]
struct CoolVertex {
    position: [f32; 3],
}
