//! The allocator contract: how the renderer asks for GPU memory, and the
//! usage-driven choice of memory type. Allocation strategies implement
//! [`GpuAllocator`]; nothing here calls Vulkan.

use crate::error::RendererError;
use ash::vk;
use std::ptr::NonNull;

/// How an allocation will be accessed; drives the memory-type choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryUsage {
    /// GPU-only data (meshes, render targets): device-local, never mapped.
    GpuOnly,
    /// CPU writes, GPU reads (staging, per-frame uniforms): host-visible,
    /// device-local when the hardware offers it.
    CpuToGpu,
    /// GPU writes, CPU reads (screenshots, queries): host-visible, cached
    /// when the hardware offers it.
    Readback,
}

/// One request for memory, independent of the strategy serving it.
#[derive(Debug, Clone, Copy)]
pub struct AllocRequest {
    pub size: u64,
    pub alignment: u64,
    /// `vk::MemoryRequirements::memory_type_bits` of the resource.
    pub memory_type_bits: u32,
    pub usage: MemoryUsage,
}

/// A block of device memory serving one request.
pub struct Allocation {
    pub memory: vk::DeviceMemory,
    pub offset: u64,
    pub size: u64,
    /// Persistent CPU pointer for host-visible allocations.
    pub mapped: Option<NonNull<u8>>,
}

// SAFETY: the mapped pointer is uniquely owned by the allocation and only
// dereferenced through exclusive access to the owning resource.
unsafe impl Send for Allocation {}
// SAFETY: a shared allocation only ever reads the pointer value itself.
unsafe impl Sync for Allocation {}

/// An allocation strategy: hands out and takes back GPU memory.
pub trait GpuAllocator {
    fn alloc(&mut self, request: &AllocRequest) -> Result<Allocation, RendererError>;

    /// # Safety
    ///
    /// The allocation must come from this allocator and must no longer be in
    /// use by the GPU.
    unsafe fn free(&mut self, allocation: Allocation);
}

/// Picks the memory type for a request: among the types the resource allows,
/// the first satisfying the usage's required properties plus its preferences,
/// falling back to the required properties alone.
pub fn choose_memory_type(
    properties: &vk::PhysicalDeviceMemoryProperties,
    memory_type_bits: u32,
    usage: MemoryUsage,
) -> Option<u32> {
    let (required, preferred) = match usage {
        MemoryUsage::GpuOnly => (
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            vk::MemoryPropertyFlags::empty(),
        ),
        MemoryUsage::CpuToGpu => (
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        ),
        MemoryUsage::Readback => (
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            vk::MemoryPropertyFlags::HOST_CACHED,
        ),
    };

    [required | preferred, required]
        .into_iter()
        .find_map(|wanted| first_matching_type(properties, memory_type_bits, wanted))
}

fn first_matching_type(
    properties: &vk::PhysicalDeviceMemoryProperties,
    memory_type_bits: u32,
    wanted: vk::MemoryPropertyFlags,
) -> Option<u32> {
    properties
        .memory_types_as_slice()
        .iter()
        .enumerate()
        .find(|(index, memory_type)| {
            memory_type_bits & (1 << index) != 0 && memory_type.property_flags.contains(wanted)
        })
        .map(|(index, _)| index as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE_LOCAL: vk::MemoryPropertyFlags = vk::MemoryPropertyFlags::DEVICE_LOCAL;

    fn memory_type(flags: vk::MemoryPropertyFlags) -> vk::MemoryType {
        vk::MemoryType {
            property_flags: flags,
            heap_index: 0,
        }
    }

    /// A discrete-GPU-shaped layout:
    /// 0: device-local only, 1: host-visible + coherent,
    /// 2: device-local + host-visible + coherent (BAR),
    /// 3: host-visible + coherent + cached.
    fn discrete() -> vk::PhysicalDeviceMemoryProperties {
        let mut properties = vk::PhysicalDeviceMemoryProperties {
            memory_type_count: 4,
            ..Default::default()
        };
        properties.memory_types[0] = memory_type(DEVICE_LOCAL);
        properties.memory_types[1] = memory_type(
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        properties.memory_types[2] = memory_type(
            DEVICE_LOCAL
                | vk::MemoryPropertyFlags::HOST_VISIBLE
                | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        properties.memory_types[3] = memory_type(
            vk::MemoryPropertyFlags::HOST_VISIBLE
                | vk::MemoryPropertyFlags::HOST_COHERENT
                | vk::MemoryPropertyFlags::HOST_CACHED,
        );
        properties
    }

    /// Apple/integrated-shaped: one type with everything.
    fn unified() -> vk::PhysicalDeviceMemoryProperties {
        let mut properties = vk::PhysicalDeviceMemoryProperties {
            memory_type_count: 1,
            ..Default::default()
        };
        properties.memory_types[0] = memory_type(
            DEVICE_LOCAL
                | vk::MemoryPropertyFlags::HOST_VISIBLE
                | vk::MemoryPropertyFlags::HOST_COHERENT,
        );
        properties
    }

    const ALL_TYPES: u32 = u32::MAX;

    #[test]
    fn gpu_only_takes_pure_device_local() {
        assert_eq!(
            choose_memory_type(&discrete(), ALL_TYPES, MemoryUsage::GpuOnly),
            Some(0)
        );
    }

    #[test]
    fn cpu_to_gpu_prefers_bar_memory() {
        // Device-local AND host-visible beats plain host-visible.
        assert_eq!(
            choose_memory_type(&discrete(), ALL_TYPES, MemoryUsage::CpuToGpu),
            Some(2)
        );
    }

    #[test]
    fn cpu_to_gpu_falls_back_to_host_visible() {
        // Resource compatible with everything except the BAR type.
        let mask = ALL_TYPES & !(1 << 2);
        assert_eq!(
            choose_memory_type(&discrete(), mask, MemoryUsage::CpuToGpu),
            Some(1)
        );
    }

    #[test]
    fn readback_prefers_cached_memory() {
        assert_eq!(
            choose_memory_type(&discrete(), ALL_TYPES, MemoryUsage::Readback),
            Some(3)
        );
    }

    #[test]
    fn the_resource_type_mask_is_law() {
        // Only type 1 is acceptable to the resource.
        assert_eq!(
            choose_memory_type(&discrete(), 1 << 1, MemoryUsage::GpuOnly),
            None,
            "a required property missing from every masked type is a failure"
        );
    }

    #[test]
    fn unified_memory_serves_every_usage() {
        for usage in [
            MemoryUsage::GpuOnly,
            MemoryUsage::CpuToGpu,
            MemoryUsage::Readback,
        ] {
            assert_eq!(choose_memory_type(&unified(), ALL_TYPES, usage), Some(0));
        }
    }
}
