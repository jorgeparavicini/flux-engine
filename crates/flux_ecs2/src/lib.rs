//! An archetypal entity component system.
//!
//! Entities are lightweight ids; data lives in [`Component`]s attached to
//! them. Entities with the same set of components are stored together, in
//! fixed-size chunks laid out for linear iteration.
//!
//! ```
//! use flux_ecs2::{Component, World};
//!
//! #[derive(Component, Debug, PartialEq)]
//! struct Position(f32, f32);
//!
//! #[derive(Component)]
//! struct Velocity(f32, f32);
//!
//! let mut world = World::new();
//! let entity = world.spawn((Position(0.0, 0.0), Velocity(1.0, 2.0)));
//!
//! world.get_mut::<Position>(entity).unwrap().0 += 5.0;
//! assert_eq!(world.get::<Position>(entity), Some(&Position(5.0, 0.0)));
//!
//! world.despawn(entity);
//! assert!(!world.is_alive(entity));
//! ```
#![feature(const_trait_impl, const_cmp)]

mod access;
mod bundle;
mod entity;
mod grant;
mod query;
mod registry;
mod schedule;
mod storage;
mod system;
mod world;
mod component;
#[cfg(any(test, feature = "reference"))]
pub mod reference;

pub use component::{Component, ComponentKey, StorageClass, ThreadSafeComponent};
pub use bundle::Bundle;
pub use entity::{Entities, Entity};
pub use registry::ComponentId;
pub use schedule::{Schedule, SystemLabel};
pub use system::{IntoSystem, Local, Single, System};
pub use world::World;
pub use query::{Added, Changed, Query, QueryFilter, QueryState, With, Without};
pub use flux_ecs2_macros::Component;
