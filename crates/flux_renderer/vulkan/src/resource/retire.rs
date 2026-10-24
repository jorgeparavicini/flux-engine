//! Deferred destruction: resources dropped mid-frame are destroyed only once
//! the GPU provably finished every frame that could reference them.

use crate::memory::allocator::{Allocation, GpuAllocator};
use ash::vk;
use std::sync::mpsc::{Receiver, Sender, channel};

/// Slot-tagged holding bins for retired items. Vulkan-free; the caller
/// supplies the frame slot whose fence wait just completed.
pub struct RetireBins<T> {
    pending: Vec<T>,
    bins: Vec<Vec<T>>,
}

impl<T> RetireBins<T> {
    #[must_use]
    pub fn new(slots: usize) -> Self {
        Self {
            pending: Vec::new(),
            bins: (0..slots).map(|_| Vec::new()).collect(),
        }
    }

    /// Parks an item until the GPU provably cannot reference it anymore.
    pub fn retire(&mut self, item: T) {
        self.pending.push(item);
    }

    /// Called once per frame, right after `slot`'s fence wait: returns the
    /// items that wait has proven unreachable and bins the pending ones.
    ///
    /// # Panics
    ///
    /// Panics when `slot` is not below the slot count given to [`Self::new`].
    pub fn advance(&mut self, slot: usize) -> Vec<T> {
        std::mem::replace(&mut self.bins[slot], std::mem::take(&mut self.pending))
    }

    /// Empties pending items and every bin, for shutdown after a device-wide
    /// idle.
    pub fn drain(&mut self) -> Vec<T> {
        let mut drained = std::mem::take(&mut self.pending);
        for bin in &mut self.bins {
            drained.append(bin);
        }
        drained
    }
}

/// A handle whose destruction is deferred until the GPU provably finished
/// with it.
pub enum Retired {
    Buffer(vk::Buffer),
    Image(vk::Image),
    ImageView(vk::ImageView),
    Memory(Allocation),
}

/// A cloneable hand-in for retired handles; resource wrappers send their
/// parts here from `Drop`.
#[derive(Clone)]
pub struct Retirer(Sender<Retired>);

impl Retirer {
    #[cfg(test)]
    pub(crate) fn for_tests(sender: Sender<Retired>) -> Self {
        Self(sender)
    }

    /// Hands an item over for deferred destruction. A send after the queue
    /// is gone is deliberately ignored: by then teardown has idled the
    /// device and immediate destruction was already safe.
    pub fn retire(&self, item: Retired) {
        let _ = self.0.send(item);
    }
}

/// Collects retired handles and destroys them once their frame slot's fence
/// wait proves the GPU is done.
pub struct RetireQueue {
    device: ash::Device,
    sender: Sender<Retired>,
    receiver: Receiver<Retired>,
    bins: RetireBins<Retired>,
}

impl RetireQueue {
    /// Implementation checklist:
    /// 1. one `channel()`; keep both ends (senders are cloned on demand)
    /// 2. bins sized to `slots`
    #[must_use]
    pub fn new(device: ash::Device, slots: usize) -> Self {
        let (sender, receiver) = channel();
        Self {
            device,
            sender,
            receiver,
            bins: RetireBins::new(slots),
        }
    }

    /// A new hand-in for resource wrappers.
    #[must_use]
    pub fn retirer(&self) -> Retirer {
        Retirer(self.sender.clone())
    }

    /// Called once per frame after `slot`'s fence wait: bins everything
    /// retired since the last frame and destroys what the wait released.
    ///
    /// Implementation checklist:
    /// 1. `try_recv` until empty, `bins.retire` each
    /// 2. `bins.advance(slot)`; destroy every released item (match on
    ///    [`Retired`]; `Memory` goes back through `allocator.free`)
    pub fn advance(&mut self, slot: usize, allocator: &mut dyn GpuAllocator) {
        self.retire_received();

        let resources = self.bins.advance(slot);
        self.destroy_retired(allocator, resources);
    }

    /// Destroys everything pending, for shutdown after a device-wide idle.
    pub fn drain(&mut self, allocator: &mut dyn GpuAllocator) {
        self.retire_received();

        let resources = self.bins.drain();
        self.destroy_retired(allocator, resources);
    }

    fn retire_received(&mut self) {
        while let Ok(item) = self.receiver.try_recv() {
            self.bins.retire(item);
        }
    }

    fn destroy_retired(&self, allocator: &mut dyn GpuAllocator, resources: Vec<Retired>) {
        resources.into_iter().for_each(|released| unsafe {
            match released {
                Retired::Buffer(buffer) => self.device.destroy_buffer(buffer, None),
                Retired::Image(image) => self.device.destroy_image(image, None),
                Retired::ImageView(image_view) => {
                    self.device.destroy_image_view(image_view, None);
                }
                Retired::Memory(allocation) => allocator.free(allocation),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLOTS: usize = 2;

    #[test]
    fn nothing_is_released_before_a_full_rotation() {
        let mut bins: RetireBins<&str> = RetireBins::new(SLOTS);
        assert_eq!(bins.advance(0), Vec::<&str>::new());
        bins.retire("a");
        assert_eq!(
            bins.advance(1),
            Vec::<&str>::new(),
            "a was binned to slot 1, not released"
        );
        assert_eq!(bins.advance(0), Vec::<&str>::new(), "slot 0 holds nothing");
        assert_eq!(
            bins.advance(1),
            vec!["a"],
            "slot 1's fence wait has now proven a unreachable"
        );
    }

    #[test]
    fn items_release_exactly_once_in_their_own_slot() {
        let mut bins: RetireBins<u32> = RetireBins::new(SLOTS);
        bins.advance(0);
        bins.retire(10); // during slot 0's frame -> binned at next advance
        assert_eq!(bins.advance(1), Vec::<u32>::new());
        bins.retire(20); // during slot 1's frame
        assert_eq!(bins.advance(0), Vec::<u32>::new());
        assert_eq!(bins.advance(1), vec![10], "slot 1's bin releases 10");
        assert_eq!(bins.advance(0), vec![20], "slot 0's bin releases 20");
        assert_eq!(bins.advance(1), Vec::<u32>::new(), "nothing releases twice");
    }

    #[test]
    fn drain_empties_pending_and_every_bin() {
        let mut bins: RetireBins<u32> = RetireBins::new(SLOTS);
        bins.advance(0);
        bins.retire(1);
        bins.advance(1);
        bins.retire(2); // pending, never binned
        let mut drained = bins.drain();
        drained.sort_unstable();
        assert_eq!(drained, vec![1, 2]);
        assert_eq!(bins.advance(0), Vec::<u32>::new());
        assert_eq!(bins.advance(1), Vec::<u32>::new());
    }
}
