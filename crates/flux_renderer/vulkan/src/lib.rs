use crate::buffers::{
    create_index_buffer, create_uniform_buffer, create_vertex_buffer, destroy_buffers,
};
use crate::command_buffer::{create_command_buffer, destroy_command_buffers, CommandBuffers};
use crate::command_pool::{create_command_pools, destroy_command_pools};
use crate::depth_buffers::{create_depth_buffers, destroy_depth_buffers, DepthBuffers};
use crate::descriptors::{create_descriptors, destroy_descriptors, Descriptors};
use crate::device::{
    create_logical_device, create_physical_device, destroy_logical_device, Device,
};
use crate::instance::{create_instance, destroy_instance, VulkanInstance};
use crate::mesh::{create_buffers, CoolVertex, MeshComponent, VulkanMesh};
use crate::pipeline::{create_pipeline, destroy_pipeline, Pipeline};
use crate::surface::{create_surface, destroy_surface};
use crate::swapchain::{create_swapchain, destroy_swapchain, Swapchain};
use ash::vk;
use ash::vk::{Handle, IndexType};
use flux_ecs::Single;
use flux_ecs::Commands;
use flux_ecs::Query;
use flux_ecs::{Schedule, World};
use flux_renderer_abstractions::mesh::Mesh;
use log::debug;

mod buffers;
mod command_buffer;
mod command_pool;
mod depth_buffers;
mod descriptors;
mod device;
mod image;
pub mod instance;
mod mesh;
mod pipeline;
mod surface;
mod swapchain;

/// True while the swapchain no longer matches the surface; the recreation
/// systems in the render schedule run while set.
#[derive(flux_ecs::Component, Default)]
pub struct SwapchainOutdated(pub bool);

fn swapchain_outdated(world: &World) -> bool {
    world
        .singleton::<SwapchainOutdated>()
        .is_some_and(|flag| flag.0)
}

fn clear_swapchain_outdated(mut flag: Single<&mut SwapchainOutdated>) {
    flag.0 = false;
}

/// Inserts the renderer's initial state: the demo mesh and per-frame data.
pub fn setup(world: &mut World) {
    let mesh = Mesh {
        vertices: vec![
            CoolVertex {
                position: [0.0, -0.5, 0.0],
                color: [1.0, 1.0, 1.0],
            },
            CoolVertex {
                position: [0.5, 0.5, 0.0],
                color: [1.0, 1.0, 1.0],
            },
            CoolVertex {
                position: [-0.5, 0.5, 0.0],
                color: [1.0, 1.0, 1.0],
            },
        ],
        indices: Some(vec![1, 0, 2]),
    };
    world.spawn((MeshComponent(mesh),));
    world.insert_singleton(FrameData::default());
    world.insert_singleton(SwapchainOutdated(false));
}

/// Creates every Vulkan resource, in dependency order.
pub fn init_schedule() -> Schedule {
    let mut schedule = Schedule::new();
    schedule.add(create_instance);
    schedule.add(create_surface);
    schedule.add(create_physical_device);
    schedule.add(create_logical_device);
    schedule.add(create_swapchain);
    schedule.add(create_pipeline);
    schedule.add(create_depth_buffers);
    schedule.add(create_command_pools);
    schedule.add(create_vertex_buffer);
    schedule.add(create_index_buffer);
    schedule.add(create_uniform_buffer);
    schedule.add(create_descriptors);
    schedule.add(create_command_buffer);
    schedule.add(create_sync_objects);
    schedule.add(create_buffers);
    schedule
}

/// Renders one frame. When the swapchain is out of date, the recreation
/// systems run in the same pass, gated on [`SwapchainOutdated`].
pub fn render_schedule() -> Schedule {
    let mut schedule = Schedule::new();
    schedule.add(render);
    schedule.add(wait_for_device_idle).run_if(swapchain_outdated);
    schedule.add(destroy_descriptors).run_if(swapchain_outdated);
    schedule.add(destroy_depth_buffers).run_if(swapchain_outdated);
    schedule.add(destroy_command_buffers).run_if(swapchain_outdated);
    schedule.add(destroy_command_pools).run_if(swapchain_outdated);
    schedule.add(destroy_pipeline).run_if(swapchain_outdated);
    schedule.add(destroy_swapchain).run_if(swapchain_outdated);
    schedule.add(create_swapchain).run_if(swapchain_outdated);
    schedule.add(create_pipeline).run_if(swapchain_outdated);
    schedule.add(create_depth_buffers).run_if(swapchain_outdated);
    schedule.add(create_command_pools).run_if(swapchain_outdated);
    schedule.add(create_descriptors).run_if(swapchain_outdated);
    schedule.add(create_command_buffer).run_if(swapchain_outdated);
    schedule.add(clear_swapchain_outdated).run_if(swapchain_outdated);
    schedule
}

/// Tears every Vulkan resource down, in reverse dependency order.
pub fn destroy_schedule() -> Schedule {
    let mut schedule = Schedule::new();
    schedule.add(wait_device_idle);
    schedule.add(destroy_sync_objects);
    schedule.add(destroy_descriptors);
    schedule.add(destroy_buffers);
    schedule.add(destroy_command_pools);
    schedule.add(destroy_depth_buffers);
    schedule.add(destroy_pipeline);
    schedule.add(destroy_swapchain);
    schedule.add(destroy_logical_device);
    schedule.add(destroy_surface);
    schedule.add(destroy_instance);
    schedule
}

#[derive(flux_ecs::Component)]
pub struct SyncObjects {
    pub image_available_semaphores: Vec<vk::Semaphore>,
    pub render_finished_semaphores: Vec<vk::Semaphore>,
    pub in_flight_fences: Vec<vk::Fence>,
    pub images_in_flight: Vec<vk::Fence>,
}


fn create_sync_objects(
    device: Single<&Device>,
    swapchain: Single<&Swapchain>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    let semaphore_info = vk::SemaphoreCreateInfo::default();
    let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);

    let mut image_available_semaphores = Vec::with_capacity(swapchain.max_frames_in_flight);
    let mut in_flight_fences = Vec::with_capacity(swapchain.max_frames_in_flight);

    for _ in 0..swapchain.max_frames_in_flight {
        image_available_semaphores.push(unsafe { device.create_semaphore(&semaphore_info, None)? });
        in_flight_fences.push(unsafe { device.create_fence(&fence_info, None)? });
    }

    let mut render_finished_semaphores = Vec::with_capacity(swapchain.max_frames_in_flight);
    for _ in 0..swapchain.images.len() {
        render_finished_semaphores.push(unsafe { device.create_semaphore(&semaphore_info, None)? });
    }

    let images_in_flight = swapchain.images.iter().map(|_| vk::Fence::null()).collect();

    commands.insert_singleton(SyncObjects {
        image_available_semaphores,
        render_finished_semaphores,
        in_flight_fences,
        images_in_flight,
    });

    Ok(())
}

#[derive(Default)]
#[derive(flux_ecs::Component)]
pub struct FrameData {
    pub frame_index: usize,
}


pub fn render(
    instance: Single<&VulkanInstance>,
    device: Single<&Device>,
    swapchain: Single<&Swapchain>,
    command_buffers_res: Single<&CommandBuffers>,
    depth_buffers: Single<&DepthBuffers>,
    pipeline: Single<&Pipeline>,
    meshes: Query<&VulkanMesh>,
    descriptors: Single<&Descriptors>,
    mut sync_objects: Single<&mut SyncObjects>,
    mut frame_data: Single<&mut FrameData>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    unsafe {
        device.wait_for_fences(
            &[sync_objects.in_flight_fences[frame_data.frame_index]],
            true,
            u64::MAX,
        )?;
    }

    let swapchain_device = ash::khr::swapchain::Device::new(&instance, &device);
    let image_available_semaphore = sync_objects.image_available_semaphores[frame_data.frame_index];
    let next_image_result = unsafe {
        swapchain_device.acquire_next_image(
            **swapchain,
            u64::MAX,
            image_available_semaphore,
            vk::Fence::null(),
        )
    };

    let image_index = match next_image_result {
        Ok((image_index, _)) => image_index as usize,
        Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return Ok(()), // TODO Recreate swapchain
        Err(e) => return Err(e),
    };

    if !sync_objects.images_in_flight[image_index].is_null() {
        unsafe {
            device.wait_for_fences(
                &[sync_objects.images_in_flight[image_index]],
                true,
                u64::MAX,
            )?;
        }
    }

    sync_objects.images_in_flight[image_index] =
        sync_objects.in_flight_fences[frame_data.frame_index];

    let command_buffer = command_buffers_res.command_buffers[image_index];

    record_command_buffer(
        &device,
        &command_buffer,
        &swapchain,
        image_index,
        &depth_buffers,
        &pipeline,
        meshes,
        descriptors,
    )?;

    let wait_semaphores = &[image_available_semaphore];
    let wait_stages = &[vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
    let command_buffers = &[command_buffers_res.command_buffers[image_index]];
    let signal_semaphores = &[sync_objects.render_finished_semaphores[image_index]];
    let submit_info = vk::SubmitInfo::default()
        .wait_semaphores(wait_semaphores)
        .wait_dst_stage_mask(wait_stages)
        .command_buffers(command_buffers)
        .signal_semaphores(signal_semaphores);

    let in_flight_fence = sync_objects.in_flight_fences[frame_data.frame_index];
    unsafe {
        device.reset_fences(&[in_flight_fence])?;
        device.queue_submit(device.graphics_queue, &[submit_info], in_flight_fence)?;
    }

    let swapchains = &[swapchain.swapchain];
    let image_indices = &[image_index as u32];
    let present_info = vk::PresentInfoKHR::default()
        .wait_semaphores(signal_semaphores)
        .swapchains(swapchains)
        .image_indices(image_indices);

    let result = unsafe { swapchain_device.queue_present(device.present_queue, &present_info) };

    let changed = result == Ok(true) || result == Err(vk::Result::ERROR_OUT_OF_DATE_KHR);

    // TODO: Probably should handle explicit resizes here as well
    if changed {
        commands.insert_singleton(SwapchainOutdated(true));
    } else {
        result?;
    }

    frame_data.frame_index = (frame_data.frame_index + 1) % swapchain.max_frames_in_flight;

    Ok(())
}

fn wait_device_idle(device: Single<&Device>) {
    unsafe {
        device
            .device_wait_idle()
            .expect("Failed to wait device idle");
    }
}

fn destroy_sync_objects(
    device: Single<&Device>,
    sync_objects: Single<&SyncObjects>,
    mut commands: Commands,
) {
    debug!("Destroying sync objects");

    for &semaphore in &sync_objects.image_available_semaphores {
        unsafe {
            device.destroy_semaphore(semaphore, None);
        }
    }

    for &semaphore in &sync_objects.render_finished_semaphores {
        unsafe {
            device.destroy_semaphore(semaphore, None);
        }
    }

    for &fence in &sync_objects.in_flight_fences {
        unsafe {
            device.destroy_fence(fence, None);
        }
    }

    commands.remove_singleton::<SyncObjects>();
}

fn wait_for_device_idle(device: Single<&Device>) -> Result<(), vk::Result> {
    unsafe { device.device_wait_idle()? }

    Ok(())
}

fn record_command_buffer(
    device: &Single<&Device>,
    command_buffer: &vk::CommandBuffer,
    swapchain: &Single<&Swapchain>,
    image_index: usize,
    depth_buffers: &Single<&DepthBuffers>,
    pipeline: &Single<&Pipeline>,
    meshes: Query<&VulkanMesh>,
    descriptors: Single<&Descriptors>,
) -> Result<(), vk::Result> {
    unsafe {
        let inheritance = vk::CommandBufferInheritanceInfo::default();

        let info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::empty())
            .inheritance_info(&inheritance);

        device.begin_command_buffer(*command_buffer, &info)?;

        let image_barrier_to_render = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::empty()) // No need to wait for previous operations
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE) // We are preparing to write
            .old_layout(vk::ImageLayout::UNDEFINED) // We don't care about the previous layout/contents
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL) // Layout needed for rendering
            .image(swapchain.images[image_index])
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .level_count(1)
                    .layer_count(1),
            );

        device.cmd_pipeline_barrier(
            *command_buffer,
            vk::PipelineStageFlags::TOP_OF_PIPE, // Source stage
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT, // Destination stage
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[image_barrier_to_render],
        );

        let render_area = vk::Rect2D::default()
            .offset(vk::Offset2D::default())
            .extent(swapchain.extent);

        let color_clear_value = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            },
        };

        let depth_clear_value = vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.0,
                stencil: 0,
            },
        };

        // TODO: Compare values with framebuffer attachments
        let color_attachment_info = vk::RenderingAttachmentInfo::default()
            .image_view(swapchain.image_views[image_index])
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(color_clear_value);

        let depth_attachment_info = vk::RenderingAttachmentInfo::default()
            .image_view(depth_buffers.depth_image_view)
            .image_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(depth_clear_value);

        let color_attachments = &[color_attachment_info];
        let rendering_info = vk::RenderingInfo::default()
            .render_area(render_area)
            .layer_count(1)
            .color_attachments(color_attachments)
            .depth_attachment(&depth_attachment_info);

        device.cmd_begin_rendering(*command_buffer, &rendering_info);
        device.cmd_bind_pipeline(
            *command_buffer,
            vk::PipelineBindPoint::GRAPHICS,
            ***pipeline,
        );

        device.cmd_bind_descriptor_sets(
            *command_buffer,
            vk::PipelineBindPoint::GRAPHICS,
            pipeline.pipeline_layout,
            0,
            &[descriptors.descriptor_sets[image_index]],
            &[],
        );

        meshes.for_each(|mesh| {
            device.cmd_bind_vertex_buffers(*command_buffer, 0, &[mesh.vertex_buffer], &[0]);
            device.cmd_bind_index_buffer(*command_buffer, mesh.index_buffer, 0, IndexType::UINT32);

            device.cmd_draw(*command_buffer, mesh.num_indices, 1, 0, 0);
        });

        device.cmd_end_rendering(*command_buffer);

        let image_barrier_to_present = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::empty())
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .image(swapchain.images[image_index])
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .level_count(1)
                    .layer_count(1),
            );

        device.cmd_pipeline_barrier(
            *command_buffer,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[image_barrier_to_present],
        );

        device.end_command_buffer(*command_buffer)?;

        Ok(())
    }
}
