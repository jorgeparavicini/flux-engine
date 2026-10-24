use crate::device::logical::RenderDevice;
use crate::error::RendererError;
use crate::pipeline::graphics::Pipeline;
use crate::present::presenter::Presenter;
use crate::resource::buffer::{UniformBufferObject, UniformBuffers};
use ash::vk;
use flux_ecs::Commands;
use flux_ecs::Single;
use log::debug;

#[derive(flux_ecs::Component)]
pub struct Descriptors {
    pub descriptor_pool: vk::DescriptorPool,
    pub descriptor_sets: Vec<vk::DescriptorSet>,
}

pub fn create_descriptors(
    device: Single<&RenderDevice>,
    pipeline: Single<&Pipeline>,
    presenter: Single<&Presenter>,
    uniform_buffer: Single<&UniformBuffers>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    let image_count = presenter.image_count();
    let pool = create_descriptor_pool(&device, image_count)?;
    let sets = create_descriptor_sets(&device, &pipeline, image_count, pool, &uniform_buffer)?;

    commands.insert_singleton(Descriptors {
        descriptor_pool: pool,
        descriptor_sets: sets,
    });

    Ok(())
}

fn create_descriptor_pool(
    device: &RenderDevice,
    image_count: usize,
) -> Result<vk::DescriptorPool, vk::Result> {
    let ubo_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::UNIFORM_BUFFER)
        .descriptor_count(image_count as u32);

    let sampler_size = vk::DescriptorPoolSize::default()
        .ty(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .descriptor_count(image_count as u32);

    let pool_sizes = &[ubo_size, sampler_size];
    let info = vk::DescriptorPoolCreateInfo::default()
        .pool_sizes(pool_sizes)
        .max_sets(image_count as u32);

    unsafe { device.create_descriptor_pool(&info, None) }
}

fn create_descriptor_sets(
    device: &RenderDevice,
    pipeline: &Pipeline,
    image_count: usize,
    pool: vk::DescriptorPool,
    uniform_buffers: &UniformBuffers,
) -> Result<Vec<vk::DescriptorSet>, vk::Result> {
    let layouts = vec![pipeline.descriptor_set_layout; image_count];
    let info = vk::DescriptorSetAllocateInfo::default()
        .descriptor_pool(pool)
        .set_layouts(&layouts);

    let sets = unsafe { device.allocate_descriptor_sets(&info)? };

    for i in 0..image_count {
        let info = vk::DescriptorBufferInfo::default()
            .buffer(uniform_buffers.buffers[i].raw())
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

pub fn destroy_descriptors(
    device: Single<&RenderDevice>,
    descriptors: Single<&Descriptors>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    debug!("Destroying descriptor pool");
    unsafe {
        device.destroy_descriptor_pool(descriptors.descriptor_pool, None);
    }

    commands.remove_singleton::<Descriptors>();

    Ok(())
}
