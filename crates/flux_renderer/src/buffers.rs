use crate::command_pool::CommandPools;
use crate::device::{Device, PhysicalDevice};
use crate::image::get_memory_type_index;
use crate::instance::VulkanInstance;
use crate::swapchain::Swapchain;
use ash::vk;
use flux_ecs::commands::Commands;
use flux_ecs::resource::{Res, Resource};
use log::debug;
use std::ptr::copy_nonoverlapping as memcpy;

type Vec2 = cgmath::Vector2<f32>;
type Vec3 = cgmath::Vector3<f32>;
type Mat4 = cgmath::Matrix4<f32>;

const VERTICES: [Vertex; 3] = [
    Vertex {
        pos: Vec3::new(-0.5, -0.5, 0.0),
        color: Vec3::new(1.0, 0.0, 0.0),
        tex_coords: Vec2::new(0.0, 1.0),
    },
    Vertex {
        pos: Vec3::new(0.5, -0.5, 0.0),
        color: Vec3::new(0.0, 1.0, 0.0),
        tex_coords: Vec2::new(1.0, 1.0),
    },
    Vertex {
        pos: Vec3::new(0.0, 0.5, 0.0),
        color: Vec3::new(0.0, 0.0, 1.0),
        tex_coords: Vec2::new(1.0, 0.0),
    },
];

pub struct Vertex {
    pos: Vec3,
    color: Vec3,
    tex_coords: Vec2,
}

pub struct UniformBufferObject {
    pub model: Mat4,
    pub view: Mat4,
    pub projection: Mat4,
}

pub struct VertexBuffer {
    pub buffer: vk::Buffer,
    pub memory: vk::DeviceMemory,
}

impl Resource for VertexBuffer {}

pub struct IndexBuffer {
    pub buffer: vk::Buffer,
    pub memory: vk::DeviceMemory,
}

impl Resource for IndexBuffer {}

pub struct UniformBuffer {
    pub buffer: vk::Buffer,
    pub memory: vk::DeviceMemory,
}

pub struct UniformBuffers {
    pub buffers: Vec<UniformBuffer>,
}

impl Resource for UniformBuffers {}

pub fn create_vertex_buffer(
    instance: Res<VulkanInstance>,
    physical_device: Res<PhysicalDevice>,
    device: Res<Device>,
    command_pools: Res<CommandPools>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    debug!("Creating vertex buffer");

    let size = (size_of::<Vertex>() * VERTICES.len()) as u64;

    let (staging_buffer, staging_buffer_memory) = create_buffer(
        &instance,
        &physical_device,
        &device,
        size,
        vk::BufferUsageFlags::TRANSFER_SRC,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;

    let memory =
        unsafe { device.map_memory(staging_buffer_memory, 0, size, vk::MemoryMapFlags::empty())? };

    unsafe {
        memcpy(
            VERTICES.as_ptr() as *const u8,
            memory.cast(),
            VERTICES.len(),
        )
    }

    unsafe { device.unmap_memory(staging_buffer_memory) };

    let (vertex_buffer, vertex_buffer_memory) = create_buffer(
        &instance,
        &physical_device,
        &device,
        size,
        vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::VERTEX_BUFFER,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;

    copy_buffer(&device, &command_pools, staging_buffer, vertex_buffer, size)?;

    unsafe {
        device.destroy_buffer(staging_buffer, None);
        device.free_memory(staging_buffer_memory, None);
    }

    let vertex_buffer_resource = VertexBuffer {
        buffer: vertex_buffer,
        memory: vertex_buffer_memory,
    };

    commands.insert_resource(vertex_buffer_resource);

    Ok(())
}

pub fn create_index_buffer(
    instance: Res<VulkanInstance>,
    physical_device: Res<PhysicalDevice>,
    device: Res<Device>,
    command_pools: Res<CommandPools>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    debug!("Creating index buffer");

    let indices: [u32; 3] = [0, 1, 2];
    let size = (size_of::<u32>() * indices.len()) as u64;

    let (staging_buffer, staging_buffer_memory) = create_buffer(
        &instance,
        &physical_device,
        &device,
        size,
        vk::BufferUsageFlags::TRANSFER_SRC,
        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
    )?;

    let memory =
        unsafe { device.map_memory(staging_buffer_memory, 0, size, vk::MemoryMapFlags::empty())? };

    unsafe {
        memcpy(indices.as_ptr() as *const u8, memory.cast(), indices.len());
    }

    unsafe { device.unmap_memory(staging_buffer_memory) };

    let (index_buffer, index_buffer_memory) = create_buffer(
        &instance,
        &physical_device,
        &device,
        size,
        vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::INDEX_BUFFER,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;

    copy_buffer(&device, &command_pools, staging_buffer, index_buffer, size)?;

    unsafe {
        device.destroy_buffer(staging_buffer, None);
        device.free_memory(staging_buffer_memory, None);
    }

    commands.insert_resource(IndexBuffer {
        buffer: index_buffer,
        memory: index_buffer_memory,
    });

    Ok(())
}

pub fn create_uniform_buffer(
    instance: Res<VulkanInstance>,
    physical_device: Res<PhysicalDevice>,
    device: Res<Device>,
    swapchain: Res<Swapchain>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    debug!("Creating uniform buffer");

    let mut buffers = UniformBuffers {
        buffers: Vec::with_capacity(swapchain.images.len()),
    };

    for _ in 0..swapchain.images.len() {
        let (uniform_buffer, uniform_buffer_memory) = create_buffer(
            &instance,
            &physical_device,
            &device,
            size_of::<UniformBufferObject>() as u64,
            vk::BufferUsageFlags::UNIFORM_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;

        buffers.buffers.push(UniformBuffer {
            buffer: uniform_buffer,
            memory: uniform_buffer_memory,
        });
    }

    commands.insert_resource(buffers);

    Ok(())
}

fn create_buffer(
    instance: &VulkanInstance,
    physical_device: &PhysicalDevice,
    device: &Device,
    size: vk::DeviceSize,
    usage: vk::BufferUsageFlags,
    properties: vk::MemoryPropertyFlags,
) -> Result<(vk::Buffer, vk::DeviceMemory), vk::Result> {
    let buffer_info = vk::BufferCreateInfo::default()
        .size(size)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);

    let buffer = unsafe { device.create_buffer(&buffer_info, None)? };
    let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };

    let memory_info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(
            get_memory_type_index(instance, physical_device, properties, requirements).unwrap(),
        );

    let buffer_memory = unsafe { device.allocate_memory(&memory_info, None)? };

    unsafe {
        device.bind_buffer_memory(buffer, buffer_memory, 0)?;
    }

    Ok((buffer, buffer_memory))
}

fn copy_buffer(
    device: &Device,
    command_pools: &CommandPools,
    src_buffer: vk::Buffer,
    dst_buffer: vk::Buffer,
    size: vk::DeviceSize,
) -> Result<(), vk::Result> {
    let command_buffer = unsafe { begin_single_time_commands(device, command_pools.graphics)? };

    let regions = vk::BufferCopy::default().size(size);
    unsafe { device.cmd_copy_buffer(command_buffer, src_buffer, dst_buffer, &[regions]) };

    end_single_time_commands(
        device,
        device.graphics_queue,
        command_pools.graphics,
        command_buffer,
    )?;

    Ok(())
}

unsafe fn begin_single_time_commands(
    device: &Device,
    command_pool: vk::CommandPool,
) -> Result<vk::CommandBuffer, vk::Result> {
    let info = vk::CommandBufferAllocateInfo::default()
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_pool(command_pool)
        .command_buffer_count(1);

    let command_buffer = unsafe { device.allocate_command_buffers(&info)?[0] };

    let info =
        vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

    unsafe { device.begin_command_buffer(command_buffer, &info)? };

    Ok(command_buffer)
}

fn end_single_time_commands(
    device: &Device,
    queue: vk::Queue,
    command_pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
) -> Result<(), vk::Result> {
    unsafe { device.end_command_buffer(command_buffer)? };

    let command_buffers = &[command_buffer];
    let submit_info = vk::SubmitInfo::default().command_buffers(command_buffers);

    unsafe { device.queue_submit(queue, &[submit_info], vk::Fence::null())? };
    unsafe { device.queue_wait_idle(queue)? };

    unsafe { device.free_command_buffers(command_pool, command_buffers) };

    Ok(())
}
