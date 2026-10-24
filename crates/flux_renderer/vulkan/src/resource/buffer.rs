use crate::device::logical::RenderDevice;
use crate::error::RendererError;
use crate::memory::allocator::MemoryUsage;
use crate::memory::gpu_memory::GpuMemory;
use crate::present::presenter::Presenter;
use crate::resource::gpu_buffer::GpuBuffer;
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

/// One host-visible uniform buffer per swapchain image.
#[derive(flux_ecs::Component)]
pub struct UniformBuffers {
    pub buffers: Vec<GpuBuffer>,
}

pub fn create_uniform_buffers(
    device: Single<&RenderDevice>,
    presenter: Single<&Presenter>,
    mut memory: Single<&mut GpuMemory>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    debug!("Creating uniform buffers");

    let buffers = (0..presenter.image_count())
        .map(|i| {
            GpuBuffer::new(
                &device,
                &mut memory,
                size_of::<UniformBufferObject>() as u64,
                vk::BufferUsageFlags::UNIFORM_BUFFER,
                MemoryUsage::CpuToGpu,
                &format!("uniform buffer {i}"),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    commands.insert_singleton(UniformBuffers { buffers });

    Ok(())
}
