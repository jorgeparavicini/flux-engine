//! Public entry point for the Flux rendering stack.
//!
//! Backend-agnostic types live in [`api`]; exactly one backend is
//! selected at compile time through a cargo feature (`vulkan` by default)
//! and wired up here, so consumers depend only on this crate.

pub use flux_renderer_api as api;
pub use flux_renderer_macros::Vertex;

pub use flux_renderer_api::settings::{AppInfo, AppVersion, RendererSettings};
pub use flux_renderer_api::surface::{PresentTarget, SurfaceProvider};

#[cfg(feature = "vulkan")]
pub use flux_renderer_vulkan as vulkan;

#[cfg(feature = "vulkan")]
pub use flux_renderer_vulkan::{render_schedule, shutdown_schedule, startup_schedule};

/// Prepares the world for rendering: stores the application's identity and
/// settings, then installs the active backend's initial state.
#[cfg(feature = "vulkan")]
pub fn setup(world: &mut flux_ecs::World, app: AppInfo, settings: RendererSettings) {
    world.insert_singleton(app);
    world.insert_singleton(settings);
    flux_renderer_vulkan::setup(world);
}

#[cfg(not(any(feature = "vulkan")))]
compile_error!("no renderer backend selected; enable exactly one backend feature, e.g. `vulkan`");
