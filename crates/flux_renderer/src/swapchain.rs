use crate::device::{Device, PhysicalDevice};
use crate::instance::{SurfaceProviderResource, VulkanInstance};
use crate::surface::VulkanSurface;
use ash::{khr, vk};
use flux_ecs::commands::Commands;
use flux_ecs::resource::{Res, Resource};
use log::debug;
use std::ops::Deref;

pub struct Swapchain {
    format: vk::SurfaceFormatKHR,
    extent: vk::Extent2D,
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    image_views: Vec<vk::ImageView>,
}

impl Resource for Swapchain {}

impl Deref for Swapchain {
    type Target = vk::SwapchainKHR;

    fn deref(&self) -> &Self::Target {
        &self.swapchain
    }
}

pub fn create_swapchain(
    instance: Res<VulkanInstance>,
    physical_device: Res<PhysicalDevice>,
    device: Res<Device>,
    surface: Res<VulkanSurface>,
    surface_provider: Res<SurfaceProviderResource>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    debug!("Creating swapchain");

    let surface_format = physical_device
        .formats
        .iter()
        .cloned()
        .find(|format| {
            format.format == vk::Format::B8G8R8A8_SRGB
                && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        })
        .unwrap_or(physical_device.formats[0]);

    let present_mode = physical_device
        .present_modes
        .iter()
        .cloned()
        .find(|mode| *mode == vk::PresentModeKHR::MAILBOX)
        .unwrap_or(vk::PresentModeKHR::FIFO); // The spec requires FIFO to be available

    let extent = if physical_device.capabilities.current_extent.width != u32::MAX {
        physical_device.capabilities.current_extent
    } else {
        let (width, height) = surface_provider.get_extent();
        let min_size = physical_device.capabilities.min_image_extent;
        let max_size = physical_device.capabilities.max_image_extent;

        vk::Extent2D {
            width: width.clamp(min_size.width, max_size.width),
            height: height.clamp(min_size.height, max_size.height),
        }
    };

    let mut image_count = physical_device.capabilities.min_image_count + 1;
    if physical_device.capabilities.max_image_count > 0
        && image_count > physical_device.capabilities.max_image_count
    {
        image_count = physical_device.capabilities.max_image_count;
    }

    let mut queue_family_indices = vec![];
    let image_sharing_mode = if physical_device.indices.graphics != physical_device.indices.present
    {
        queue_family_indices.push(physical_device.indices.graphics);
        queue_family_indices.push(physical_device.indices.present);
        vk::SharingMode::CONCURRENT
    } else {
        vk::SharingMode::EXCLUSIVE
    };

    let create_info = vk::SwapchainCreateInfoKHR::default()
        .surface(**surface)
        .min_image_count(image_count)
        .image_format(surface_format.format)
        .image_color_space(surface_format.color_space)
        .image_extent(extent)
        .image_array_layers(1)
        .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
        .image_sharing_mode(image_sharing_mode)
        .queue_family_indices(&queue_family_indices)
        .pre_transform(physical_device.capabilities.current_transform)
        .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
        .present_mode(present_mode)
        .clipped(true)
        .old_swapchain(vk::SwapchainKHR::null());

    let loader = khr::swapchain::Device::new(&instance, &device);
    let swapchain = unsafe { loader.create_swapchain(&create_info, None) }?;
    let images = unsafe { loader.get_swapchain_images(swapchain)? };

    let image_views = images
        .iter()
        .map(|image| create_image_view(*image, surface_format.format, &device))
        .collect::<Vec<_>>();

    commands.insert_resource(Swapchain {
        swapchain,
        images,
        format: surface_format,
        extent,
        image_views,
    });

    Ok(())
}

fn create_image_view(image: vk::Image, format: vk::Format, device: &Device) -> vk::ImageView {
    let create_info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .components(vk::ComponentMapping {
            r: vk::ComponentSwizzle::IDENTITY,
            g: vk::ComponentSwizzle::IDENTITY,
            b: vk::ComponentSwizzle::IDENTITY,
            a: vk::ComponentSwizzle::IDENTITY,
        })
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });

    unsafe { device.create_image_view(&create_info, None).unwrap() }
}

pub fn destroy_swapchain(
    instance: Res<VulkanInstance>,
    device: Res<Device>,
    swapchain: Res<Swapchain>,
    mut commands: Commands,
) {
    debug!("Destroying swapchain");
    let loader = khr::swapchain::Device::new(&instance, &device);

    unsafe {
        for &image_view in &swapchain.image_views {
            device.destroy_image_view(image_view, None);
        }
        loader.destroy_swapchain(**swapchain, None);
    }

    commands.remove_resource::<Swapchain>();
}
