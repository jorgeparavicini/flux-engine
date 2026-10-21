use crate::device::logical::RenderDevice;
use crate::error::RendererError;
use crate::present::presenter::Presenter;
use crate::render::commands::CommandPools;
use crate::resource::image::get_memory_type_index;
use ash::vk;
use flux_ecs::Commands;
use flux_ecs::Single;
use glam::Mat4;
use log::debug;

pub struct UniformBufferObject {
    pub model: Mat4,
    pub view: Mat4,
    pub projection: Mat4,
}

pub struct UniformBuffer {
    pub buffer: vk::Buffer,
    pub memory: vk::DeviceMemory,
}

#[derive(flux_ecs::Component)]
pub struct UniformBuffers {
    pub buffers: Vec<UniformBuffer>,
}

pub fn create_uniform_buffer(
    device: Single<&RenderDevice>,
    presenter: Single<&Presenter>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    debug!("Creating uniform buffer");

    let mut buffers = UniformBuffers {
        buffers: Vec::with_capacity(presenter.image_count()),
    };

    for i in 0..presenter.image_count() {
        let (uniform_buffer, uniform_buffer_memory) = create_buffer(
            &device,
            size_of::<UniformBufferObject>() as u64,
            vk::BufferUsageFlags::UNIFORM_BUFFER,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        device
            .debug_utils
            .set_name(uniform_buffer, &format!("uniform buffer {i}"));

        buffers.buffers.push(UniformBuffer {
            buffer: uniform_buffer,
            memory: uniform_buffer_memory,
        });
    }

    commands.insert_singleton(buffers);

    Ok(())
}

pub fn create_buffer(
    device: &RenderDevice,
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
        .memory_type_index(get_memory_type_index(device, properties, requirements).unwrap());

    let buffer_memory = unsafe { device.allocate_memory(&memory_info, None)? };

    unsafe {
        device.bind_buffer_memory(buffer, buffer_memory, 0)?;
    }

    Ok((buffer, buffer_memory))
}

pub fn copy_buffer(
    device: &RenderDevice,
    command_pools: &CommandPools,
    src_buffer: vk::Buffer,
    dst_buffer: vk::Buffer,
    size: vk::DeviceSize,
) -> Result<(), vk::Result> {
    let command_buffer = unsafe { begin_single_time_commands(device, command_pools.transfer)? };

    let regions = vk::BufferCopy::default().size(size);
    unsafe { device.cmd_copy_buffer(command_buffer, src_buffer, dst_buffer, &[regions]) };

    end_single_time_commands(
        device,
        device.transfer.raw,
        command_pools.transfer,
        command_buffer,
    )?;

    Ok(())
}

unsafe fn begin_single_time_commands(
    device: &RenderDevice,
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
    device: &RenderDevice,
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

pub fn destroy_buffers(device: Single<&RenderDevice>, uniform_buffers: Single<&UniformBuffers>) {
    debug!("Destroying buffers");

    unsafe {
        for uniform_buffer in &uniform_buffers.buffers {
            device.destroy_buffer(uniform_buffer.buffer, None);
            device.free_memory(uniform_buffer.memory, None);
        }
    }
}
