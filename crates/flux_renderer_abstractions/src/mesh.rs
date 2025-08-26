// TODO: Not a fan of this naming
pub enum VertexFormat {
    Float32x2,
    Float32x3,
}

pub struct VertexAttribute {
    pub location: u32,
    pub format: VertexFormat,
    pub offset: u32,
}

pub struct VertexLayout {
    pub attributes: Vec<VertexAttribute>,
    pub stride: u32,
}

pub trait Vertex {
    fn layout() -> VertexLayout;
}

pub struct Mesh<V: Vertex> {
    pub vertices: Vec<V>,
    pub indices: Option<Vec<u32>>,
}
