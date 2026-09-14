use crate::device::instance::VulkanInstance;
use crate::device::selection::{DeviceRequirements, PhysicalDevice};
use ash::vk;
use flux_ecs::Commands;
use flux_ecs::Single;
use log::{debug, info};
use std::collections::HashSet;
use std::ops::Deref;

#[derive(flux_ecs::Component)]
pub struct Device {
    pub device: ash::Device,
    pub graphics_queue: vk::Queue,
    pub graphics_queue_index: u32,
    pub present_queue: vk::Queue,
    pub present_queue_index: u32,
    pub transfer_queue: vk::Queue,
    pub transfer_queue_index: u32,
}

impl Deref for Device {
    type Target = ash::Device;

    fn deref(&self) -> &Self::Target {
        &self.device
    }
}

pub fn create_logical_device(
    instance: Single<&VulkanInstance>,
    physical_device: Single<&PhysicalDevice>,
    device_requirements: Option<Single<&DeviceRequirements>>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    info!(
        "Creating logical device for physical device: {:?}",
        *physical_device
    );

    let mut unique_indices = HashSet::new();
    unique_indices.insert(physical_device.indices.graphics);
    unique_indices.insert(physical_device.indices.present);
    unique_indices.insert(physical_device.indices.transfer);

    debug!(
        "Creating logical device with {} queue families",
        unique_indices.len()
    );

    let queue_create_infos: Vec<vk::DeviceQueueCreateInfo> = unique_indices
        .into_iter()
        .map(|index| {
            vk::DeviceQueueCreateInfo::default()
                .queue_family_index(index)
                .queue_priorities(&[1.0])
        })
        .collect();

    let requirements = device_requirements
        .map(|res| res.clone())
        .unwrap_or_default();

    let extensions = requirements
        .extensions
        .iter()
        .map(|&e| e.as_ptr())
        .collect::<Vec<_>>();

    let features = vk::PhysicalDeviceFeatures::default().sampler_anisotropy(true);

    let mut dynamic_rendering_features =
        vk::PhysicalDeviceDynamicRenderingFeatures::default().dynamic_rendering(true);

    let mut physical_device_features_2 = vk::PhysicalDeviceFeatures2::default()
        .features(features)
        .push_next(&mut dynamic_rendering_features);

    let create_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queue_create_infos)
        .enabled_extension_names(&extensions)
        .push_next(&mut physical_device_features_2);

    let device = unsafe { instance.create_device(**physical_device, &create_info, None) }?;

    let graphics_queue = unsafe { device.get_device_queue(physical_device.indices.graphics, 0) };
    let present_queue = unsafe { device.get_device_queue(physical_device.indices.present, 0) };
    let transfer_queue = unsafe { device.get_device_queue(physical_device.indices.transfer, 0) };

    let logical_device = Device {
        device,
        graphics_queue,
        graphics_queue_index: physical_device.indices.graphics,
        present_queue,
        present_queue_index: physical_device.indices.present,
        transfer_queue,
        transfer_queue_index: physical_device.indices.transfer,
    };

    commands.insert_singleton(logical_device);

    Ok(())
}

pub fn destroy_logical_device(device: Single<&Device>, mut commands: Commands) {
    info!("Destroying logical device");

    unsafe { device.destroy_device(None) };

    commands.remove_singleton::<Device>();
}
