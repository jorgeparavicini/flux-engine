use raw_window_handle::{HasRawDisplayHandle, RawDisplayHandle};
use winit::event_loop::{EventLoop};
use crate::instance::{create_instance, SurfaceProvider, SurfaceProviderResource};
use crate::surface::create_surface;
use flux_ecs::plugin::Plugin;
use flux_ecs::schedule::ScheduleLabel;
use flux_ecs::world::World;

mod instance;
mod surface;

pub struct RendererPlugin;

struct WinitSurfaceProvider {
    window: winit::window::Window,
}

impl SurfaceProvider for WinitSurfaceProvider {
    fn get_window_handle(&self) -> RawDisplayHandle {
        self.window.raw_display_handle().unwrap()
    }
}

impl Plugin for RendererPlugin {
    fn init(&self, world: &mut World) {
        let event_loop = EventLoop::new().unwrap();
        let window = event_loop.create_window(Default::default()).unwrap();
        let surface_provider = WinitSurfaceProvider { window };
        let surface_provider_resource = SurfaceProviderResource {provider: Box::new(surface_provider) };
        world.add_resource(surface_provider_resource);
        world.add_system(ScheduleLabel::Initialization, create_instance);
        world.add_system(ScheduleLabel::Initialization, create_surface);
    }
}
