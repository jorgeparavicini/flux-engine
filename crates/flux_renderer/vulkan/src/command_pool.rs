use crate::device::Device;
use ash::vk;
use flux_ecs::Commands;
use flux_ecs::Single;
use log::debug;

#[derive(flux_ecs::Component)]
pub struct CommandPools {
    pub graphics: vk::CommandPool,
    pub transfer: vk::CommandPool,
}

pub fn create_command_pools(
    device: Single<&Device>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    debug!("Creating command pools");

    let info = vk::CommandPoolCreateInfo::default()
        .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
        .queue_family_index(device.graphics_queue_index);

    let graphics_pool = unsafe { device.create_command_pool(&info, None)? };

    let info = vk::CommandPoolCreateInfo::default().queue_family_index(device.transfer_queue_index);

    let transfer_pool = unsafe { device.create_command_pool(&info, None)? };

    commands.insert_singleton(CommandPools {
        graphics: graphics_pool,
        transfer: transfer_pool,
    });

    Ok(())
}

pub fn destroy_command_pools(
    device: Single<&Device>,
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
