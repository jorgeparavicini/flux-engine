//! An owned Vulkan buffer: created bound to memory, destroyed through the
//! retire queue when dropped.

use std::ptr;

use crate::device::logical::RenderDevice;
use crate::error::RendererError;
use crate::memory::allocator::{AllocRequest, Allocation, GpuAllocator, MemoryUsage};
use crate::memory::gpu_memory::GpuMemory;
use crate::resource::retire::{Retired, Retirer};
use ash::vk;

/// A buffer and the memory backing it. Dropping retires both; the GPU may
/// keep using them until the frame that dropped them provably finished.
pub struct GpuBuffer {
    raw: vk::Buffer,
    allocation: Option<Allocation>,
    retirer: Retirer,
    size: u64,
}

impl GpuBuffer {
    /// Creates a buffer of `size` bytes bound to freshly allocated memory,
    /// named for captures and validation messages.
    pub fn new(
        device: &RenderDevice,
        memory: &mut GpuMemory,
        size: u64,
        buffer_usage: vk::BufferUsageFlags,
        memory_usage: MemoryUsage,
        name: &str,
    ) -> Result<Self, RendererError> {
        let buffer_info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(buffer_usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);

        let buffer = unsafe { device.create_buffer(&buffer_info, None)? };
        let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };

        // Nothing here has reached the GPU yet, so failure unwinds destroy
        // immediately instead of going through the retire queue.
        let allocation = match memory.allocator.alloc(&AllocRequest {
            size: requirements.size,
            alignment: requirements.alignment,
            memory_type_bits: requirements.memory_type_bits,
            usage: memory_usage,
        }) {
            Ok(allocation) => allocation,
            Err(e) => {
                unsafe { device.destroy_buffer(buffer, None) };
                return Err(e);
            }
        };

        if let Err(e) =
            unsafe { device.bind_buffer_memory(buffer, allocation.memory, allocation.offset) }
        {
            unsafe {
                device.destroy_buffer(buffer, None);
                memory.allocator.free(allocation);
            }
            return Err(e.into());
        }

        device.debug_utils.set_name(buffer, name);

        Ok(Self {
            raw: buffer,
            allocation: Some(allocation),
            retirer: memory.retirer(),
            size,
        })
    }

    #[must_use]
    pub const fn raw(&self) -> vk::Buffer {
        self.raw
    }

    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Copies `data` into the mapped allocation at `offset`.
    ///
    /// # Panics
    ///
    /// Panics when the buffer is not host-visible or the write would run past
    /// the end of the buffer; both are caller bugs.
    #[allow(clippy::needless_pass_by_ref_mut)]
    pub fn write(&mut self, offset: u64, data: &[u8]) {
        let Some(mapped) = self
            .allocation
            .as_ref()
            .and_then(|allocation| allocation.mapped)
        else {
            panic!("write requires host-visible memory; this buffer has no mapping");
        };

        let end = offset
            .checked_add(data.len() as u64)
            .expect("write range overflows");
        assert!(
            end <= self.size,
            "write of {} bytes at offset {offset} exceeds buffer size {}",
            data.len(),
            self.size
        );

        unsafe {
            ptr::copy_nonoverlapping(
                data.as_ptr(),
                mapped.as_ptr().add(offset as usize),
                data.len(),
            );
        }
    }
}

impl Drop for GpuBuffer {
    fn drop(&mut self) {
        self.retirer.retire(Retired::Buffer(self.raw));
        if let Some(allocation) = self.allocation.take() {
            self.retirer.retire(Retired::Memory(allocation));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr::NonNull;
    use std::sync::mpsc::channel;

    /// A buffer over plain heap memory: no device, no GPU.
    fn host_buffer(storage: &mut [u8]) -> GpuBuffer {
        let (sender, receiver) = channel();
        // Keep the receiver alive for the test so drops have somewhere to go.
        std::mem::forget(receiver);
        GpuBuffer {
            raw: vk::Buffer::null(),
            allocation: Some(Allocation {
                memory: vk::DeviceMemory::null(),
                offset: 0,
                size: storage.len() as u64,
                mapped: NonNull::new(storage.as_mut_ptr()),
            }),
            retirer: Retirer::for_tests(sender),
            size: storage.len() as u64,
        }
    }

    #[test]
    fn write_lands_at_the_requested_offset() {
        let mut storage = [0u8; 8];
        let mut buffer = host_buffer(&mut storage);
        buffer.write(2, &[7, 8, 9]);
        drop(buffer);
        assert_eq!(storage, [0, 0, 7, 8, 9, 0, 0, 0]);
    }

    #[test]
    #[should_panic(expected = "exceeds buffer size")]
    fn writes_past_the_end_are_a_caller_bug() {
        let mut storage = [0u8; 4];
        let mut buffer = host_buffer(&mut storage);
        buffer.write(2, &[1, 2, 3]);
    }
}
