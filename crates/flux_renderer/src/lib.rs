//! Public entry point for the Flux rendering stack.
//!
//! Backend-agnostic types live in [`api`]; exactly one backend is
//! selected at compile time through a cargo feature (`vulkan` by default)
//! and wired up here, so consumers depend only on this crate.

pub use flux_renderer_api as api;
pub use flux_renderer_macros::Vertex;

pub use flux_renderer_api::surface::{PresentTarget, SurfaceProvider};

#[cfg(feature = "vulkan")]
pub use flux_renderer_vulkan as vulkan;

#[cfg(feature = "vulkan")]
pub use flux_renderer_vulkan::{render_schedule, setup, shutdown_schedule, startup_schedule};

#[cfg(not(any(feature = "vulkan")))]
compile_error!("no renderer backend selected; enable exactly one backend feature, e.g. `vulkan`");
