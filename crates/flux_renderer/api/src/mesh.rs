use std::fmt::Debug;

/// The data type of one vertex attribute, matching wgpu's vocabulary.
pub enum VertexFormat {
    Float32x2,
    Float32x3,
}

/// One attribute within a vertex: where it lives and what it holds.
pub struct VertexAttribute {
    pub location: u32,
    pub format: VertexFormat,
    pub offset: u32,
}

/// The complete memory layout of one vertex type.
pub struct VertexLayout {
    pub attributes: Vec<VertexAttribute>,
    pub stride: u32,
}

/// A type that can be uploaded as vertex data. Implementations describe
/// their own memory layout; derive this rather than writing it by hand.
pub trait Vertex {
    fn layout() -> VertexLayout;
}

/// Geometry as the application authors it: vertices plus optional indices.
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
    /// The size of a single vertex in bytes.
    #[must_use]
    pub const fn vertex_size() -> usize {
        size_of::<V>()
    }
}
