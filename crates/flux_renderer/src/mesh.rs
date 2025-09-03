use crate::buffers::{copy_buffer, create_buffer};
use crate::command_pool::CommandPools;
use crate::device::{Device, PhysicalDevice};
use crate::instance::VulkanInstance;
use ash::vk;
use flux_ecs::commands::Commands;
use flux_ecs::query::Query;
use flux_ecs::resource::Res;
use flux_renderer_abstractions::mesh::VertexFormat::Float32x3;
use flux_renderer_abstractions::mesh::{Mesh, Vertex, VertexAttribute, VertexLayout};
use log::debug;
use std::ptr::copy_nonoverlapping;

#[repr(C)]
struct CoolVertex {
    position: [f32; 3],
}

impl Vertex for CoolVertex {
    fn layout() -> VertexLayout {
        let attribute = VertexAttribute {
            location: 0,
            format: Float32x3,
            offset: 0,
        };

        VertexLayout {
            attributes: vec![attribute],
            stride: 0,
        }
    }
}

pub struct VulkanMesh {
    pub vertex_buffer: vk::Buffer,
    pub vertex_buffer_memory: vk::DeviceMemory,
    pub index_buffer: vk::Buffer,
    pub index_buffer_memory: vk::DeviceMemory,
}

pub fn create_buffers(
    instance: Res<VulkanInstance>,
    physical_device: Res<PhysicalDevice>,
    device: Res<Device>,
    command_pools: Res<CommandPools>,
    // TODO: Needs to be able to be generalized
    meshes: Query<&Mesh<CoolVertex>>,
    mut commands: Commands,
) {}

fn create_vertex_buffer(
    instance: &VulkanInstance,
    physical_device: &PhysicalDevice,
    device: &Device,
    command_pools: &CommandPools,
    mesh: &Mesh<CoolVertex>,
) -> Result<(vk::Buffer, vk::DeviceMemory), vk::Result> {
    debug!("Creating vertex buffer for mesh {:?}", mesh);

    let size = Mesh::<CoolVertex>::size() as u64 * mesh.vertices.len() as u64;

    let (staging_buffer, staging_buffer_memory) = create_buffer(
        instance,
        physical_device,
        device,
        size,
        vk::BufferUsageFlags::TRANSFER_SRC,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;

    let memory = unsafe {
        device.map_memory(staging_buffer_memory, 0, size, vk::MemoryMapFlags::empty())?
    };

    unsafe {
        copy_nonoverlapping(mesh.vertices.as_ptr(), memory.cast(), mesh.vertices.len());
        device.unmap_memory(staging_buffer_memory);
    }

    let (vertex_buffer, vertex_buffer_memory) = create_buffer(
        instance,
        physical_device,
        device,
        size,
        vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::VERTEX_BUFFER,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;

    copy_buffer(device, command_pools, staging_buffer, vertex_buffer, size)?;

    unsafe {
        device.destroy_buffer(staging_buffer, None);
        device.free_memory(staging_buffer_memory, None);
    }

    Ok((vertex_buffer, vertex_buffer_memory))
}

fn create_index_buffer(
    instance: &VulkanInstance,
    physical_device: &PhysicalDevice,
    device: &Device,
    command_pools: &CommandPools,
    mesh: &Mesh<CoolVertex>,
) -> Result<Option<(vk::Buffer, vk::DeviceMemory)>, vk::Result> {
    debug!("Creating index buffer for mesh {:?}", mesh);

    if mesh.indices.is_none() {
        return Ok(None);
    }

    let indices = &mesh.indices?;

    let Some(indices) = &mesh.indices;
    let size = size_of::<u32>() * indices.len();

    let (staging_buffer, staging_buffer_memory) = create_buffer(
        instance,
        physical_device,
        device,
        size as u64,
        vk::BufferUsageFlags::TRANSFER_SRC,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;

    Ok(None)
}
