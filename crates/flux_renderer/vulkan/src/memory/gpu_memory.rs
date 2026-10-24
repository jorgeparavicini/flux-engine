//! The renderer's memory hub: the active allocation strategy and the retire
//! queue that returns memory to it on the GPU's schedule.

use crate::device::logical::RenderDevice;
use crate::memory::dedicated::DedicatedAllocator;
use crate::present::presenter::FRAMES_IN_FLIGHT;
use crate::resource::retire::{RetireQueue, Retirer};
use flux_ecs::{Commands, Single};
use log::info;

/// Owns the allocator and the retire queue as one singleton, so allocation
/// and deferred destruction cannot drift apart.
#[derive(flux_ecs::Component)]
#[component(non_send)]
pub struct GpuMemory {
    pub allocator: DedicatedAllocator,
    pub retire: RetireQueue,
}

impl GpuMemory {
    /// Implementation checklist: a [`DedicatedAllocator`] and a
    /// [`RetireQueue`] sized to [`FRAMES_IN_FLIGHT`], both from the device.
    #[must_use]
    pub fn new(device: &RenderDevice) -> Self {
        Self {
            allocator: DedicatedAllocator::new(device),
            retire: RetireQueue::new(device.raw_clone(), FRAMES_IN_FLIGHT),
        }
    }

    /// A hand-in for resource wrappers' deferred destruction.
    #[must_use]
    pub fn retirer(&self) -> Retirer {
        self.retire.retirer()
    }

    /// Frame tick: destroys whatever `slot`'s fence wait just released.
    pub fn advance(&mut self, slot: usize) {
        self.retire.advance(slot, &mut self.allocator);
    }

    /// Shutdown: destroys everything pending. Call after a device-wide idle.
    pub fn drain(&mut self) {
        self.retire.drain(&mut self.allocator);
    }
}

/// Creates the [`GpuMemory`] singleton.
pub fn create_gpu_memory(device: Single<&RenderDevice>, mut commands: Commands) {
    info!("Creating GPU memory hub");
    commands.insert_singleton(GpuMemory::new(&device));
}

/// Drains the retire queue and removes the singleton; run after the device
/// idles and before the device is destroyed.
pub fn destroy_gpu_memory(mut memory: Single<&mut GpuMemory>, mut commands: Commands) {
    memory.drain();
    commands.remove_singleton::<GpuMemory>();
}
