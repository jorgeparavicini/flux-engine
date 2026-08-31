#![feature(const_trait_impl, const_cmp)]
mod entity;
mod storage;
mod component;
#[cfg(any(test, feature = "reference"))]
pub mod reference;

pub use component::{Component, ComponentKey, StorageClass, ThreadSafeComponent};
pub use entity::{Entities, Entity};
pub use flux_ecs2_macros::Component;
