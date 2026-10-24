//! Getting bytes to the GPU: a per-frame staging ring the CPU writes into,
//! and an uploader that records the transfer-queue copies out of it.

use crate::error::RendererError;
use crate::memory::gpu_memory::GpuMemory;
use crate::present::presenter::FRAMES_IN_FLIGHT;
use crate::resource::gpu_buffer::GpuBuffer;
use crate::{MemoryUsage, device::logical::RenderDevice};
use ash::vk;
use flux_ecs::{Commands, Single};
use log::debug;

/// Bytes each frame slot's staging region holds.
pub const STAGING_REGION_SIZE: u64 = 4 * 1024 * 1024;

/// A staged span inside the ring, ready to be copied out of.
#[derive(Debug, Clone, Copy)]
pub struct StagingSlice {
    pub buffer: vk::Buffer,
    pub offset: u64,
    pub size: u64,
}

/// Per-slot bump cursors over one buffer split into equal regions; the
/// Vulkan-free core of the staging ring. Offsets are aligned, regions never
/// overlap, and a slot's region resets when its frame slot comes around.
pub struct BumpRegions {
    region_size: u64,
    cursors: Vec<u64>,
}

impl BumpRegions {
    #[must_use]
    pub fn new(region_size: u64, slots: usize) -> Self {
        Self {
            region_size,
            cursors: vec![0; slots],
        }
    }

    /// Claims `size` bytes in `slot`'s region at `alignment`, returning the
    /// offset into the whole buffer, or `None` when the region is full.
    ///
    /// # Panics
    ///
    /// Panics when `slot` is out of range; slots come from the frame loop,
    /// never from data, so a bad one is a caller bug.
    pub fn stage(&mut self, slot: usize, size: u64, alignment: u64) -> Option<u64> {
        let aligned = self.cursors[slot].next_multiple_of(alignment);
        if aligned + size > self.region_size {
            return None;
        }

        self.cursors[slot] = aligned + size;
        Some(aligned + slot as u64 * self.region_size)
    }

    /// Resets `slot`'s region; its previous contents are consumed, proven by
    /// the same fence wait that recycles the frame slot.
    pub fn advance(&mut self, slot: usize) {
        self.cursors[slot] = 0;
    }
}

/// One host-visible buffer serving CPU-to-GPU traffic for all in-flight
/// frames, one region per frame slot.
#[derive(flux_ecs::Component)]
pub struct StagingRing {
    buffer: GpuBuffer,
    regions: BumpRegions,
}

impl StagingRing {
    /// Creates the ring: one host-visible transfer-source buffer holding a
    /// `region_size` region for every in-flight frame.
    ///
    /// # Errors
    ///
    /// Fails when the backing buffer cannot be created or allocated.
    pub fn new(
        device: &RenderDevice,
        memory: &mut GpuMemory,
        region_size: u64,
    ) -> Result<Self, RendererError> {
        let buffer = GpuBuffer::new(
            device,
            memory,
            region_size * FRAMES_IN_FLIGHT as u64,
            vk::BufferUsageFlags::TRANSFER_SRC,
            MemoryUsage::CpuToGpu,
            "staging ring",
        )?;

        let regions = BumpRegions::new(region_size, FRAMES_IN_FLIGHT);

        Ok(Self { buffer, regions })
    }

    /// Copies `data` into `slot`'s region and returns where it landed, or
    /// `None` when the region is full this frame. Offsets are 4-byte
    /// aligned, which covers every transfer source Vulkan requires.
    pub fn stage(&mut self, slot: usize, data: &[u8]) -> Option<StagingSlice> {
        let offset = self.regions.stage(slot, data.len() as u64, 4)?;

        self.buffer.write(offset, data);

        Some(StagingSlice {
            buffer: self.buffer.raw(),
            offset,
            size: data.len() as u64,
        })
    }

    /// Frame tick, alongside the retire queue's advance.
    pub fn advance(&mut self, slot: usize) {
        self.regions.advance(slot);
    }
}

/// Records and submits transfer-queue copies.
///
/// Submission blocks until the copies finish: every current upload happens
/// at startup, where waiting is correct and simple. Streaming uploads will
/// add a timeline-semaphore handoff to the graphics queue (gated on
/// `timeline_semaphore`) and queue-family ownership transfer when they
/// arrive.
#[derive(flux_ecs::Component)]
pub struct Uploader {
    device: ash::Device,
    pool: vk::CommandPool,
    command_buffer: vk::CommandBuffer,
    fence: vk::Fence,
    queue: vk::Queue,
}

impl Uploader {
    /// Creates the uploader: a transient command pool on the transfer queue
    /// family, one reusable command buffer, and the fence submissions block
    /// on.
    ///
    /// # Errors
    ///
    /// Fails when the pool, command buffer, or fence cannot be created.
    pub fn new(device: &RenderDevice) -> Result<Self, RendererError> {
        let info = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::TRANSIENT)
            .queue_family_index(device.transfer.family);
        let pool = unsafe { device.create_command_pool(&info, None)? };
        device.debug_utils.set_name(pool, "uploader pool");

        // Nothing here has reached the GPU yet, so failure unwinds destroy
        // immediately; the pool takes its command buffer with it.
        let info = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        let command_buffer = match unsafe { device.allocate_command_buffers(&info) } {
            Ok(buffers) => buffers[0],
            Err(e) => {
                unsafe { device.destroy_command_pool(pool, None) };
                return Err(e.into());
            }
        };
        device
            .debug_utils
            .set_name(command_buffer, "uploader commands");

        let fence = match unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) } {
            Ok(fence) => fence,
            Err(e) => {
                unsafe { device.destroy_command_pool(pool, None) };
                return Err(e.into());
            }
        };
        device.debug_utils.set_name(fence, "uploader fence");

        Ok(Self {
            device: device.raw_clone(),
            pool,
            command_buffer,
            fence,
            queue: device.transfer.raw,
        })
    }

    /// Starts recording a batch of copies, discarding the previous batch's
    /// recording through a pool reset.
    ///
    /// # Errors
    ///
    /// Fails when the pool reset or the command buffer begin fails.
    pub fn begin(&mut self) -> Result<(), RendererError> {
        unsafe {
            self.device
                .reset_command_pool(self.pool, vk::CommandPoolResetFlags::empty())?;

            // One-time-submit tells the driver the recording is never
            // resubmitted, matching the per-batch pool reset.
            let info = vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
            self.device
                .begin_command_buffer(self.command_buffer, &info)?;
        }

        Ok(())
    }

    /// Records one buffer-to-buffer copy.
    pub fn copy_buffer(&mut self, src: StagingSlice, dst: vk::Buffer, dst_offset: u64) {
        unsafe {
            let region = vk::BufferCopy::default()
                .src_offset(src.offset)
                .dst_offset(dst_offset)
                .size(src.size);
            self.device
                .cmd_copy_buffer(self.command_buffer, src.buffer, dst, &[region]);
        }
    }

    /// Ends recording, submits on the transfer queue, and blocks until the
    /// GPU finishes.
    ///
    /// # Errors
    ///
    /// Fails when recording cannot end, the submit is rejected, or the fence
    /// wait fails; all signal a lost device.
    pub fn submit_and_wait(&mut self) -> Result<(), RendererError> {
        unsafe {
            self.device.end_command_buffer(self.command_buffer)?;

            let command_buffers = &[self.command_buffer];
            let submit_info = vk::SubmitInfo::default().command_buffers(command_buffers);
            self.device
                .queue_submit(self.queue, &[submit_info], self.fence)?;
            self.device.wait_for_fences(&[self.fence], true, u64::MAX)?;
            self.device.reset_fences(&[self.fence])?;
        }

        Ok(())
    }
}

impl Drop for Uploader {
    fn drop(&mut self) {
        unsafe {
            self.device.destroy_fence(self.fence, None);
            // Command buffers die with their pool.
            self.device.destroy_command_pool(self.pool, None);
        }
    }
}

/// Creates the [`StagingRing`] and [`Uploader`] singletons.
pub fn create_upload_path(
    device: Single<&RenderDevice>,
    mut memory: Single<&mut GpuMemory>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    debug!("Creating upload path");
    commands.insert_singleton(StagingRing::new(&device, &mut memory, STAGING_REGION_SIZE)?);
    commands.insert_singleton(Uploader::new(&device)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGION: u64 = 64;
    const SLOTS: usize = 2;

    #[test]
    fn offsets_are_aligned_and_land_in_the_right_region() {
        let mut regions = BumpRegions::new(REGION, SLOTS);
        assert_eq!(regions.stage(0, 10, 4), Some(0));
        assert_eq!(
            regions.stage(0, 4, 4),
            Some(12),
            "cursor 10 aligns up to 12"
        );
        assert_eq!(
            regions.stage(1, 8, 4),
            Some(REGION),
            "slot 1's region starts after slot 0's"
        );
    }

    #[test]
    fn a_full_region_stages_nothing_more() {
        let mut regions = BumpRegions::new(REGION, SLOTS);
        assert_eq!(regions.stage(0, 60, 4), Some(0));
        assert_eq!(regions.stage(0, 8, 4), None, "only 4 bytes remain");
        assert_eq!(
            regions.stage(0, 4, 4),
            Some(60),
            "a smaller claim still fits"
        );
        assert_eq!(regions.stage(1, 8, 4), Some(REGION), "slot 1 unaffected");
    }

    #[test]
    fn advance_resets_only_that_slot() {
        let mut regions = BumpRegions::new(REGION, SLOTS);
        regions.stage(0, 32, 4);
        regions.stage(1, 32, 4);
        regions.advance(0);
        assert_eq!(regions.stage(0, 32, 4), Some(0), "slot 0 reset");
        assert_eq!(
            regions.stage(1, 16, 4),
            Some(REGION + 32),
            "slot 1 kept its cursor"
        );
    }
}
