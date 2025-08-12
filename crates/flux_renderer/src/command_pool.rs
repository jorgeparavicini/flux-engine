use ash::vk;
use log::debug;
use flux_ecs::commands::Commands;
use flux_ecs::resource::{Res, Resource};
use crate::device::Device;

pub struct CommandPools {
    pub graphics: vk::CommandPool,
    pub transfer: vk::CommandPool,
}

impl Resource for CommandPools {}

pub fn create_command_pools(device: Res<Device>, mut commands: Commands) -> Result<(), vk::Result> {
    debug!("Creating command pools");

    let info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(device.graphics_queue_index);

    let graphics_pool = unsafe {
        device.create_command_pool(&info, None)?
    };

    let info = vk::CommandPoolCreateInfo::default()
        .queue_family_index(device.transfer_queue_index);

    let transfer_pool = unsafe {
        device.create_command_pool(&info, None)?
    };

    commands.insert_resource(CommandPools {
        graphics: graphics_pool,
        transfer: transfer_pool,
    });

    Ok(())
}

pub fn destroy_command_pools(
    device: Res<Device>,
    command_pools: Res<CommandPools>,
    mut commands: Commands,
) {
    debug!("Destroying command pools");

    unsafe {
        device.destroy_command_pool(command_pools.graphics, None);
        device.destroy_command_pool(command_pools.transfer, None);
    }

    commands.remove_resource::<CommandPools>();
}