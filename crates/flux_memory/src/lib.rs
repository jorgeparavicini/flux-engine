#![feature(variant_count)]

mod region;
mod tracking_allocator;

pub use region::{Region, RegionGuard, get_current_region};
pub use tracking_allocator::ALLOCATOR;
