//! The baseline allocation strategy: one `vk::DeviceMemory` per request.
//! Trivially correct, measurably wasteful, and the permanent fallback for
//! allocations that suballocation cannot serve.

use std::ptr::NonNull;

use ash::vk;
use log::{debug, warn};

use crate::device::logical::RenderDevice;
use crate::error::RendererError;
use crate::memory::allocator::{AllocRequest, Allocation, GpuAllocator, choose_memory_type};

/// Allocates one device-memory object per request. Host-visible allocations
/// are mapped once, persistently. Every allocation must come back through
/// [`GpuAllocator::free`]; dropping the allocator with live allocations
/// leaks them and logs how many.
pub struct DedicatedAllocator {
    device: ash::Device,
    memory_properties: vk::PhysicalDeviceMemoryProperties,
    allocations: u32,
}

impl DedicatedAllocator {
    pub fn new(device: &RenderDevice) -> Self {
        Self {
            device: device.raw_clone(),
            memory_properties: device.capabilities.memory,
            allocations: 0,
        }
    }
}

impl GpuAllocator for DedicatedAllocator {
    fn alloc(&mut self, request: &AllocRequest) -> Result<Allocation, RendererError> {
        let Some(memory_type) = choose_memory_type(
            &self.memory_properties,
            request.memory_type_bits,
            request.usage,
        ) else {
            return Err(RendererError::NoCompatibleMemoryType {
                usage: request.usage,
                type_bits: request.memory_type_bits,
            });
        };

        let memory_info = vk::MemoryAllocateInfo::default()
            .allocation_size(request.size)
            .memory_type_index(memory_type);

        let memory = unsafe { self.device.allocate_memory(&memory_info, None)? };

        let host_visible = self.memory_properties.memory_types[memory_type as usize]
            .property_flags
            .contains(vk::MemoryPropertyFlags::HOST_VISIBLE);
        let mapped = if host_visible {
            let ptr = unsafe {
                self.device
                    .map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
            };
            match ptr {
                Ok(ptr) => NonNull::new(ptr.cast::<u8>()),
                Err(e) => {
                    // A half-built allocation must not outlive its error.
                    unsafe { self.device.free_memory(memory, None) };
                    return Err(e.into());
                }
            }
        } else {
            None
        };

        self.allocations += 1;
        debug!(
            "Allocated {} bytes of memory type {memory_type} for {:?} ({} live)",
            request.size, request.usage, self.allocations
        );

        Ok(Allocation {
            memory,
            offset: 0,
            size: request.size,
            mapped,
        })
    }

    unsafe fn free(&mut self, allocation: Allocation) {
        if allocation.mapped.is_some() {
            unsafe {
                self.device.unmap_memory(allocation.memory);
            }
        }

        unsafe {
            self.device.free_memory(allocation.memory, None);
        }

        self.allocations -= 1;
    }
}

impl DedicatedAllocator {
    /// Allocations handed out and not yet freed.
    #[must_use]
    pub const fn live(&self) -> u32 {
        self.allocations
    }
}

impl Drop for DedicatedAllocator {
    fn drop(&mut self) {
        if self.allocations > 0 {
            warn!(
                "DedicatedAllocator dropped with {} live allocations; their device memory is leaked",
                self.allocations
            );
        }
    }
}
