use crate::region::{get_current_region, Region};
use std::alloc::{GlobalAlloc, Layout, System};
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};

#[global_allocator]
pub static ALLOCATOR: TrackedAllocator = TrackedAllocator::new();

#[derive(Default)]
pub struct TrackedAllocator {
    allocations: [AtomicUsize; mem::variant_count::<Region>()],
    allocated_bytes: [AtomicUsize; mem::variant_count::<Region>()],
}

impl TrackedAllocator {
    const fn new() -> Self {
        Self {
            allocations: [
                AtomicUsize::new(0),
                AtomicUsize::new(0),
                AtomicUsize::new(0),
                AtomicUsize::new(0),
                AtomicUsize::new(0),
                AtomicUsize::new(0),
            ],
            allocated_bytes: [
                AtomicUsize::new(0),
                AtomicUsize::new(0),
                AtomicUsize::new(0),
                AtomicUsize::new(0),
                AtomicUsize::new(0),
                AtomicUsize::new(0),
            ],
        }
    }

    fn region_to_index(region: Region) -> usize {
        match region {
            Region::Graphics => 0,
            Region::Physics => 1,
            Region::Audio => 2,
            Region::Scene => 3,
            Region::General => 4,
            Region::ECS => 5,
        }
    }

    pub fn get_count(&self, region: Region) -> usize {
        let index = Self::region_to_index(region);
        self.allocations[index].load(Ordering::SeqCst)
    }

    pub fn get_bytes(&self, region: Region) -> usize {
        let index = Self::region_to_index(region);
        self.allocated_bytes[index].load(Ordering::SeqCst)
    }
}

unsafe impl GlobalAlloc for TrackedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let index = Self::region_to_index(get_current_region());
        self.allocations[index].fetch_add(1, Ordering::SeqCst);
        self.allocated_bytes[index].fetch_add(layout.size(), Ordering::SeqCst);

        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let index = Self::region_to_index(get_current_region());
        self.allocations[index].fetch_sub(1, Ordering::SeqCst);
        self.allocated_bytes[index].fetch_sub(layout.size(), Ordering::SeqCst);

        System.dealloc(ptr, layout);
    }
}

#[allow(clippy::vec_init_then_push)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test() {
        let allocation_count = ALLOCATOR.get_count(Region::Graphics);

        let _region_guard = crate::RegionGuard::new(Region::Graphics);
        let mut vec = Vec::<i32>::new();
        vec.push(1);

        assert_eq!(ALLOCATOR.get_count(Region::Graphics), allocation_count + 1);
    }
}
