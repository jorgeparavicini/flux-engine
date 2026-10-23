use crate::device::instance::{create_instance, destroy_instance};
use crate::device::logical::{create_render_device, destroy_render_device};
use crate::memory::gpu_memory::{create_gpu_memory, destroy_gpu_memory};
use crate::pipeline::descriptors::{create_descriptors, destroy_descriptors};
use crate::pipeline::graphics::{create_pipeline, destroy_pipeline};
use crate::present::presenter::{create_presenter, destroy_presenter, rebuild_presenter};
use crate::present::surface::create_surface;
use crate::render::commands::{
    create_command_buffer, create_command_pools, destroy_command_buffers, destroy_command_pools,
};
use crate::render::frame::{
    PresentationOutdated, clear_presentation_outdated, presentation_outdated, render,
    wait_device_idle,
};
use crate::render::mesh::{CoolVertex, MeshComponent, create_buffers};
use crate::resource::buffer::{create_uniform_buffer, destroy_buffers};
use crate::resource::depth::{create_depth_buffers, destroy_depth_buffers};
use flux_ecs::{Schedule, World};
use flux_renderer_api::mesh::Mesh;

/// Inserts the renderer's initial state: the demo mesh and the presentation
/// flag.
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
        indices: Some(vec![0, 2, 1]),
    };
    world.spawn((MeshComponent(mesh),));
    world.insert_singleton(PresentationOutdated(false));
}

/// Creates every Vulkan resource, in dependency order.
pub fn startup_schedule() -> Schedule {
    let mut schedule = Schedule::new();
    schedule.add(create_instance);
    schedule.add(create_surface);
    schedule.add(create_render_device);
    schedule.add(create_gpu_memory);
    schedule.add(create_presenter);
    schedule.add(create_pipeline);
    schedule.add(create_depth_buffers);
    schedule.add(create_command_pools);
    schedule.add(create_uniform_buffer);
    schedule.add(create_descriptors);
    schedule.add(create_command_buffer);
    schedule.add(create_buffers);
    schedule
}

/// Renders one frame. When the presentation is out of date, the presenter is
/// rebuilt and its dependents are recreated in the same pass, gated on
/// [`PresentationOutdated`].
pub fn render_schedule() -> Schedule {
    let mut schedule = Schedule::new();
    schedule.add(render);
    schedule
        .add(rebuild_presenter)
        .run_if(presentation_outdated);
    schedule
        .add(destroy_descriptors)
        .run_if(presentation_outdated);
    schedule
        .add(destroy_depth_buffers)
        .run_if(presentation_outdated);
    schedule
        .add(destroy_command_buffers)
        .run_if(presentation_outdated);
    schedule.add(destroy_pipeline).run_if(presentation_outdated);
    schedule.add(create_pipeline).run_if(presentation_outdated);
    schedule
        .add(create_depth_buffers)
        .run_if(presentation_outdated);
    schedule
        .add(create_descriptors)
        .run_if(presentation_outdated);
    schedule
        .add(create_command_buffer)
        .run_if(presentation_outdated);
    schedule
        .add(clear_presentation_outdated)
        .run_if(presentation_outdated);
    schedule
}

/// Tears every Vulkan resource down, in reverse dependency order.
pub fn shutdown_schedule() -> Schedule {
    let mut schedule = Schedule::new();
    schedule.add(wait_device_idle);
    schedule.add(destroy_descriptors);
    schedule.add(destroy_buffers);
    schedule.add(destroy_command_pools);
    schedule.add(destroy_depth_buffers);
    schedule.add(destroy_pipeline);
    schedule.add(destroy_presenter);
    schedule.add(destroy_gpu_memory);
    schedule.add(destroy_render_device);
    schedule.add(destroy_instance);
    schedule
}
