//! An owned Vulkan image with its default view: created bound to memory,
//! destroyed through the retire queue when dropped.

use crate::device::logical::RenderDevice;
use crate::error::RendererError;
use crate::memory::allocator::{AllocRequest, Allocation, GpuAllocator, MemoryUsage};
use crate::memory::gpu_memory::GpuMemory;
use crate::resource::retire::{Retired, Retirer};
use ash::vk;

/// Everything needed to create a [`GpuImage`].
#[derive(Debug, Clone, Copy)]
pub struct GpuImageDesc {
    pub extent: vk::Extent2D,
    pub format: vk::Format,
    pub tiling: vk::ImageTiling,
    pub usage: vk::ImageUsageFlags,
    pub aspects: vk::ImageAspectFlags,
}

/// An image, its default view, and the memory backing them. Dropping retires
/// all three on the GPU's schedule.
pub struct GpuImage {
    raw: vk::Image,
    view: vk::ImageView,
    allocation: Option<Allocation>,
    retirer: Retirer,
    pub format: vk::Format,
    pub extent: vk::Extent2D,
}

impl GpuImage {
    /// Creates a 2D image with one mip level and its default view, both
    /// named for captures and validation messages.
    pub fn new(
        device: &RenderDevice,
        memory: &mut GpuMemory,
        desc: GpuImageDesc,
        memory_usage: MemoryUsage,
        name: &str,
    ) -> Result<Self, RendererError> {
        let create_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .extent(vk::Extent3D {
                width: desc.extent.width,
                height: desc.extent.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .format(desc.format)
            .tiling(desc.tiling)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .usage(desc.usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .samples(vk::SampleCountFlags::TYPE_1)
            .flags(vk::ImageCreateFlags::empty());

        let image = unsafe { device.create_image(&create_info, None)? };
        let requirements = unsafe { device.get_image_memory_requirements(image) };

        let allocation = match memory.allocator.alloc(&AllocRequest {
            size: requirements.size,
            alignment: requirements.alignment,
            memory_type_bits: requirements.memory_type_bits,
            usage: memory_usage,
        }) {
            Ok(allocation) => allocation,
            Err(e) => {
                unsafe { device.destroy_image(image, None) };
                return Err(e);
            }
        };

        if let Err(e) =
            unsafe { device.bind_image_memory(image, allocation.memory, allocation.offset) }
        {
            unsafe {
                device.destroy_image(image, None);
                memory.allocator.free(allocation);
            }
            return Err(e.into());
        }

        device.debug_utils.set_name(image, name);

        let subresource_range = vk::ImageSubresourceRange::default()
            .aspect_mask(desc.aspects)
            .base_mip_level(0)
            .level_count(1)
            .base_array_layer(0)
            .layer_count(1);

        let info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(desc.format)
            .subresource_range(subresource_range);

        let image_view = match unsafe { device.create_image_view(&info, None) } {
            Ok(view) => view,
            Err(e) => {
                unsafe {
                    device.destroy_image(image, None);
                    memory.allocator.free(allocation);
                }
                return Err(e.into());
            }
        };

        device
            .debug_utils
            .set_name(image_view, &format!("{name} view"));

        Ok(Self {
            raw: image,
            view: image_view,
            allocation: Some(allocation),
            retirer: memory.retirer(),
            extent: desc.extent,
            format: desc.format,
        })
    }

    #[must_use]
    pub const fn raw(&self) -> vk::Image {
        self.raw
    }

    #[must_use]
    pub const fn view(&self) -> vk::ImageView {
        self.view
    }
}

impl Drop for GpuImage {
    fn drop(&mut self) {
        self.retirer.retire(Retired::ImageView(self.view));
        self.retirer.retire(Retired::Image(self.raw));
        if let Some(allocation) = self.allocation.take() {
            self.retirer.retire(Retired::Memory(allocation));
        }
    }
}
