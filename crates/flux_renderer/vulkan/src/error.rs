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
    #[error("no suitable physical device found")]
    NoSuitableDevice,
    #[error("Vulkan loader version {found} is below the renderer's minimum {minimum}")]
    UnsupportedLoader { found: String, minimum: String },
    #[error("required instance extensions are missing: {0:?}")]
    MissingInstanceExtensions(Vec<String>),
    #[error(
        "no compatible memory type for {usage:?} within the resource's type mask {type_bits:#b}"
    )]
    NoCompatibleMemoryType {
        usage: crate::memory::allocator::MemoryUsage,
        type_bits: u32,
    },
    #[error("the staging region cannot hold another {requested} bytes this frame")]
    StagingFull { requested: u64 },
    #[error("the device supports none of the candidate depth formats")]
    NoSupportedDepthFormat,
}
