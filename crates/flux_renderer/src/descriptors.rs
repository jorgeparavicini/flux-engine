use crate::buffers::{UniformBufferObject, UniformBuffers};
use crate::device::Device;
use crate::pipeline::Pipeline;
use crate::swapchain::Swapchain;
use ash::vk;
use flux_ecs::commands::Commands;
use flux_ecs::resource::{Res, Resource};

pub struct Descriptors {
    pub descriptor_pool: vk::DescriptorPool,
    pub descriptor_sets: Vec<vk::DescriptorSet>,
}

impl Resource for Descriptors {}

pub fn create_descriptors(
    device: Res<Device>,
    pipeline: Res<Pipeline>,
    swapchain: Res<Swapchain>,
    uniform_buffer: Res<UniformBuffers>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    let pool = create_descriptor_pool(&device, &swapchain)?;
    let sets = create_descriptor_sets(&device, &pipeline, &swapchain, pool, &uniform_buffer)?;

    commands.insert_resource(Descriptors {
        descriptor_pool: pool,
        descriptor_sets: sets,
    });

    Ok(())
}

fn create_descriptor_pool(
    device: &Device,
    swapchain: &Swapchain,
) -> Result<vk::DescriptorPool, vk::Result> {
    let ubo_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::UNIFORM_BUFFER)
        .descriptor_count(swapchain.image_views.len() as u32);

    let sampler_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .descriptor_count(swapchain.image_views.len() as u32);

    let pool_sizes = &[ubo_size];
    let info = vk::DescriptorPoolCreateInfo::default()
        .pool_sizes(pool_sizes)
        .max_sets(swapchain.image_views.len() as u32);

    unsafe { device.create_descriptor_pool(&info, None) }
}

fn create_descriptor_sets(
    device: &Device,
    pipeline: &Pipeline,
    swapchain: &Swapchain,
    pool: vk::DescriptorPool,
    uniform_buffers: &UniformBuffers,
) -> Result<Vec<vk::DescriptorSet>, vk::Result> {
    let layouts = vec![pipeline.descriptor_set_layout; swapchain.image_views.len()];
    let info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(&layouts);

    let sets = unsafe { device.allocate_descriptor_sets(&info)? };

    for i in 0..swapchain.images.len() {
        let info = vk::DescriptorBufferInfo::default()
            .buffer(uniform_buffers.buffers[i].buffer)
            .offset(0)
            .range(size_of::<UniformBufferObject>() as u64);

        let buffer_info = &[info];
        let ubo_write = vk::WriteDescriptorSet::default()
            .dst_set(sets[i])
            .dst_binding(0)
            .dst_array_element(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .buffer_info(buffer_info);

        unsafe { device.update_descriptor_sets(&[ubo_write], &[] as &[vk::CopyDescriptorSet]) };
    }

    Ok(sets)
}
