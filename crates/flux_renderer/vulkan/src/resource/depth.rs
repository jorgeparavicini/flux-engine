use crate::device::instance::Instance;
use crate::device::logical::RenderDevice;
use crate::error::RendererError;
use crate::memory::allocator::MemoryUsage;
use crate::memory::gpu_memory::GpuMemory;
use crate::present::presenter::Presenter;
use crate::resource::gpu_image::{GpuImage, GpuImageDesc};
use ash::vk;
use flux_ecs::Commands;
use flux_ecs::Single;
use log::debug;

/// The forward pass's depth attachment, sized to the presenter.
#[derive(flux_ecs::Component)]
pub struct DepthBuffer {
    pub image: GpuImage,
}

/// Creates the [`DepthBuffer`] singleton for the presenter's current extent.
/// Re-running it replaces the previous image, which retires on drop.
pub fn create_depth_buffer(
    instance: Single<&Instance>,
    device: Single<&RenderDevice>,
    presenter: Single<&Presenter>,
    mut memory: Single<&mut GpuMemory>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    debug!("Creating depth buffer");

    let format =
        depth_format(&instance, device.physical).ok_or(RendererError::NoSupportedDepthFormat)?;

    let image = GpuImage::new(
        &device,
        &mut memory,
        GpuImageDesc {
            extent: presenter.extent,
            format,
            tiling: vk::ImageTiling::OPTIMAL,
            usage: vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            aspects: vk::ImageAspectFlags::DEPTH,
        },
        MemoryUsage::GpuOnly,
        "depth image",
    )?;

    commands.insert_singleton(DepthBuffer { image });

    Ok(())
}

fn depth_format(instance: &Instance, physical_device: vk::PhysicalDevice) -> Option<vk::Format> {
    let candidates = [
        vk::Format::D32_SFLOAT_S8_UINT,
        vk::Format::D32_SFLOAT,
        vk::Format::D24_UNORM_S8_UINT,
    ];

    supported_format(
        instance,
        physical_device,
        &candidates,
        vk::ImageTiling::OPTIMAL,
        vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT,
    )
}

fn supported_format(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
    candidates: &[vk::Format],
    tiling: vk::ImageTiling,
    features: vk::FormatFeatureFlags,
) -> Option<vk::Format> {
    candidates.iter().copied().find(|&format| {
        let properties =
            unsafe { instance.get_physical_device_format_properties(physical_device, format) };

        match tiling {
            vk::ImageTiling::LINEAR => properties.linear_tiling_features.contains(features),
            vk::ImageTiling::OPTIMAL => properties.optimal_tiling_features.contains(features),
            _ => false,
        }
    })
}
