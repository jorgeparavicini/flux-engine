use crate::device::{Device, PhysicalDevice};
use crate::image::{create_image, create_image_view};
use crate::instance::VulkanInstance;
use crate::swapchain::Swapchain;
use ash::vk;
use flux_ecs::commands::Commands;
use flux_ecs::resource::{Res, Resource};
use log::debug;

pub struct DepthBuffers {
    pub depth_image: vk::Image,
    pub depth_image_view: vk::ImageView,
    pub depth_image_memory: vk::DeviceMemory,
    pub depth_format: vk::Format,
}

impl Resource for DepthBuffers {}

pub fn create_depth_buffers(
    instance: Res<VulkanInstance>,
    physical_device: Res<PhysicalDevice>,
    device: Res<Device>,
    swapchain: Res<Swapchain>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    debug!("Creating depth buffers");

    let depth_format = get_depth_format(&instance, &physical_device).unwrap();

    let (depth_image, depth_image_memory) = create_image(
        &instance,
        &physical_device,
        &device,
        swapchain.extent.width,
        swapchain.extent.height,
        depth_format,
        vk::ImageTiling::OPTIMAL,
        vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
        vk::MemoryPropertyFlags::DEVICE_LOCAL,
    )?;

    let depth_image_view = create_image_view(
        &device,
        depth_image,
        depth_format,
        vk::ImageAspectFlags::DEPTH,
    )?;

    let depth_buffers = DepthBuffers {
        depth_image,
        depth_image_view,
        depth_image_memory,
        depth_format,
    };

    commands.insert_resource(depth_buffers);

    Ok(())
}

fn get_depth_format(
    instance: &VulkanInstance,
    physical_device: &PhysicalDevice,
) -> Option<vk::Format> {
    let candidates = [
        vk::Format::D32_SFLOAT_S8_UINT,
        vk::Format::D32_SFLOAT,
        vk::Format::D24_UNORM_S8_UINT,
    ];

    get_supported_format(
        instance,
        physical_device,
        &candidates,
        vk::ImageTiling::OPTIMAL,
        vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT,
    )
}

fn get_supported_format(
    instance: &VulkanInstance,
    physical_device: &PhysicalDevice,
    candidates: &[vk::Format],
    tiling: vk::ImageTiling,
    features: vk::FormatFeatureFlags,
) -> Option<vk::Format> {
    candidates.iter().cloned().find(|f| {
        let properties =
            unsafe { instance.get_physical_device_format_properties(**physical_device, *f) };

        match tiling {
            vk::ImageTiling::LINEAR => properties.linear_tiling_features.contains(features),
            vk::ImageTiling::OPTIMAL => properties.optimal_tiling_features.contains(features),
            _ => false,
        }
    })
}
