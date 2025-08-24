use flux_ecs::schedule::ScheduleLabel::{Destroy, Initialization, Render};
use flux_ecs::world::World;
use flux_renderer::instance::{SurfaceProvider, SurfaceProviderResource};
use flux_renderer::RendererPlugin;
use winit::event::{Event, WindowEvent};
use winit::event_loop::EventLoop;
use winit::raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle,
};

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

    fn request_redraw(&self) {
        self.window.request_redraw();
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    pretty_env_logger::init();

    let mut world = World::new();

    let event_loop = EventLoop::new().unwrap();
    let window = event_loop.create_window(Default::default()).unwrap();
    let surface_provider = WinitSurfaceProvider { window };
    let surface_provider_resource = SurfaceProviderResource {
        provider: Box::new(surface_provider),
    };
    world.add_resource(surface_provider_resource);

    world.add_plugin(RendererPlugin);
    world.run_schedule(&Initialization);

    let mut minimized = false;
    event_loop.run(move |event, elwt| match event {
        Event::AboutToWait => world
            .get_resource::<SurfaceProviderResource>()
            .unwrap()
            .request_redraw(),
        Event::WindowEvent { event, .. } => match event {
            WindowEvent::RedrawRequested if !elwt.exiting() && !minimized => {
                world.run_schedule(&Render)
            }
            WindowEvent::CloseRequested => {
                elwt.exit();
                world.run_schedule(&Destroy)
            }
            _ => {}
        },
        _ => {}
    })?;

    Ok(())
}
