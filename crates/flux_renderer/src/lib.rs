//! Public entry point for the Flux rendering stack.
//!
//! Backend-agnostic types live in [`abstractions`]; the active backend is
//! re-exported at the crate root so consumers depend only on `flux_renderer`.

pub use flux_renderer_abstractions as abstractions;
pub use flux_renderer_macros::Vertex;
pub use flux_renderer_vulkan as vulkan;

pub use flux_renderer_vulkan::*;
