use crate::buffers::{
    create_index_buffer, create_uniform_buffer, create_vertex_buffer, destroy_buffers,
};
use crate::command_buffer::{create_command_buffer, destroy_command_buffers, CommandBuffers};
use crate::command_pool::{create_command_pools, destroy_command_pools};
use crate::depth_buffers::{create_depth_buffers, destroy_depth_buffers};
use crate::descriptors::{create_descriptors, destroy_descriptors};
use crate::device::{
    create_logical_device, create_physical_device, destroy_logical_device, Device,
};
use crate::instance::{create_instance, destroy_instance, VulkanInstance};
use crate::pipeline::{create_pipeline, destroy_pipeline};
use crate::surface::{create_surface, destroy_surface};
use crate::swapchain::{create_swapchain, destroy_swapchain, Swapchain};
use ash::vk;
use ash::vk::Handle;
use flux_ecs::commands::Commands;
use flux_ecs::plugin::Plugin;
use flux_ecs::resource::{MutRes, Res, Resource};
use flux_ecs::schedule::ScheduleLabel;
use flux_ecs::world::World;
use log::debug;
use raw_window_handle::{HasRawDisplayHandle, HasRawWindowHandle};

mod buffers;
mod command_buffer;
mod command_pool;
mod depth_buffers;
mod descriptors;
mod device;
mod image;
pub mod instance;
mod pipeline;
mod surface;
mod swapchain;

pub struct RendererPlugin;

impl Plugin for RendererPlugin {
    fn init(&self, world: &mut World) {
        world.add_default_resource::<FrameData>();
        world.add_system(ScheduleLabel::Initialization, create_instance);
        world.add_system(ScheduleLabel::Initialization, create_surface);
        world.add_system(ScheduleLabel::Initialization, create_physical_device);
        world.add_system(ScheduleLabel::Initialization, create_logical_device);
        world.add_system(ScheduleLabel::Initialization, create_swapchain);
        world.add_system(ScheduleLabel::Initialization, create_pipeline);
        world.add_system(ScheduleLabel::Initialization, create_depth_buffers);
        world.add_system(ScheduleLabel::Initialization, create_command_pools);
        world.add_system(ScheduleLabel::Initialization, create_vertex_buffer);
        world.add_system(ScheduleLabel::Initialization, create_index_buffer);
        world.add_system(ScheduleLabel::Initialization, create_uniform_buffer);
        world.add_system(ScheduleLabel::Initialization, create_descriptors);
        world.add_system(ScheduleLabel::Initialization, create_command_buffer);
        world.add_system(ScheduleLabel::Initialization, create_sync_objects);

        world.add_system(ScheduleLabel::Render, render);

        world.add_system(ScheduleLabel::Destroy, wait_device_idle);
        world.add_system(ScheduleLabel::Destroy, destroy_sync_objects);
        world.add_system(ScheduleLabel::Destroy, destroy_descriptors);
        world.add_system(ScheduleLabel::Destroy, destroy_buffers);
        world.add_system(ScheduleLabel::Destroy, destroy_command_pools);
        world.add_system(ScheduleLabel::Destroy, destroy_depth_buffers);
        world.add_system(ScheduleLabel::Destroy, destroy_pipeline);
        world.add_system(ScheduleLabel::Destroy, destroy_swapchain);
        world.add_system(ScheduleLabel::Destroy, destroy_logical_device);
        world.add_system(ScheduleLabel::Destroy, destroy_surface);
        world.add_system(ScheduleLabel::Destroy, destroy_instance);

        world.add_system(ScheduleLabel::RecreateSwapchain, wait_for_device_idle);
        world.add_system(ScheduleLabel::RecreateSwapchain, destroy_descriptors);
        world.add_system(ScheduleLabel::RecreateSwapchain, destroy_depth_buffers);
        world.add_system(ScheduleLabel::RecreateSwapchain, destroy_command_buffers);
        world.add_system(ScheduleLabel::RecreateSwapchain, destroy_command_pools);
        world.add_system(ScheduleLabel::RecreateSwapchain, destroy_pipeline);
        world.add_system(ScheduleLabel::RecreateSwapchain, destroy_swapchain);
        world.add_system(ScheduleLabel::RecreateSwapchain, create_swapchain);
        world.add_system(ScheduleLabel::RecreateSwapchain, create_pipeline);
        world.add_system(ScheduleLabel::RecreateSwapchain, create_depth_buffers);
        world.add_system(ScheduleLabel::RecreateSwapchain, create_command_pools);
        world.add_system(ScheduleLabel::RecreateSwapchain, create_descriptors);
        world.add_system(ScheduleLabel::RecreateSwapchain, create_command_buffer);
    }
}

pub struct SyncObjects {
    pub image_available_semaphores: Vec<vk::Semaphore>,
    pub render_finished_semaphores: Vec<vk::Semaphore>,
    pub in_flight_fences: Vec<vk::Fence>,
    pub images_in_flight: Vec<vk::Fence>,
}

impl Resource for SyncObjects {}

fn create_sync_objects(
    device: Res<Device>,
    swapchain: Res<Swapchain>,
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

    commands.insert_resource(SyncObjects {
        image_available_semaphores,
        render_finished_semaphores,
        in_flight_fences,
        images_in_flight,
    });

    Ok(())
}

#[derive(Default)]
pub struct FrameData {
    pub frame_index: usize,
}

impl Resource for FrameData {}

pub fn render(
    instance: Res<VulkanInstance>,
    device: Res<Device>,
    swapchain: Res<Swapchain>,
    command_buffers_res: Res<CommandBuffers>,
    mut sync_objects: MutRes<SyncObjects>,
    mut frame_data: MutRes<FrameData>,
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
        commands.run_schedule_once(ScheduleLabel::RecreateSwapchain);
    } else if let Err(e) = result {
        return Err(e);
    }

    frame_data.frame_index = (frame_data.frame_index + 1) % swapchain.max_frames_in_flight;

    Ok(())
}

fn wait_device_idle(device: Res<Device>) {
    unsafe {
        device
            .device_wait_idle()
            .expect("Failed to wait device idle");
    }
}

fn destroy_sync_objects(
    device: Res<Device>,
    sync_objects: Res<SyncObjects>,
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

    commands.remove_resource::<SyncObjects>();
}

fn wait_for_device_idle(device: Res<Device>) -> Result<(), vk::Result> {
    unsafe { device.device_wait_idle()? }

    Ok(())
}
