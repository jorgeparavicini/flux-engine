use crate::device::instance::{create_instance, destroy_instance};
use crate::device::logical::{create_logical_device, destroy_logical_device};
use crate::device::selection::create_physical_device;
use crate::pipeline::descriptors::{create_descriptors, destroy_descriptors};
use crate::pipeline::graphics::{create_pipeline, destroy_pipeline};
use crate::present::surface::{create_surface, destroy_surface};
use crate::present::swapchain::{create_swapchain, destroy_swapchain};
use crate::render::commands::{
    create_command_buffer, create_command_pools, destroy_command_buffers, destroy_command_pools,
};
use crate::render::frame::{
    FrameData, SwapchainOutdated, clear_swapchain_outdated, create_sync_objects,
    destroy_sync_objects, render, swapchain_outdated, wait_device_idle, wait_for_device_idle,
};
use crate::render::mesh::{CoolVertex, MeshComponent, create_buffers};
use crate::resource::buffer::{
    create_index_buffer, create_uniform_buffer, create_vertex_buffer, destroy_buffers,
};
use crate::resource::depth::{create_depth_buffers, destroy_depth_buffers};
use flux_ecs::{Schedule, World};
use flux_renderer_api::mesh::Mesh;

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
pub fn startup_schedule() -> Schedule {
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
    schedule
        .add(wait_for_device_idle)
        .run_if(swapchain_outdated);
    schedule.add(destroy_descriptors).run_if(swapchain_outdated);
    schedule
        .add(destroy_depth_buffers)
        .run_if(swapchain_outdated);
    schedule
        .add(destroy_command_buffers)
        .run_if(swapchain_outdated);
    schedule
        .add(destroy_command_pools)
        .run_if(swapchain_outdated);
    schedule.add(destroy_pipeline).run_if(swapchain_outdated);
    schedule.add(destroy_swapchain).run_if(swapchain_outdated);
    schedule.add(create_swapchain).run_if(swapchain_outdated);
    schedule.add(create_pipeline).run_if(swapchain_outdated);
    schedule
        .add(create_depth_buffers)
        .run_if(swapchain_outdated);
    schedule
        .add(create_command_pools)
        .run_if(swapchain_outdated);
    schedule.add(create_descriptors).run_if(swapchain_outdated);
    schedule
        .add(create_command_buffer)
        .run_if(swapchain_outdated);
    schedule
        .add(clear_swapchain_outdated)
        .run_if(swapchain_outdated);
    schedule
}

/// Tears every Vulkan resource down, in reverse dependency order.
pub fn shutdown_schedule() -> Schedule {
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
