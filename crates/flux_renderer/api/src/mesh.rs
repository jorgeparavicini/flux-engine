use std::fmt::Debug;

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

impl<V: Vertex> Debug for Mesh<V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mesh")
            .field("vertices", &self.vertices.len())
            .field(
                "indices",
                &self.indices.as_ref().map_or(0, |indices| indices.len()),
            )
            .finish()
    }
}

impl<V: Vertex> Mesh<V> {
    pub fn vertex_size() -> usize {
        size_of::<V>()
    }
}
