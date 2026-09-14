use flux_ecs::World;
use flux_renderer::{AppInfo, AppVersion, PresentTarget, RendererSettings, SurfaceProvider};
use winit::event::{Event, WindowEvent};
use winit::event_loop::EventLoop;
use winit::raw_window_handle::{
    HasRawDisplayHandle, HasRawWindowHandle, RawDisplayHandle, RawWindowHandle,
};

struct WinitSurfaceProvider {
    window: winit::window::Window,
}

impl SurfaceProvider for WinitSurfaceProvider {
    fn display_handle(&self) -> RawDisplayHandle {
        self.window.raw_display_handle().unwrap()
    }

    fn window_handle(&self) -> RawWindowHandle {
        self.window.raw_window_handle().unwrap()
    }

    fn extent(&self) -> (u32, u32) {
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
    let surface_provider_resource = PresentTarget {
        provider: Box::new(surface_provider),
    };
    world.insert_singleton(surface_provider_resource);

    flux_renderer::setup(
        &mut world,
        AppInfo {
            name: "Flux Sandbox".to_owned(),
            version: AppVersion {
                major: 0,
                minor: 1,
                patch: 0,
            },
        },
        RendererSettings::default(),
    );
    let mut init = flux_renderer::startup_schedule();
    let mut render = flux_renderer::render_schedule();
    let mut destroy = flux_renderer::shutdown_schedule();
    init.run(&mut world);

    let minimized = false;
    event_loop.run(move |event, elwt| match event {
        Event::AboutToWait => world.singleton::<PresentTarget>().unwrap().request_redraw(),
        Event::WindowEvent { event, .. } => match event {
            WindowEvent::RedrawRequested if !elwt.exiting() && !minimized => {
                render.run(&mut world);
            }
            WindowEvent::CloseRequested => {
                elwt.exit();
                destroy.run(&mut world);
            }
            _ => {}
        },
        _ => {}
    })?;

    Ok(())
}
