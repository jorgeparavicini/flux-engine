#![feature(const_trait_impl, const_cmp)]
mod bundle;
mod entity;
mod registry;
mod storage;
mod world;
mod component;
#[cfg(any(test, feature = "reference"))]
pub mod reference;

pub use component::{Component, ComponentKey, StorageClass, ThreadSafeComponent};
pub use bundle::Bundle;
pub use entity::{Entities, Entity};
pub use registry::ComponentId;
pub use world::World;
pub use flux_ecs2_macros::Component;
