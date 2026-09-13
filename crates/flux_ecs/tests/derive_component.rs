//! Observable behaviour of `#[derive(Component)]`.
//!
//! Lives in `tests/` because the derive expands to `::flux_ecs::...` paths,
//! which only resolve from outside the crate. This file's crate name is
//! `derive_component`, so `module_path!()` at the root is exactly that.
//!
//! `ComponentKey` implements `const PartialEq`; comparing keys in const context
//! requires the same feature gates as the crate itself.
#![feature(const_trait_impl, const_cmp)]

use flux_ecs::{Component, ComponentKey, StorageClass};
use std::collections::HashSet;
use std::rc::Rc;

#[derive(Component)]
struct Position {
    #[allow(dead_code)]
    x: f32,
    #[allow(dead_code)]
    y: f32,
}

#[derive(Component)]
struct Velocity(#[allow(dead_code)] f32, #[allow(dead_code)] f32);

#[derive(Component)]
struct Marker;

#[derive(Component)]
#[allow(dead_code)]
enum Phase {
    Idle,
    Running,
}

/// `Rc` is neither Send nor Sync — this only compiles because of the attribute.
#[derive(Component)]
#[component(non_send)]
struct VulkanHandle(#[allow(dead_code)] Rc<u8>);

/// Perfectly thread-safe type that still asks to be main-thread-pinned.
#[derive(Component)]
#[component(non_send)]
struct PinnedAnyway(#[allow(dead_code)] u32);

mod inner {
    use flux_ecs::Component;

    /// Same type NAME as the root `Position` — must get a different key.
    #[derive(Component)]
    pub struct Position;

    pub mod deeper {
        use flux_ecs::Component;

        #[derive(Component)]
        pub struct Position;
    }
}

mod elsewhere {
    use flux_ecs::{Component, ComponentKey};

    /// Reads a KEY from a different module than the one that defined it.
    pub fn position_key_seen_from_here() -> ComponentKey {
        super::Position::KEY
    }
}

// ------------------------------------------------------------ key derivation

#[test]
fn key_is_fnv_of_module_path_colon_colon_type_name() {
    // Pins the exact path format: `concat!(module_path!(), "::", stringify!(Name))`.
    assert_eq!(
        Position::KEY,
        ComponentKey::from_path("derive_component::Position")
    );
    assert_eq!(
        Velocity::KEY,
        ComponentKey::from_path("derive_component::Velocity")
    );
    assert_eq!(
        Marker::KEY,
        ComponentKey::from_path("derive_component::Marker")
    );
    assert_eq!(
        Phase::KEY,
        ComponentKey::from_path("derive_component::Phase")
    );
}

#[test]
fn module_path_is_part_of_the_key() {
    assert_eq!(
        inner::Position::KEY,
        ComponentKey::from_path("derive_component::inner::Position")
    );
    assert_eq!(
        inner::deeper::Position::KEY,
        ComponentKey::from_path("derive_component::inner::deeper::Position")
    );
}

#[test]
fn same_type_name_in_different_modules_gives_different_keys() {
    // This is why the module path is hashed in: bare type names collide constantly.
    assert_ne!(Position::KEY, inner::Position::KEY);
    assert_ne!(Position::KEY, inner::deeper::Position::KEY);
    assert_ne!(inner::Position::KEY, inner::deeper::Position::KEY);
}

#[test]
fn every_derived_type_has_a_distinct_key() {
    let keys: HashSet<ComponentKey> = [
        Position::KEY,
        Velocity::KEY,
        Marker::KEY,
        Phase::KEY,
        VulkanHandle::KEY,
        PinnedAnyway::KEY,
        inner::Position::KEY,
        inner::deeper::Position::KEY,
    ]
    .into_iter()
    .collect();
    assert_eq!(keys.len(), 8);
}

#[test]
fn key_does_not_depend_on_where_it_is_read() {
    assert_eq!(elsewhere::position_key_seen_from_here(), Position::KEY);
}

#[test]
fn key_is_a_true_const() {
    // Keys must be usable in const items and const assertions.
    const K: ComponentKey = Position::KEY;
    const _: () = assert!(Position::KEY != Velocity::KEY);
    const _: () = assert!(Position::KEY == K);
    assert_eq!(K, Position::KEY);
}

// ------------------------------------------------------------------ defaults

#[test]
#[allow(
    clippy::assertions_on_constants,
    reason = "deliberately checks compile-time constants"
)]
fn derived_components_get_the_documented_defaults() {
    assert_eq!(Position::STORAGE, StorageClass::Chunked);
    assert!(!Position::NON_SEND);
    assert_eq!(Marker::STORAGE, StorageClass::Chunked);
    assert!(!Marker::NON_SEND);
    assert!(!Phase::NON_SEND);
}

// ------------------------------------------------------------------ non_send

#[test]
#[allow(
    clippy::assertions_on_constants,
    reason = "deliberately checks compile-time constants"
)]
fn non_send_attribute_sets_the_flag() {
    assert!(VulkanHandle::NON_SEND);
    assert!(PinnedAnyway::NON_SEND);
}

#[test]
fn non_send_does_not_change_other_defaults() {
    assert_eq!(VulkanHandle::STORAGE, StorageClass::Chunked);
    assert_eq!(
        VulkanHandle::KEY,
        ComponentKey::from_path("derive_component::VulkanHandle")
    );
}

#[test]
fn non_send_type_can_actually_be_constructed_and_used() {
    // Guards against the derive emitting something that makes the type unusable.
    let h = VulkanHandle(Rc::new(7));
    let _clone_of_inner = Rc::clone(&h.0);
    let _ = Position { x: 1.0, y: 2.0 };
    let _ = Velocity(0.0, 0.0);
    let _ = Marker;
    let _ = Phase::Idle;
}

// --------------------------------------------------------- trait bound only

#[test]
fn component_bound_is_only_static() {
    // `Component: 'static` has no `Send + Sync` supertrait bound; a non-send type
    // must satisfy the bare trait bound.
    fn requires_component<T: Component>() {}
    requires_component::<VulkanHandle>();
    requires_component::<Position>();
}
