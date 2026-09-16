use crate::device::logical::RenderDevice;
use ash::vk;

pub fn create_image(
    device: &RenderDevice,
    width: u32,
    height: u32,
    format: vk::Format,
    tiling: vk::ImageTiling,
    usage: vk::ImageUsageFlags,
    properties: vk::MemoryPropertyFlags,
) -> Result<(vk::Image, vk::DeviceMemory), vk::Result> {
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .format(format)
        .tiling(tiling)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .samples(vk::SampleCountFlags::TYPE_1)
        .flags(vk::ImageCreateFlags::empty());

    let image = unsafe { device.create_image(&info, None)? };

    let requirements = unsafe { device.get_image_memory_requirements(image) };

    let info = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(get_memory_type_index(device, properties, requirements).unwrap());

    let image_memory = unsafe { device.allocate_memory(&info, None)? };

    unsafe {
        device.bind_image_memory(image, image_memory, 0)?;
    }

    Ok((image, image_memory))
}

/// Finds a memory type from the device's measured memory properties that
/// satisfies both the allocation's requirements and the requested flags.
pub fn get_memory_type_index(
    device: &RenderDevice,
    properties: vk::MemoryPropertyFlags,
    requirements: vk::MemoryRequirements,
) -> Option<u32> {
    let memory = &device.capabilities.memory;
    (0..memory.memory_type_count).find(|i| {
        let suitable = (requirements.memory_type_bits & (1 << i) as u32) != 0;
        let memory_type = memory.memory_types[*i as usize];
        suitable && memory_type.property_flags.contains(properties)
    })
}

pub fn create_image_view(
    device: &RenderDevice,
    image: vk::Image,
    format: vk::Format,
    aspects: vk::ImageAspectFlags,
) -> Result<vk::ImageView, vk::Result> {
    let subresource_range = vk::ImageSubresourceRange::default()
        .aspect_mask(aspects)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1);

    let info = vk::ImageViewCreateInfo::default()
        .image(image)
        .view_type(vk::ImageViewType::TYPE_2D)
        .format(format)
        .subresource_range(subresource_range);

    Ok(unsafe { device.create_image_view(&info, None)? })
}
