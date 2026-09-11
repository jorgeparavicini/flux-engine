use crate::entity::Entity;
use crate::relation::RelationTarget;
use crate::{Component, ComponentKey, StorageClass};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Hasher for keys that already are hashes: passes the low 64 bits through.
#[derive(Default)]
pub(crate) struct KeyHasher(u64);

impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // ComponentKey hashes as one u128 write; fold it to 64 bits.
        let mut lo = [0u8; 8];
        lo.copy_from_slice(&bytes[..8]);
        self.0 = u64::from_le_bytes(lo);
    }

    fn write_u128(&mut self, value: u128) {
        self.0 = value as u64;
    }
}

pub(crate) type KeyMap<V> = HashMap<ComponentKey, V, BuildHasherDefault<KeyHasher>>;

/// Dense per-world index of a registered component type.
///
/// Assigned in registration order; only meaningful within the world whose
/// registry produced it. For a world-independent identity use
/// [`ComponentKey`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComponentId(pub(crate) u64);

pub(crate) struct ComponentInfo {
    pub key: ComponentKey,
    pub name: &'static str,
    pub size: usize,
    pub align: usize,
    pub drop_fn: Option<unsafe fn(*mut u8, usize)>,
    pub toggleable: bool,
    pub on_add: Option<crate::world::Hook>,
    pub on_remove: Option<crate::world::Hook>,
    /// The `(relation, target)` this id stands for, if it is a relation pair.
    pub relation: Option<RelationTarget>,
    #[cfg(debug_assertions)]
    type_id: std::any::TypeId,
}

/// Per-world registry mapping component types to ids and layout metadata.
#[derive(Default)]
pub struct Registry {
    infos: Vec<ComponentInfo>,
    by_key: KeyMap<ComponentId>,
    by_pair: HashMap<(ComponentKey, Entity), ComponentId>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `T`'s id, registering it on first sight.
    ///
    /// # Panics
    ///
    /// If `T` is a `Tag` component with a nonzero size, or (in debug builds)
    /// if two distinct types share a [`ComponentKey`].
    pub fn register<T: Component>(&mut self) -> ComponentId {
        if let Some(&id) = self.by_key.get(&T::KEY) {
            #[cfg(debug_assertions)]
            {
                let existing = &self.infos[id.0 as usize];
                assert_eq!(
                    existing.type_id,
                    std::any::TypeId::of::<T>(),
                    "ComponentKey collision: `{}` and `{}` share a key",
                    existing.name,
                    std::any::type_name::<T>()
                );
            }
            return id;
        }

        assert!(
            T::STORAGE != StorageClass::Tag || size_of::<T>() == 0,
            "tag components must be zero-sized"
        );
        let id =
            ComponentId(u64::try_from(self.infos.len()).expect("component id space exhausted"));
        self.infos.push(ComponentInfo {
            key: T::KEY,
            name: std::any::type_name::<T>(),
            size: size_of::<T>(),
            align: align_of::<T>(),
            drop_fn: std::mem::needs_drop::<T>()
                .then_some(drop_in_place_n::<T> as unsafe fn(*mut u8, usize)),
            toggleable: T::TOGGLEABLE,
            on_add: T::ON_ADD,
            on_remove: T::ON_REMOVE,
            relation: None,
            #[cfg(debug_assertions)]
            type_id: std::any::TypeId::of::<T>(),
        });
        self.by_key.insert(T::KEY, id);
        id
    }

    /// Returns the id for the `(relation, target)` pair, registering it on
    /// first sight. Pairs are zero-sized tags; the target is carried in the
    /// registry entry, not in storage.
    pub(crate) fn register_relation(
        &mut self,
        relation: ComponentKey,
        target: Entity,
    ) -> ComponentId {
        if let Some(&id) = self.by_pair.get(&(relation, target)) {
            return id;
        }
        let id =
            ComponentId(u64::try_from(self.infos.len()).expect("component id space exhausted"));
        self.infos.push(ComponentInfo {
            key: relation,
            name: "relation",
            size: 0,
            align: 1,
            drop_fn: None,
            toggleable: false,
            on_add: None,
            on_remove: None,
            relation: Some(RelationTarget { relation, target }),
            #[cfg(debug_assertions)]
            type_id: std::any::TypeId::of::<()>(),
        });
        self.by_pair.insert((relation, target), id);
        id
    }

    /// The id of the `(relation, target)` pair, if it has been registered.
    pub(crate) fn lookup_pair(&self, relation: ComponentKey, target: Entity) -> Option<ComponentId> {
        self.by_pair.get(&(relation, target)).copied()
    }

    /// Drops the `(relation, target)` lookup entry. The registry slot itself
    /// is not reclaimed; see PERFORMANCE.md.
    pub(crate) fn forget_pair(&mut self, relation: ComponentKey, target: Entity) {
        self.by_pair.remove(&(relation, target));
    }

    /// The id of the component registered under `key`, if any.
    pub(crate) fn lookup(&self, key: ComponentKey) -> Option<ComponentId> {
        self.by_key.get(&key).copied()
    }

    pub(crate) fn info(&self, id: ComponentId) -> &ComponentInfo {
        &self.infos[id.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.infos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.infos.is_empty()
    }
}

unsafe fn drop_in_place_n<T>(ptr: *mut u8, count: usize) {
    unsafe {
        std::ptr::drop_in_place(std::ptr::slice_from_raw_parts_mut(ptr.cast::<T>(), count));
    }
}

#[cfg(test)]
mod tests {
    use crate::component::{Component, ComponentKey, StorageClass};
    use crate::registry::{ComponentId, Registry};
    use std::cell::Cell;
    use std::mem::{MaybeUninit, size_of};
    use std::rc::Rc;

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("registry::tests::", stringify!($name)));
            }
        };
    }

    struct Pos(#[allow(dead_code)] f32, #[allow(dead_code)] f32);
    component!(Pos);

    struct Vel(#[allow(dead_code)] f32);
    component!(Vel);

    struct Marker;
    impl Component for Marker {
        const KEY: ComponentKey = ComponentKey::from_path("registry::tests::Marker");
        const STORAGE: StorageClass = StorageClass::Tag;
    }

    struct Sparse(#[allow(dead_code)] u8);
    impl Component for Sparse {
        const KEY: ComponentKey = ComponentKey::from_path("registry::tests::Sparse");
        const STORAGE: StorageClass = StorageClass::SparseSet;
    }

    struct Pinned(#[allow(dead_code)] Rc<u8>);
    impl Component for Pinned {
        const KEY: ComponentKey = ComponentKey::from_path("registry::tests::Pinned");
        const NON_SEND: bool = true;
    }

    #[repr(align(16))]
    struct Aligned16(#[allow(dead_code)] [u8; 4]);
    component!(Aligned16);

    struct Owns(#[allow(dead_code)] String);
    component!(Owns);

    struct DropCounter(Rc<Cell<usize>>);
    component!(DropCounter);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    // ------------------------------------------------------------- id assignment

    #[test]
    fn ids_are_dense_and_sequential_from_zero() {
        let mut r = Registry::new();
        let a = r.register::<Pos>();
        let b = r.register::<Vel>();
        let c = r.register::<Marker>();
        assert_eq!(a, ComponentId(0));
        assert_eq!(b, ComponentId(1));
        assert_eq!(c, ComponentId(2));
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn register_is_idempotent() {
        let mut r = Registry::new();
        let first = r.register::<Pos>();
        let _ = r.register::<Vel>();
        let again = r.register::<Pos>();
        assert_eq!(first, again, "re-registering returns the existing id");
        assert_eq!(r.len(), 2, "no duplicate entry is created");
    }

    #[test]
    fn component_id_is_small_and_orderable() {
        assert_eq!(size_of::<ComponentId>(), 8);
        let mut ids = [ComponentId(2), ComponentId(0), ComponentId(1)];
        ids.sort();
        assert_eq!(ids, [ComponentId(0), ComponentId(1), ComponentId(2)]);
    }

    #[test]
    fn lookup_finds_registered_keys_only() {
        let mut r = Registry::new();
        let pos = r.register::<Pos>();
        assert_eq!(r.lookup(Pos::KEY), Some(pos));
        assert_eq!(r.lookup(Vel::KEY), None);
    }

    #[test]
    fn empty_registry() {
        let r = Registry::default();
        assert_eq!(r.len(), 0);
        assert!(r.is_empty());
    }

    // ----------------------------------------------------------------- metadata

    #[test]
    fn info_records_layout_facts() {
        let mut r = Registry::new();
        let pos = r.register::<Pos>();
        let aligned = r.register::<Aligned16>();
        let marker = r.register::<Marker>();

        let i = r.info(pos);
        assert_eq!(i.key, Pos::KEY);
        assert_eq!(i.size, size_of::<Pos>());
        assert_eq!(i.align, align_of::<Pos>());
        assert!(
            i.name.contains("Pos"),
            "name should identify the type, got {:?}",
            i.name
        );

        let i = r.info(aligned);
        assert_eq!((i.size, i.align), (16, 16), "repr(align) is respected");

        let i = r.info(marker);
        assert_eq!((i.size, i.align), (0, 1));
    }

    #[test]
    fn tag_and_sparse_components_still_register() {
        let mut r = Registry::new();
        let tag = r.register::<Marker>();
        let sparse = r.register::<Sparse>();
        let pinned = r.register::<Pinned>();
        assert_eq!(r.info(tag).size, 0);
        assert!(r.lookup(Sparse::KEY) == Some(sparse));
        assert!(r.lookup(Pinned::KEY) == Some(pinned));
    }

    // ------------------------------------------------------------------ drop_fn

    #[test]
    fn drop_fn_is_none_for_types_that_need_no_drop() {
        let mut r = Registry::new();
        let pos = r.register::<Pos>();
        let tag = r.register::<Marker>();
        assert!(r.info(pos).drop_fn.is_none());
        assert!(r.info(tag).drop_fn.is_none());
    }

    #[test]
    fn drop_fn_is_some_for_types_that_own_memory() {
        let mut r = Registry::new();
        let owns = r.register::<Owns>();
        assert!(r.info(owns).drop_fn.is_some());
    }

    #[test]
    fn drop_fn_drops_each_element_exactly_once() {
        let mut r = Registry::new();
        let id = r.register::<DropCounter>();
        let drop_fn = r.info(id).drop_fn.expect("DropCounter needs drop");

        let drops = Rc::new(Cell::new(0));
        let mut buf: [MaybeUninit<DropCounter>; 3] = [const { MaybeUninit::uninit() }; 3];
        for slot in &mut buf {
            slot.write(DropCounter(Rc::clone(&drops)));
        }
        assert_eq!(drops.get(), 0);
        unsafe { drop_fn(buf.as_mut_ptr().cast(), 3) };
        assert_eq!(drops.get(), 3, "all three elements dropped, none twice");
    }

    #[test]
    fn drop_fn_with_count_zero_is_a_no_op() {
        let mut r = Registry::new();
        let id = r.register::<DropCounter>();
        let drop_fn = r.info(id).drop_fn.expect("needs drop");
        unsafe {
            drop_fn(
                std::ptr::NonNull::<DropCounter>::dangling().as_ptr().cast(),
                0,
            )
        };
        // nothing to assert beyond "no crash / no UB" — miri checks the rest
    }

    // ---------------------------------------------------------------- guardrails

    #[test]
    #[should_panic(expected = "zero-sized")]
    fn tag_component_with_nonzero_size_is_rejected() {
        struct FatTag(#[allow(dead_code)] u32);
        impl Component for FatTag {
            const KEY: ComponentKey = ComponentKey::from_path("registry::tests::FatTag");
            const STORAGE: StorageClass = StorageClass::Tag;
        }
        Registry::new().register::<FatTag>();
    }

    #[test]
    #[should_panic(expected = "collision")]
    #[cfg(debug_assertions)]
    fn key_collision_between_distinct_types_panics_in_debug() {
        struct First;
        impl Component for First {
            const KEY: ComponentKey = ComponentKey::from_path("registry::tests::SharedKey");
        }
        struct Second;
        impl Component for Second {
            const KEY: ComponentKey = ComponentKey::from_path("registry::tests::SharedKey");
        }
        let mut r = Registry::new();
        r.register::<First>();
        r.register::<Second>(); // same KEY, different TypeId: must panic loudly
    }
}
