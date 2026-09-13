use std::hash::{Hash, Hasher};

/// Stable, compile-time identity of a component type.
///
/// A 128-bit FNV-1a hash of the type's canonical path. Unlike `TypeId`, it is
/// a true `const`, and `==` is usable in const context (requires the
/// `const_trait_impl` and `const_cmp` features in the comparing crate).
#[derive(Copy, Clone, Eq)]
pub struct ComponentKey(u128);

const impl PartialEq for ComponentKey {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Hash for ComponentKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl ComponentKey {
    /// Computes the key for a type path.
    ///
    /// `path` must be the type's defining module path followed by `::` and the
    /// bare type name, e.g. `my_crate::physics::Position` — no leading `::`,
    /// no generic arguments. `#[derive(Component)]` produces this form via
    /// `concat!(module_path!(), "::", "TypeName")`; manual `Component`
    /// implementations must use the same form.
    pub const fn from_path(path: &str) -> Self {
        const FNV_OFFSET_BASIS: u128 = 0x6c62272e07bb014262b821756295c58d;
        const FNV_PRIME: u128 = 0x0000000001000000000000000000013B;
        let mut hash = FNV_OFFSET_BASIS;
        let bytes = path.as_bytes();
        let mut index = 0;
        while index < bytes.len() {
            hash ^= bytes[index] as u128;
            hash = hash.wrapping_mul(FNV_PRIME);
            index += 1;
        }
        Self(hash)
    }
}

impl std::fmt::Debug for ComponentKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ComponentKey({:#034x})", self.0)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
/// How a component's values are stored.
pub enum StorageClass {
    /// Dense per-archetype columns; the default, fastest to iterate.
    Chunked,
    /// Reserved for components on a small, rapidly changing set of entities.
    SparseSet,
    /// Zero-sized marker: present in the archetype, occupies no memory.
    Tag,
}

/// A type that can be attached to an entity.
///
/// Implement with `#[derive(Component)]`. An entity has at most one value of
/// each component type.
pub trait Component: 'static {
    /// This type's stable identity; see [`ComponentKey`].
    const KEY: ComponentKey;
    /// How values of this type are stored.
    const STORAGE: StorageClass = StorageClass::Chunked;
    /// Whether this type must stay on the thread that created it.
    /// Set by `#[component(non_send)]`.
    const NON_SEND: bool = false;

    /// Whether presence can be toggled per entity without an archetype move.
    /// Set by `#[component(toggleable)]`.
    const TOGGLEABLE: bool = false;

    /// Called after this component is added to one or more entities, with the
    /// affected entities as a single slice per structural change.
    const ON_ADD: Option<crate::world::Hook> = None;

    /// Called before this component is removed from one or more entities,
    /// while they still hold it, with the affected entities as a single slice.
    const ON_REMOVE: Option<crate::world::Hook> = None;
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not `Send + Sync`, so it cannot be a component",
    note = "add `#[component(non_send)]` to pin it to the main thread"
)]
/// Marker satisfied by every `Send + Sync` type; asserted by
/// `#[derive(Component)]` unless the type opts out with
/// `#[component(non_send)]`.
pub trait ThreadSafeComponent: Send + Sync {}

#[diagnostic::do_not_recommend]
impl<T: Send + Sync> ThreadSafeComponent for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::mem::size_of;

    // Published FNV-1a 128-bit test vectors (from the FNV reference test suite).
    const FNV_OFFSET_BASIS: u128 = 0x6c62272e07bb014262b821756295c58d;
    const FNV_A: u128 = 0xd228cb696f1a8caf78912b704e4a8964;
    const FNV_FOOBAR: u128 = 0x343e1662793c64bf6f0d3597ba446f18;

    #[test]
    fn key_is_16_bytes() {
        assert_eq!(size_of::<ComponentKey>(), 16);
    }

    #[test]
    fn empty_path_hashes_to_the_fnv_offset_basis() {
        // FNV-1a of zero bytes is the offset basis by definition — this pins
        // the constant and catches an off-by-one in the loop.
        assert_eq!(ComponentKey::from_path("").0, FNV_OFFSET_BASIS);
    }

    #[test]
    fn matches_published_fnv1a_128_test_vectors() {
        // Pins the algorithm as FNV-1a (xor THEN multiply), the prime, and
        // byte order. A home-grown variant would silently diverge.
        assert_eq!(ComponentKey::from_path("a").0, FNV_A);
        assert_eq!(ComponentKey::from_path("foobar").0, FNV_FOOBAR);
    }

    #[test]
    fn is_deterministic() {
        let p = "my_crate::physics::Position";
        assert_eq!(ComponentKey::from_path(p), ComponentKey::from_path(p));
    }

    #[test]
    fn distinct_paths_give_distinct_keys() {
        // Realistic paths plus adversarial near-misses: case, separator
        // placement, prefixes, and unicode.
        let paths = [
            "",
            "a",
            "A",
            "a::b",
            "a::B",
            "ab::c",
            "a::bc",
            "a::b::c",
            "a::b::",
            "::a::b",
            "my_crate::Position",
            "my_crate::position",
            "my_crate::Position2",
            "my_crate::physics::Position",
            "my_crate::render::Position",
            "my_crate::Velocity",
            "flux_renderer::instance::SurfaceProviderResource",
            "flux_renderer::instance::VulkanInstance",
            "ünïcode::Pösition",
        ];
        let keys: HashSet<ComponentKey> =
            paths.iter().map(|p| ComponentKey::from_path(p)).collect();
        assert_eq!(
            keys.len(),
            paths.len(),
            "every distinct path must yield a distinct key"
        );
    }

    #[test]
    fn is_usable_in_const_context_and_const_comparable() {
        // Keys must be usable as `const` items and comparable in const context.
        const POS: ComponentKey = ComponentKey::from_path("x::Position");
        const VEL: ComponentKey = ComponentKey::from_path("x::Velocity");
        const _: () = assert!(POS != VEL);
        const _: () = assert!(POS == ComponentKey::from_path("x::Position"));
        assert_ne!(POS, VEL);
    }

    #[test]
    #[allow(
        clippy::assertions_on_constants,
        reason = "deliberately checks compile-time constants"
    )]
    fn const_equality_agrees_with_runtime_equality() {
        const A: ComponentKey = ComponentKey::from_path("a");
        const B: ComponentKey = ComponentKey::from_path("b");
        const A_EQ_A: bool = A == A;
        const A_EQ_B: bool = A == B;
        assert_eq!(A_EQ_A, A == A);
        assert_eq!(A_EQ_B, A == B);
        assert!(A_EQ_A);
        assert!(!A_EQ_B);
    }

    #[test]
    fn debug_prints_fixed_width_hex() {
        let k = ComponentKey::from_path("");
        assert_eq!(
            format!("{k:?}"),
            "ComponentKey(0x6c62272e07bb014262b821756295c58d)"
        );
        // Small values are zero-padded to the full 32 hex digits.
        assert_eq!(
            format!("{:?}", ComponentKey(1)),
            format!("ComponentKey({:#034x})", 1u128)
        );
        assert!(format!("{:?}", ComponentKey(1)).ends_with("0001)"));
    }

    #[test]
    #[allow(
        clippy::assertions_on_constants,
        reason = "deliberately checks compile-time constants"
    )]
    fn manual_component_impl_gets_the_documented_defaults() {
        struct Manual;
        impl Component for Manual {
            const KEY: ComponentKey = ComponentKey::from_path("tests::Manual");
        }
        assert_eq!(Manual::STORAGE, StorageClass::Chunked);
        assert!(!Manual::NON_SEND);
        assert_eq!(Manual::KEY, ComponentKey::from_path("tests::Manual"));
    }

    #[test]
    fn component_key_is_hashable_and_copy() {
        let k = ComponentKey::from_path("k");
        let copy = k;
        let mut set = HashSet::new();
        set.insert(k);
        assert!(set.contains(&copy));
    }
}
