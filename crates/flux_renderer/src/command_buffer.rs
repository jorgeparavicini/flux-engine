use crate::buffers::{IndexBuffer, VertexBuffer};
use crate::command_pool::CommandPools;
use crate::depth_buffers::DepthBuffers;
use crate::descriptors::Descriptors;
use crate::device::Device;
use crate::pipeline::Pipeline;
use crate::swapchain::Swapchain;
use ash::vk;
use flux_ecs::commands::Commands;
use flux_ecs::resource::{Res, Resource};
use log::debug;
use std::ops::Deref;

pub struct CommandBuffers {
    pub command_buffers: Vec<vk::CommandBuffer>,
}

impl Deref for CommandBuffers {
    type Target = Vec<vk::CommandBuffer>;

    fn deref(&self) -> &Self::Target {
        &self.command_buffers
    }
}

impl Resource for CommandBuffers {}

pub fn create_command_buffer(
    device: Res<Device>,
    command_pools: Res<CommandPools>,
    swapchain: Res<Swapchain>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    debug!("Creating command buffer");

    let allocate_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pools.graphics)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(swapchain.images.len() as u32);

    let command_buffers = unsafe { device.allocate_command_buffers(&allocate_info)? };
    commands.insert_resource(CommandBuffers { command_buffers });

    Ok(())
}

pub fn destroy_command_buffers(
    device: Res<Device>,
    command_pools: Res<CommandPools>,
    command_buffers: Res<CommandBuffers>,
    mut commands: Commands,
) {
    debug!("Destroying command buffers");
    unsafe {
        device.free_command_buffers(command_pools.graphics, &command_buffers.command_buffers);
    }
    commands.remove_resource::<CommandBuffers>();
}
