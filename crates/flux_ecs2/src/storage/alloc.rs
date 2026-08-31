use std::alloc::Layout;
use std::ptr::NonNull;

pub const CHUNK_SIZE: usize = 16 * 1024;
pub const CHUNK_ALIGN: usize = 64;


#[derive(Default)]
pub struct ChunkAlloc {
    free: Vec<NonNull<u8>>,
    live: usize,
    peak: usize,
}

impl ChunkAlloc {
    const LAYOUT: Layout = match Layout::from_size_align(CHUNK_SIZE, CHUNK_ALIGN) {
        Ok(layout) => layout,
        Err(_) => panic!("failed to create layout for chunk allocation"),
    };

    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc(&mut self) -> NonNull<u8> {
        self.live += 1;
        self.peak = self.live.max(self.peak);
        self.free.pop().unwrap_or_else(|| {
            let p = unsafe { std::alloc::alloc(Self::LAYOUT) };
            NonNull::new(p).unwrap_or_else(|| std::alloc::handle_alloc_error(Self::LAYOUT))
        })
    }

    pub unsafe fn dealloc(&mut self, chunk: NonNull<u8>) {
        debug_assert!(!self.free.contains(&chunk), "double free");
        debug_assert!(self.live > 0, "freeing empty chunk");

        self.free.push(chunk);
        self.live -= 1;
    }

    pub fn live(&self) -> usize {
        self.live
    }

    pub fn peak(&self) -> usize {
        self.peak
    }
}

impl Drop for ChunkAlloc {
    fn drop(&mut self) {
        debug_assert!(std::thread::panicking() || self.live == 0, "ChunkAlloc dropped with {} live chunks", self.live);

        for chunk in self.free.drain(..) {
            unsafe {
                std::alloc::dealloc(chunk.as_ptr(), Self::LAYOUT);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // ------------------------------------------------------------ block shape

    #[test]
    fn chunk_shape_constants_are_pinned() {
        // Literals on purpose: layout code all over the crate assumes exactly
        // this shape, so a change must be a conscious, test-breaking act.
        assert_eq!(CHUNK_SIZE, 16 * 1024);
        assert_eq!(CHUNK_ALIGN, 64);
    }

    #[test]
    fn blocks_are_aligned_and_writable_end_to_end() {
        let mut a = ChunkAlloc::new();
        let p = a.alloc();
        assert_eq!(p.as_ptr() as usize % 64, 0, "64-byte aligned (literal: not derived from the constant under test)");
        // Write and read back every byte; under miri this proves the block
        // really is CHUNK_SIZE bytes with valid provenance.
        unsafe {
            for i in 0..CHUNK_SIZE {
                p.as_ptr().add(i).write((i % 251) as u8);
            }
            for i in 0..CHUNK_SIZE {
                assert_eq!(p.as_ptr().add(i).read(), (i % 251) as u8);
            }
            a.dealloc(p);
        }
    }

    #[test]
    fn distinct_blocks_do_not_overlap() {
        let mut a = ChunkAlloc::new();
        let blocks: Vec<_> = (0..8u8).map(|_| a.alloc()).collect();
        unsafe {
            for (tag, p) in blocks.iter().enumerate() {
                std::ptr::write_bytes(p.as_ptr(), tag as u8, CHUNK_SIZE);
            }
            for (tag, p) in blocks.iter().enumerate() {
                assert_eq!(p.as_ptr().read(), tag as u8);
                assert_eq!(p.as_ptr().add(CHUNK_SIZE - 1).read(), tag as u8);
            }
            for p in blocks {
                a.dealloc(p);
            }
        }
    }

    // -------------------------------------------------------------- counters

    #[test]
    fn live_and_peak_track_allocations() {
        let mut a = ChunkAlloc::new();
        assert_eq!((a.live(), a.peak()), (0, 0));
        let p1 = a.alloc();
        let p2 = a.alloc();
        let p3 = a.alloc();
        assert_eq!((a.live(), a.peak()), (3, 3));
        unsafe {
            a.dealloc(p2);
            a.dealloc(p3);
        }
        assert_eq!((a.live(), a.peak()), (1, 3), "peak is a high-water mark");
        let p4 = a.alloc();
        let p5 = a.alloc();
        assert_eq!((a.live(), a.peak()), (3, 3), "recycling does not move the peak");
        let p6 = a.alloc();
        assert_eq!((a.live(), a.peak()), (4, 4), "new high-water mark");
        unsafe {
            a.dealloc(p1);
            a.dealloc(p4);
            a.dealloc(p5);
            a.dealloc(p6);
        }
        assert_eq!((a.live(), a.peak()), (0, 4));
    }

    // -------------------------------------------------------------- recycling

    #[test]
    fn freed_block_is_recycled_before_new_allocation() {
        let mut a = ChunkAlloc::new();
        let p = a.alloc();
        let addr = p.as_ptr() as usize;
        unsafe { a.dealloc(p) };
        let q = a.alloc();
        assert_eq!(q.as_ptr() as usize, addr, "free list is used before the system allocator");
        unsafe { a.dealloc(q) };
    }

    #[test]
    fn churn_on_one_slot_reuses_one_block() {
        let mut a = ChunkAlloc::new();
        let mut seen = HashSet::new();
        for _ in 0..1_000 {
            let p = a.alloc();
            seen.insert(p.as_ptr() as usize);
            unsafe { a.dealloc(p) };
        }
        assert_eq!(seen.len(), 1, "alloc/dealloc churn must not touch the system allocator");
        assert_eq!((a.live(), a.peak()), (0, 1));
    }

    #[test]
    fn interleaved_churn_keeps_counters_consistent() {
        // Deterministic xorshift stream; grows and shrinks a working set.
        let mut rng: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        let mut a = ChunkAlloc::new();
        let mut held: Vec<_> = Vec::new();
        let mut addresses = HashSet::new();
        for _ in 0..2_000 {
            if next() % 3 == 0 && !held.is_empty() {
                let i = (next() as usize) % held.len();
                unsafe { a.dealloc(held.swap_remove(i)) };
            } else {
                let p = a.alloc();
                assert!(
                    addresses.insert(p.as_ptr() as usize) || !held.iter().any(|h| h.as_ptr() == p.as_ptr()),
                    "allocator handed out a block that is still held"
                );
                held.push(p);
            }
            assert_eq!(a.live(), held.len(), "live == blocks currently held");
            assert!(a.peak() >= a.live());
        }
        for p in held.drain(..) {
            unsafe { a.dealloc(p) };
        }
        assert_eq!(a.live(), 0);
    }

    // ------------------------------------------------------------------ drop

    #[test]
    fn drop_with_everything_returned_frees_all_memory() {
        // The assertion here is miri's leak checker: if the free list is not
        // released in Drop, `cargo miri test` fails this test with a leak.
        let mut a = ChunkAlloc::new();
        let blocks: Vec<_> = (0..16).map(|_| a.alloc()).collect();
        for p in blocks {
            unsafe { a.dealloc(p) };
        }
        assert_eq!(a.live(), 0);
        drop(a);
    }

    #[test]
    fn default_is_empty() {
        let a = ChunkAlloc::default();
        assert_eq!((a.live(), a.peak()), (0, 0));
    }

    // --------------------------------------------------- debug-mode guardrails

    #[test]
    #[should_panic(expected = "live")]
    #[cfg_attr(
        miri,
        ignore = "deliberately leaks the live block, which miri's leak checker rejects"
    )]
    #[cfg(debug_assertions)]
    fn dropping_with_live_blocks_panics_in_debug() {
        let mut a = ChunkAlloc::new();
        let p = a.alloc();
        let _ = p;
        drop(a); // one block still live: the debug assertion must fire
    }

    #[test]
    #[should_panic(expected = "double")]
    #[cfg_attr(miri, ignore = "leaks on unwind by design; covered in debug builds without miri")]
    #[cfg(debug_assertions)]
    fn double_dealloc_panics_in_debug() {
        let mut a = ChunkAlloc::new();
        let p = a.alloc();
        unsafe {
            a.dealloc(p);
            a.dealloc(p); // already on the free list: the debug assertion must fire
        }
    }
}
