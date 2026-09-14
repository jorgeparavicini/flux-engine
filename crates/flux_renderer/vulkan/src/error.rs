use crate::device::selection::SuitabilityError;
use ash::vk;
use thiserror::Error;

/// Errors surfaced by the renderer's systems.
///
/// Internal helpers keep returning [`vk::Result`]; conversion happens at the
/// system boundary. The hierarchy grows per subsystem as the layers are
/// rebuilt (device, presentation, memory).
#[derive(Debug, Error)]
pub enum RendererError {
    #[error(transparent)]
    Vulkan(#[from] vk::Result),
    #[error(transparent)]
    Suitability(#[from] SuitabilityError),
    #[error("no suitable physical device found")]
    NoSuitableDevice,
    #[error("shader error: {0}")]
    Shader(String),
}
