use crate::command_pool::CommandPools;
use crate::device::Device;
use crate::swapchain::Swapchain;
use ash::vk;
use flux_ecs::Single;
use flux_ecs::Commands;
use log::debug;
use std::ops::Deref;

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
    device: Single<&Device>,
    command_pools: Single<&CommandPools>,
    swapchain: Single<&Swapchain>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    debug!("Creating command buffer");

    let allocate_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pools.graphics)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(swapchain.images.len() as u32);

    let command_buffers = unsafe { device.allocate_command_buffers(&allocate_info)? };
    commands.insert_singleton(CommandBuffers { command_buffers });

    Ok(())
}

pub fn destroy_command_buffers(
    device: Single<&Device>,
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
