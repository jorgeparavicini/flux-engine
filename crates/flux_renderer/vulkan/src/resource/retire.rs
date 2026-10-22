//! Deferred destruction: resources dropped mid-frame are destroyed only once
//! the GPU provably finished every frame that could reference them.

// ── implementation goes here; the tests below are the spec ──────────────────
//
// `RetireBins<T>` is the Vulkan-free core: `retire` parks an item as
// pending; `advance(slot)` is called once per frame right after that slot's
// fence wait, returns everything previously binned to the slot (now provably
// unreachable by the GPU), and bins the pending items; `drain` empties
// everything for shutdown, after a device-wide idle.

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
