use crate::device::logical::RenderDevice;
use crate::error::RendererError;
use crate::memory::allocator::MemoryUsage;
use crate::memory::gpu_memory::GpuMemory;
use crate::resource::gpu_buffer::GpuBuffer;
use crate::resource::upload::{StagingRing, Uploader};
use ash::vk;
use flux_ecs::{Commands, Entity, Query, Single};
use flux_renderer_api::mesh::VertexFormat::Float32x3;
use flux_renderer_api::mesh::{Mesh, Vertex, VertexAttribute, VertexLayout};
use log::debug;

#[repr(C)]
pub struct CoolVertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

impl Vertex for CoolVertex {
    fn layout() -> VertexLayout {
        let position_attribute = VertexAttribute {
            location: 0,
            format: Float32x3,
            offset: 0,
        };

        let color_attribute = VertexAttribute {
            location: 1,
            format: Float32x3,
            offset: size_of::<[f32; 3]>() as u32,
        };

        VertexLayout {
            attributes: vec![position_attribute, color_attribute],
            stride: 0,
        }
    }
}

/// The renderer's mesh input: [`Mesh`] made concrete so it can live on an
/// entity.
#[derive(flux_ecs::Component)]
pub struct MeshComponent(pub Mesh<CoolVertex>);

/// A mesh's GPU-side buffers, living on the same entity as its
/// [`MeshComponent`]; despawning the entity frees them.
#[derive(flux_ecs::Component)]
pub struct VulkanMesh {
    pub vertex_buffer: GpuBuffer,
    pub vertex_count: u32,
    pub indices: Option<IndexBuffer>,
}

pub struct IndexBuffer {
    pub buffer: GpuBuffer,
    pub count: u32,
}

/// Uploads every mesh in one transfer batch. Startup-shaped: the staging
/// space comes from frame slot 0, which the first frame recycles.
pub fn upload_meshes(
    device: Single<&RenderDevice>,
    mut memory: Single<&mut GpuMemory>,
    mut staging: Single<&mut StagingRing>,
    mut uploader: Single<&mut Uploader>,
    meshes: Query<(Entity, &MeshComponent)>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    debug!("Uploading meshes");

    uploader.begin()?;

    let mut gpu_meshes = Vec::new();
    let mut result = Ok(());
    meshes.for_each(|(entity, mesh)| {
        if result.is_ok() {
            match upload_mesh(&device, &mut memory, &mut staging, &mut uploader, &mesh.0) {
                Ok(gpu_mesh) => gpu_meshes.push((entity, gpu_mesh)),
                Err(e) => result = Err(e),
            }
        }
    });
    result?;

    uploader.submit_and_wait()?;

    for (entity, gpu_mesh) in gpu_meshes {
        commands.insert(entity, gpu_mesh);
    }

    Ok(())
}

fn upload_mesh(
    device: &RenderDevice,
    memory: &mut GpuMemory,
    staging: &mut StagingRing,
    uploader: &mut Uploader,
    mesh: &Mesh<CoolVertex>,
) -> Result<VulkanMesh, RendererError> {
    // SAFETY: `CoolVertex` is `repr(C)` with only `f32` fields, so it has no
    // padding bytes.
    let vertex_bytes = unsafe { as_bytes(&mesh.vertices) };
    let vertex_buffer = upload_buffer(
        device,
        memory,
        staging,
        uploader,
        vertex_bytes,
        vk::BufferUsageFlags::VERTEX_BUFFER,
        "mesh vertex buffer",
    )?;

    let indices = match mesh.indices.as_deref() {
        Some(indices) if !indices.is_empty() => {
            // SAFETY: `u32` has no padding bytes.
            let index_bytes = unsafe { as_bytes(indices) };
            let buffer = upload_buffer(
                device,
                memory,
                staging,
                uploader,
                index_bytes,
                vk::BufferUsageFlags::INDEX_BUFFER,
                "mesh index buffer",
            )?;
            Some(IndexBuffer {
                buffer,
                count: indices.len() as u32,
            })
        }
        _ => None,
    };

    Ok(VulkanMesh {
        vertex_buffer,
        vertex_count: mesh.vertices.len() as u32,
        indices,
    })
}

/// Creates a GPU-only buffer and records the copy of `bytes` into it.
fn upload_buffer(
    device: &RenderDevice,
    memory: &mut GpuMemory,
    staging: &mut StagingRing,
    uploader: &mut Uploader,
    bytes: &[u8],
    usage: vk::BufferUsageFlags,
    name: &str,
) -> Result<GpuBuffer, RendererError> {
    let slice = staging.stage(0, bytes).ok_or(RendererError::StagingFull {
        requested: bytes.len() as u64,
    })?;

    let buffer = GpuBuffer::new(
        device,
        memory,
        bytes.len() as u64,
        usage | vk::BufferUsageFlags::TRANSFER_DST,
        MemoryUsage::GpuOnly,
        name,
    )?;
    uploader.copy_buffer(slice, buffer.raw(), 0);

    Ok(buffer)
}

/// The raw bytes of `values`.
///
/// # Safety
///
/// `T` must have no padding bytes; reading padding is reading uninitialized
/// memory.
const unsafe fn as_bytes<T>(values: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), size_of_val(values)) }
}
