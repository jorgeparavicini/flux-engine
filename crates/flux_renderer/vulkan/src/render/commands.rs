use crate::device::logical::RenderDevice;
use crate::error::RendererError;
use crate::present::swapchain::Swapchain;
use ash::vk;
use flux_ecs::Commands;
use flux_ecs::Single;
use log::debug;
use std::ops::Deref;

#[derive(flux_ecs::Component)]
pub struct CommandPools {
    pub graphics: vk::CommandPool,
    pub transfer: vk::CommandPool,
}

pub fn create_command_pools(
    device: Single<&RenderDevice>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    debug!("Creating command pools");

    let info = vk::CommandPoolCreateInfo::default()
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
        .queue_family_index(device.graphics.family);

    let graphics_pool = unsafe { device.create_command_pool(&info, None)? };

    let info = vk::CommandPoolCreateInfo::default().queue_family_index(device.transfer.family);

    let transfer_pool = unsafe { device.create_command_pool(&info, None)? };

    device
        .debug_utils
        .set_name(graphics_pool, "graphics command pool");
    device
        .debug_utils
        .set_name(transfer_pool, "transfer command pool");

    commands.insert_singleton(CommandPools {
        graphics: graphics_pool,
        transfer: transfer_pool,
    });

    Ok(())
}

pub fn destroy_command_pools(
    device: Single<&RenderDevice>,
    command_pools: Single<&CommandPools>,
    mut commands: Commands,
) {
    debug!("Destroying command pools");

    unsafe {
        device.destroy_command_pool(command_pools.graphics, None);
        device.destroy_command_pool(command_pools.transfer, None);
    }

    commands.remove_singleton::<CommandPools>();
}

#[derive(flux_ecs::Component)]
pub struct CommandBuffers {
    pub command_buffers: Vec<vk::CommandBuffer>,
}

impl Deref for CommandBuffers {
    type Target = Vec<vk::CommandBuffer>;

    fn deref(&self) -> &Self::Target {
        &self.command_buffers
    }
}

pub fn create_command_buffer(
    device: Single<&RenderDevice>,
    command_pools: Single<&CommandPools>,
    swapchain: Single<&Swapchain>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    debug!("Creating command buffer");

    let allocate_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pools.graphics)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(swapchain.images.len() as u32);

    let command_buffers = unsafe { device.allocate_command_buffers(&allocate_info)? };
    for (i, buffer) in command_buffers.iter().enumerate() {
        device
            .debug_utils
            .set_name(*buffer, &format!("frame command buffer {i}"));
    }
    commands.insert_singleton(CommandBuffers { command_buffers });

    Ok(())
}

pub fn destroy_command_buffers(
    device: Single<&RenderDevice>,
    command_pools: Single<&CommandPools>,
    command_buffers: Single<&CommandBuffers>,
    mut commands: Commands,
) {
    debug!("Destroying command buffers");
    unsafe {
        device.free_command_buffers(command_pools.graphics, &command_buffers.command_buffers);
    }
    commands.remove_singleton::<CommandBuffers>();
}
