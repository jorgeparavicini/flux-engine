#![feature(variant_count)]

mod region;
mod tracking_allocator;

pub use region::{get_current_region, Region, RegionGuard};
pub use tracking_allocator::ALLOCATOR;
