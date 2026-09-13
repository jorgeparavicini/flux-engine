//! Public entry point for the Flux rendering stack.
//!
//! Backend-agnostic types live in [`abstractions`]; exactly one backend is
//! selected at compile time through a cargo feature (`vulkan` by default)
//! and wired up here, so consumers depend only on this crate.

pub use flux_renderer_abstractions as abstractions;
pub use flux_renderer_macros::Vertex;

pub use flux_renderer_abstractions::surface::{SurfaceProvider, SurfaceProviderResource};

#[cfg(feature = "vulkan")]
pub use flux_renderer_vulkan as vulkan;

#[cfg(feature = "vulkan")]
pub use flux_renderer_vulkan::{destroy_schedule, init_schedule, render_schedule, setup};

#[cfg(not(any(feature = "vulkan")))]
compile_error!("no renderer backend selected; enable exactly one backend feature, e.g. `vulkan`");
