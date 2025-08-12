use crate::command_pool::{create_command_pools, destroy_command_pools};
use crate::device::{create_logical_device, create_physical_device, destroy_logical_device};
use crate::instance::{
    SurfaceProvider, SurfaceProviderResource, create_instance, destroy_instance,
};
use crate::pipeline::{create_pipeline, destroy_pipeline};
use crate::surface::{create_surface, destroy_surface};
use crate::swapchain::{create_swapchain, destroy_swapchain};
use flux_ecs::plugin::Plugin;
use flux_ecs::schedule::ScheduleLabel;
use flux_ecs::world::World;
use raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use winit::event_loop::EventLoop;

mod command_pool;
mod device;
mod instance;
mod pipeline;
mod surface;
mod swapchain;

pub struct RendererPlugin;

struct WinitSurfaceProvider {
    window: winit::window::Window,
}

impl SurfaceProvider for WinitSurfaceProvider {
    fn get_display_handle(&self) -> RawDisplayHandle {
        self.window.raw_display_handle().unwrap()
    }

    fn get_window_handle(&self) -> RawWindowHandle {
        self.window.raw_window_handle().unwrap()
    }

    fn get_extent(&self) -> (u32, u32) {
        let size = self.window.inner_size();
        (size.width, size.height)
    }
}

impl Plugin for RendererPlugin {
    fn init(&self, world: &mut World) {
        let event_loop = EventLoop::new().unwrap();
        let window = event_loop.create_window(Default::default()).unwrap();
        let surface_provider = WinitSurfaceProvider { window };
        let surface_provider_resource = SurfaceProviderResource {
            provider: Box::new(surface_provider),
        };
        world.add_resource(surface_provider_resource);
        world.add_system(ScheduleLabel::Initialization, create_instance);
        world.add_system(ScheduleLabel::Initialization, create_surface);
        world.add_system(ScheduleLabel::Initialization, create_physical_device);
        world.add_system(ScheduleLabel::Initialization, create_logical_device);
        world.add_system(ScheduleLabel::Initialization, create_swapchain);
        world.add_system(ScheduleLabel::Initialization, create_pipeline);
        world.add_system(ScheduleLabel::Initialization, create_command_pools);

        world.add_system(ScheduleLabel::Destroy, destroy_command_pools);
        world.add_system(ScheduleLabel::Destroy, destroy_pipeline);
        world.add_system(ScheduleLabel::Destroy, destroy_swapchain);
        world.add_system(ScheduleLabel::Destroy, destroy_logical_device);
        world.add_system(ScheduleLabel::Destroy, destroy_surface);
        world.add_system(ScheduleLabel::Destroy, destroy_instance);
    }
}
