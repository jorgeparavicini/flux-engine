use crate::grant::AccessGrant;
use crate::query::data::QueryData;
use crate::query::filter::QueryFilter;
use crate::registry::{ComponentId, KeyMap, Registry};
use crate::storage::alloc::ChunkAlloc;
use crate::storage::archetype::{ArchetypeId, Archetypes};
use crate::storage::chunks::{ChunkId, Chunks};
use crate::storage::ops;
use crate::{Bundle, Component, Entities, Entity, Query, QueryState};

/// A collection of entities and their components.
///
/// ```
/// use flux_ecs2::{Component, World};
///
/// #[derive(Component, Debug, PartialEq)]
/// struct Health(u32);
///
/// let mut world = World::new();
/// let entity = world.spawn(Health(100));
/// world.insert(entity, Health(50)); // replaces
/// assert_eq!(world.remove::<Health>(entity), Some(Health(50)));
/// ```
#[derive(Default)]
pub struct World {
    entities: Entities,
    singletons: KeyMap<Entity>,
    registry: Registry,
    archetypes: Archetypes,
    chunks: Chunks,
    alloc: ChunkAlloc,
    version: u64,
}

/// Shared view of a world during one system run: what parameters fetch from.
pub struct WorldCells<'w> {
    pub(crate) chunks: &'w Chunks,
    pub(crate) archetypes: &'w Archetypes,
    pub(crate) reg: &'w Registry,
    pub(crate) entities: &'w Entities,
}

impl World {
    /// Creates an empty world.
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawns an entity with the bundle's components.
    ///
    /// # Panics
    ///
    /// If the bundle contains the same component more than once.
    pub fn spawn<B: Bundle>(&mut self, bundle: B) -> Entity {
        let entity = self.entities.alloc();
        self.place(entity, bundle);
        entity
    }

    /// Spawns `bundle` onto a reserved entity id.
    pub(crate) fn spawn_reserved<B: Bundle>(&mut self, entity: Entity, bundle: B) {
        self.entities.alloc_specific(entity);
        self.place(entity, bundle);
    }

    /// Places a freshly allocated entity's components; shared spawn tail.
    fn place<B: Bundle>(&mut self, entity: Entity, bundle: B) {
        let field_ids = B::ids(&mut self.registry);
        let mut signature = field_ids.clone();
        signature.sort();
        assert!(
            signature.windows(2).all(|w| w[0] != w[1]),
            "bundle contains a duplicate component"
        );
        let arch_id = self
            .archetypes
            .get_or_create(&signature, &self.registry)
            .expect("bundle is too large to fit in a single chunk");
        let (chunk, row) = {
            let arch = self.archetypes.get_mut(arch_id);
            unsafe { ops::alloc_row(arch, arch_id, &mut self.chunks, &mut self.alloc, entity) }
        };

        let arch = self.archetypes.get(arch_id);
        let mut index = 0;
        unsafe {
            bundle.write(&mut |src| {
                let column = arch
                    .signature()
                    .binary_search(&field_ids[index])
                    .expect("bundle component in signature");
                ops::write_component(
                    &self.chunks,
                    &arch.layout,
                    &self.registry,
                    chunk,
                    column,
                    row,
                    src,
                );
                index += 1;
            });
        }

        let slot = self.entities.slot_mut(entity).expect("just allocated");
        slot.chunk = chunk.0;
        slot.row = row;
        self.version += 1;
        self.stamp_all_columns(chunk, arch_id, true);
    }

    /// Despawns `entity`, dropping all of its components.
    ///
    /// Returns false on a dead or stale handle, leaving the world unchanged.
    pub fn despawn(&mut self, entity: Entity) -> bool {
        let Some(slot) = self.entities.slot(entity) else {
            return false;
        };
        let (chunk, row) = (ChunkId(slot.chunk), slot.row);
        let arch_id = self.chunks.archetype(chunk);
        let arch = self.archetypes.get_mut(arch_id);
        let swapped = unsafe {
            ops::swap_remove_row(arch, &mut self.chunks, &mut self.alloc, &self.registry, chunk, row, true)
        };
        self.fix_swapped_slot(swapped, chunk, row);
        self.entities.dealloc(entity)
    }

    /// Whether `entity` refers to a live entity.
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.entities.is_alive(entity)
    }

    /// Number of live entities.
    pub fn len(&self) -> usize {
        self.entities.live_count()
    }

    /// Whether the world has no live entities.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A reference to `entity`'s `T`.
    ///
    /// None if the entity is dead or does not have the component.
    pub fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        let (chunk, row, column, arch_id) = self.locate::<T>(entity)?;
        let arch = self.archetypes.get(arch_id);
        unsafe {
            Some(
                &*ops::component_ptr(
                    &self.chunks,
                    &arch.layout,
                    &self.registry,
                    chunk,
                    column,
                    row,
                )
                    .cast::<T>(),
            )
        }
    }

    /// A mutable reference to `entity`'s `T`.
    ///
    /// None if the entity is dead or does not have the component.
    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let (chunk, row, column, arch_id) = self.locate::<T>(entity)?;
        self.version += 1;
        self.chunks.stamp_write_version(chunk, column, self.version);
        let arch = self.archetypes.get(arch_id);
        unsafe {
            Some(
                &mut *ops::component_ptr(
                    &self.chunks,
                    &arch.layout,
                    &self.registry,
                    chunk,
                    column,
                    row,
                )
                    .cast::<T>(),
            )
        }
    }

    /// Whether `entity` is alive and has a `T`.
    pub fn has<T: Component>(&self, entity: Entity) -> bool {
        self.locate::<T>(entity).is_some()
    }

    /// Adds `value` to `entity`, or replaces the entity's existing `T`,
    /// dropping the old value.
    ///
    /// Returns false on a dead or stale handle; the value is dropped.
    pub fn insert<T: Component>(&mut self, entity: Entity, value: T) -> bool {
        let Some(slot) = self.entities.slot(entity) else {
            return false;
        };
        let (chunk, row) = (ChunkId(slot.chunk), slot.row);
        let id = self.registry.register::<T>();
        let src_id = self.chunks.archetype(chunk);

        if let Ok(column) = self.archetypes.get(src_id).signature().binary_search(&id) {
            let arch = self.archetypes.get(src_id);
            unsafe {
                let ptr = ops::component_ptr(&self.chunks, &arch.layout, &self.registry, chunk, column, row)
                    .cast::<T>();
                ptr.drop_in_place();
                ptr.write(value);
            }
            self.version += 1;
            self.chunks.stamp_write_version(chunk, column, self.version);
            return true;
        }

        let dst_id = self.add_edge_target(src_id, id);
        let (src_arch, dst_arch) = self.archetypes.get_pair_mut(src_id, dst_id);
        let (dst_chunk, dst_row, swapped) = unsafe {
            ops::move_row(
                src_arch,
                dst_arch,
                dst_id,
                &mut self.chunks,
                &mut self.alloc,
                &self.registry,
                chunk,
                row,
                true,
            )
        };
        let dst_column = dst_arch
            .signature()
            .binary_search(&id)
            .expect("inserted component is in the target signature");
        unsafe {
            ops::component_ptr(&self.chunks, &dst_arch.layout, &self.registry, dst_chunk, dst_column, dst_row)
                .cast::<T>()
                .write(value);
        }
        let slot = self.entities.slot_mut(entity).expect("checked live above");
        slot.chunk = dst_chunk.0;
        slot.row = dst_row;
        self.fix_swapped_slot(swapped, chunk, row);
        self.version += 1;
        self.stamp_all_columns(dst_chunk, dst_id, true);
        true
    }

    /// Takes `T` off `entity` and returns it.
    ///
    /// None if the entity is dead or does not have the component.
    pub fn remove<T: Component>(&mut self, entity: Entity) -> Option<T> {
        let (chunk, row, column, arch_id) = self.locate::<T>(entity)?;
        let id = self.registry.lookup(T::KEY).expect("located above");
        let arch = self.archetypes.get(arch_id);
        let value = unsafe {
            ops::component_ptr(&self.chunks, &arch.layout, &self.registry, chunk, column, row)
                .cast::<T>()
                .read()
        };
        let dst_id = self.remove_edge_target(arch_id, id);
        let (src_arch, dst_arch) = self.archetypes.get_pair_mut(arch_id, dst_id);
        let (dst_chunk, dst_row, swapped) = unsafe {
            ops::move_row(
                src_arch,
                dst_arch,
                dst_id,
                &mut self.chunks,
                &mut self.alloc,
                &self.registry,
                chunk,
                row,
                false,
            )
        };
        let slot = self.entities.slot_mut(entity).expect("located above");
        slot.chunk = dst_chunk.0;
        slot.row = dst_row;
        self.fix_swapped_slot(swapped, chunk, row);
        self.version += 1;
        self.stamp_all_columns(dst_chunk, dst_id, true);
        Some(value)
    }

    /// Spawns or replaces the world's single `T`, returning its entity.
    ///
    /// The first call spawns a dedicated entity holding the value; later
    /// calls replace the value on that entity.
    pub fn insert_singleton<T: Component>(&mut self, value: T) -> Entity {
        if let Some(&entity) = self.singletons.get(&T::KEY)
            && self.is_alive(entity)
        {
            self.insert(entity, value);
            return entity;
        }
        let entity = self.spawn((value,));
        self.singletons.insert(T::KEY, entity);
        entity
    }

    /// The world's single `T`, if one was inserted.
    pub fn singleton<T: Component>(&self) -> Option<&T> {
        let entity = *self.singletons.get(&T::KEY)?;
        self.get::<T>(entity)
    }

    /// Despawns the entity holding the world's single `T`.
    ///
    /// Returns false when no live singleton `T` exists.
    pub fn remove_singleton<T: Component>(&mut self) -> bool {
        let Some(entity) = self.singletons.remove(&T::KEY) else {
            return false;
        };
        self.despawn(entity)
    }

    /// Mutable access to the world's single `T`, if one was inserted.
    pub fn singleton_mut<T: Component>(&mut self) -> Option<&mut T> {
        let entity = *self.singletons.get(&T::KEY)?;
        self.get_mut::<T>(entity)
    }

    /// The archetype `entity`'s components would move to when adding `id`,
    /// creating it on first use and caching the edge.
    fn add_edge_target(&mut self, src: ArchetypeId, id: ComponentId) -> ArchetypeId {
        if let Some(&dst) = self.archetypes.get(src).edge_add.get(&id) {
            return dst;
        }
        let mut signature = self.archetypes.get(src).signature().to_vec();
        let position = signature
            .binary_search(&id)
            .expect_err("component is not in the source signature");
        signature.insert(position, id);
        let dst = self
            .archetypes
            .get_or_create(&signature, &self.registry)
            .expect("signature is too large to fit in a single chunk");
        self.archetypes.get_mut(src).edge_add.insert(id, dst);
        dst
    }

    /// The archetype `entity`'s components would move to when removing `id`,
    /// creating it on first use, and caching the edge.
    fn remove_edge_target(&mut self, src: ArchetypeId, id: ComponentId) -> ArchetypeId {
        if let Some(&dst) = self.archetypes.get(src).edge_remove.get(&id) {
            return dst;
        }
        let mut signature = self.archetypes.get(src).signature().to_vec();
        let position = signature
            .binary_search(&id)
            .expect("component is in the source signature");
        signature.remove(position);
        let dst = self
            .archetypes
            .get_or_create(&signature, &self.registry)
            .expect("a shrunk signature always has a layout");
        self.archetypes.get_mut(src).edge_remove.insert(id, dst);
        dst
    }

    /// A query over this world's entities, using `state`'s cached matches.
    ///
    /// The query shape is checked at compile time: a shape whose access
    /// conflicts with itself is rejected during code generation.
    ///
    /// ```
    /// use flux_ecs2::{Component, QueryState, World};
    ///
    /// #[derive(Component)]
    /// struct Health(u32);
    ///
    /// let mut world = World::new();
    /// world.spawn(Health(10));
    /// let mut state = QueryState::<&mut Health>::new();
    /// for column in world.query(&mut state).chunks() {
    ///     for health in column {
    ///         health.0 += 1;
    ///     }
    /// }
    /// ```
    ///
    /// Two mutable accesses to one component alias and do not compile:
    ///
    /// ```compile_fail,E0080
    /// use flux_ecs2::{Component, QueryState, World};
    ///
    /// #[derive(Component)]
    /// struct Health(u32);
    ///
    /// let mut world = World::new();
    /// let mut state = QueryState::<(&mut Health, &mut Health)>::new();
    /// let _ = world.query(&mut state); // error: query aliases a component mutably
    /// ```
    ///
    /// Neither does a shared access alongside a mutable one:
    ///
    /// ```compile_fail,E0080
    /// use flux_ecs2::{Component, QueryState, World};
    ///
    /// #[derive(Component)]
    /// struct Health(u32);
    ///
    /// let mut world = World::new();
    /// let mut state = QueryState::<(&Health, &mut Health)>::new();
    /// let _ = world.query(&mut state); // error: query aliases a component mutably
    /// ```
    pub fn query<'w, 's, D: QueryData, F: QueryFilter>(
        &'w mut self,
        state: &'s mut QueryState<D, F>,
    ) -> Query<'w, 's, D, F> {
        const { assert!(!D::ACCESS.self_conflicting(), "query aliases a component mutably") }
        self.version += 1;
        let last_seen = state.advance_cursor(self.version);
        state.refresh(&self.archetypes, &self.registry);
        Query {
            chunks: &self.chunks,
            archetypes: &self.archetypes,
            reg: &self.registry,
            grant: AccessGrant::at_version(D::ACCESS, self.version),
            state,
            last_seen,
        }
    }

    /// Runs `system` once over this world.
    pub fn run(&mut self, system: &mut impl crate::System) {
        system.run(self);
    }

    pub(crate) fn bump_version(&mut self) -> u64 {
        self.version += 1;
        self.version
    }

    pub(crate) fn cells(&self) -> WorldCells<'_> {
        WorldCells {
            chunks: &self.chunks,
            archetypes: &self.archetypes,
            reg: &self.registry,
            entities: &self.entities,
        }
    }

    fn locate<T: Component>(&self, entity: Entity) -> Option<(ChunkId, u16, usize, ArchetypeId)> {
        let id = self.registry.lookup(T::KEY)?;
        let slot = self.entities.slot(entity)?;
        let chunk = ChunkId(slot.chunk);
        let arch_id = self.chunks.archetype(chunk);
        let column = self
            .archetypes
            .get(arch_id)
            .signature()
            .binary_search(&id)
            .ok()?;
        Some((chunk, slot.row, column, arch_id))
    }

    fn fix_swapped_slot(&mut self, swapped: Option<Entity>, chunk: ChunkId, row: u16) {
        if let Some(moved) = swapped {
            let slot = self
                .entities
                .slot_mut(moved)
                .expect("swapped entity is live");
            slot.chunk = chunk.0;
            slot.row = row;
        }
    }

    fn stamp_all_columns(&self, chunk: ChunkId, arch_id: ArchetypeId, added: bool) {
        for column in 0..self.archetypes.get(arch_id).signature().len() {
            self.chunks.stamp_write_version(chunk, column, self.version);
            if added {
                self.chunks.stamp_added_version(chunk, column, self.version);
            }
        }
    }
}

impl Drop for World {
    /// Drops every live component value and releases all chunks.
    fn drop(&mut self) {
        for index in 0..self.archetypes.len() {
            let arch_id = ArchetypeId(index as u32);
            let chunk_ids = self.archetypes.get(arch_id).chunks.clone();
            for chunk in chunk_ids {
                let arch = self.archetypes.get(arch_id);
                let rows = self.chunks.len(chunk) as usize;
                for (column, component_id) in arch.layout.components.iter().enumerate() {
                    if let Some(drop_fn) = self.registry.info(*component_id).drop_fn {
                        unsafe {
                            drop_fn(
                                ops::component_ptr(&self.chunks, &arch.layout, &self.registry, chunk, column, 0),
                                rows,
                            );
                        }
                    }
                }
                self.chunks.set_len(chunk, 0);
                self.chunks.destroy(&mut self.alloc, chunk);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{Component, ComponentKey, StorageClass};
    use std::cell::Cell;
    use std::rc::Rc;

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("world::tests::", stringify!($name)));
            }
        };
    }

    #[derive(Copy, Clone, PartialEq, Debug)]
    struct A(u64);
    component!(A);
    #[derive(Copy, Clone, PartialEq, Debug)]
    struct B(u16);
    component!(B);
    #[derive(Copy, Clone, PartialEq, Debug)]
    struct Big([u64; 64]);
    component!(Big);
    #[derive(Copy, Clone, PartialEq, Debug)]
    struct Marker;
    impl Component for Marker {
        const KEY: ComponentKey = ComponentKey::from_path("world::tests::Marker");
        const STORAGE: StorageClass = StorageClass::Tag;
    }
    struct DropCounter(Rc<Cell<usize>>, u64);
    component!(DropCounter);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    fn counter() -> (Rc<Cell<usize>>, impl Fn(u64) -> DropCounter) {
        let cell = Rc::new(Cell::new(0));
        let c = Rc::clone(&cell);
        (cell, move |payload| DropCounter(Rc::clone(&c), payload))
    }

    // ------------------------------------------------------------------ spawn

    #[test]
    fn spawn_empty_entity() {
        let mut w = World::new();
        let e = w.spawn(());
        assert!(w.is_alive(e));
        assert_eq!(w.len(), 1);
        assert!(!w.has::<A>(e));
        assert!(w.get::<A>(e).is_none());
    }

    #[test]
    fn spawn_bundle_roundtrip() {
        let mut w = World::new();
        let e = w.spawn((A(1), B(2)));
        assert_eq!(w.get::<A>(e), Some(&A(1)));
        assert_eq!(w.get::<B>(e), Some(&B(2)));
        assert!(w.has::<A>(e) && w.has::<B>(e));
        assert!(w.get::<Big>(e).is_none());

        let single = w.spawn(A(9));
        assert_eq!(w.get::<A>(single), Some(&A(9)));
        assert!(!w.has::<B>(single));
    }

    #[test]
    fn tuple_field_order_is_irrelevant_to_identity() {
        let mut w = World::new();
        let ab = w.spawn((A(1), B(2)));
        let ba = w.spawn((B(4), A(3)));
        assert_eq!(w.get::<A>(ab), Some(&A(1)));
        assert_eq!(w.get::<B>(ba), Some(&B(4)));
        assert_eq!(w.archetypes.len(), 1, "same component set, same archetype");
    }

    #[test]
    #[should_panic(expected = "duplicate")]
    fn duplicate_component_in_bundle_panics() {
        let mut w = World::new();
        let _ = w.spawn((A(1), A(2)));
    }

    #[test]
    fn spawning_across_chunk_boundaries_keeps_every_entity_addressable() {
        let mut w = World::new();
        // Big makes capacity small enough that 300 spawns span several chunks.
        let entities: Vec<_> = (0..300u64).map(|i| w.spawn((Big([i; 64]), A(i)))).collect();
        assert_eq!(w.len(), 300);
        for (i, e) in entities.iter().enumerate() {
            assert_eq!(w.get::<A>(*e), Some(&A(i as u64)));
            assert_eq!(w.get::<Big>(*e).unwrap().0[0], i as u64);
        }
    }

    // -------------------------------------------------------------------- get

    #[test]
    fn get_mut_writes_persist() {
        let mut w = World::new();
        let e = w.spawn((A(1),));
        w.get_mut::<A>(e).unwrap().0 = 42;
        assert_eq!(w.get::<A>(e), Some(&A(42)));
        assert!(w.get_mut::<B>(e).is_none());
    }

    #[test]
    fn get_of_a_type_never_seen_anywhere_is_none() {
        let mut w = World::new();
        let e = w.spawn(());
        // Big was never registered by any spawn/insert in this world.
        assert!(w.get::<Big>(e).is_none());
        assert!(!w.has::<Big>(e));
        assert!(w.remove::<Big>(e).is_none());
    }

    #[test]
    fn zst_components_are_present_and_removable() {
        let mut w = World::new();
        let e = w.spawn((A(1), Marker));
        assert!(w.has::<Marker>(e));
        assert_eq!(w.get::<Marker>(e), Some(&Marker));
        assert_eq!(w.remove::<Marker>(e), Some(Marker));
        assert!(!w.has::<Marker>(e));
        assert!(w.insert(e, Marker));
        assert!(w.has::<Marker>(e));
    }

    // ---------------------------------------------------------------- despawn

    #[test]
    fn despawn_kills_drops_and_rejects_stale_handles() {
        let mut w = World::new();
        let (drops, make) = counter();
        let e = w.spawn((A(1),));
        w.insert(e, make(0));
        assert!(w.despawn(e));
        assert_eq!(drops.get(), 1, "despawn drops components");
        assert!(!w.is_alive(e));
        assert!(w.get::<A>(e).is_none());
        assert!(!w.despawn(e));
        assert_eq!(w.len(), 0);
    }

    #[test]
    fn despawn_updates_the_swapped_entitys_slot() {
        let mut w = World::new();
        let entities: Vec<_> = (0..5u64).map(|i| w.spawn((A(i), B(i as u16)))).collect();
        // Removing a middle entity relocates the tail row; every survivor must
        // still resolve to its own values afterwards.
        assert!(w.despawn(entities[1]));
        for (i, e) in entities.iter().enumerate() {
            if i == 1 {
                assert!(!w.is_alive(*e));
            } else {
                assert_eq!(w.get::<A>(*e), Some(&A(i as u64)), "entity {i} slot intact");
                assert_eq!(w.get::<B>(*e), Some(&B(i as u16)));
            }
        }
    }

    #[test]
    fn slot_reuse_does_not_leak_component_visibility() {
        let mut w = World::new();
        let old = w.spawn((A(7),));
        w.despawn(old);
        let new = w.spawn((B(8),)); // recycles the index
        assert!(w.get::<A>(old).is_none(), "stale handle sees nothing");
        assert!(w.get::<B>(old).is_none());
        assert!(!w.has::<A>(new), "old occupant's components must not leak");
        assert_eq!(w.get::<B>(new), Some(&B(8)));
    }

    // ----------------------------------------------------------------- insert

    #[test]
    fn insert_new_component_moves_the_entity_and_keeps_neighbors_intact() {
        let mut w = World::new();
        let stays = w.spawn((A(1),));
        let moves = w.spawn((A(2),));
        let tail = w.spawn((A(3),));

        assert!(w.insert(moves, B(20)));
        assert_eq!(
            w.get::<A>(moves),
            Some(&A(2)),
            "moved entity keeps its data"
        );
        assert_eq!(w.get::<B>(moves), Some(&B(20)));
        // the move vacated a row in the source archetype; the swapped-in
        // entity's slot must have followed
        assert_eq!(w.get::<A>(stays), Some(&A(1)));
        assert_eq!(w.get::<A>(tail), Some(&A(3)));
        assert!(!w.has::<B>(stays));
    }

    #[test]
    fn insert_replaces_in_place_and_drops_the_old_value() {
        let mut w = World::new();
        let (drops, make) = counter();
        let e = w.spawn(());
        assert!(w.insert(e, make(1)));
        assert_eq!(drops.get(), 0);
        assert!(w.insert(e, make(2)));
        assert_eq!(drops.get(), 1, "replaced value dropped exactly once");
        w.despawn(e);
        assert_eq!(drops.get(), 2);
    }

    #[test]
    fn insert_on_dead_entity_fails_and_drops_the_value() {
        let mut w = World::new();
        let (drops, make) = counter();
        let e = w.spawn(());
        w.despawn(e);
        assert!(!w.insert(e, make(0)));
        assert_eq!(drops.get(), 1, "rejected value must not leak");
    }

    // ----------------------------------------------------------------- remove

    #[test]
    fn remove_returns_the_value_without_dropping_it() {
        let mut w = World::new();
        let (drops, make) = counter();
        let e = w.spawn((A(5),));
        w.insert(e, make(77));

        let taken = w.remove::<DropCounter>(e).expect("present");
        assert_eq!(drops.get(), 0, "removed value is moved out, not dropped");
        assert_eq!(taken.1, 77);
        drop(taken);
        assert_eq!(drops.get(), 1);

        assert!(!w.has::<DropCounter>(e));
        assert_eq!(
            w.get::<A>(e),
            Some(&A(5)),
            "other components survive the move"
        );
        assert!(w.is_alive(e));
    }

    #[test]
    fn remove_absent_or_dead_is_none() {
        let mut w = World::new();
        let e = w.spawn((A(1),));
        assert!(w.remove::<B>(e).is_none());
        assert_eq!(w.remove::<A>(e), Some(A(1)));
        assert!(w.remove::<A>(e).is_none(), "second remove finds nothing");
        w.despawn(e);
        assert!(w.remove::<A>(e).is_none());
    }

    #[test]
    fn remove_updates_the_swapped_entitys_slot() {
        let mut w = World::new();
        let entities: Vec<_> = (0..4u64).map(|i| w.spawn((A(i), B(i as u16)))).collect();
        assert_eq!(w.remove::<B>(entities[0]), Some(B(0)));
        for (i, e) in entities.iter().enumerate().skip(1) {
            assert_eq!(w.get::<A>(*e), Some(&A(i as u64)), "survivor {i} intact");
            assert_eq!(w.get::<B>(*e), Some(&B(i as u16)));
        }
        assert_eq!(w.get::<A>(entities[0]), Some(&A(0)));
    }

    #[test]
    fn insert_then_remove_round_trips_across_archetypes() {
        let mut w = World::new();
        let e = w.spawn((A(1),));
        for round in 0..10u16 {
            assert!(w.insert(e, B(round)));
            assert_eq!(w.get::<B>(e), Some(&B(round)));
            assert_eq!(w.remove::<B>(e), Some(B(round)));
            assert_eq!(w.get::<A>(e), Some(&A(1)), "A survives round {round}");
        }
        assert_eq!(w.archetypes.len(), 2, "churn reuses the two archetypes");
    }

    // ------------------------------------------------------------------- drop

    #[test]
    fn dropping_the_world_drops_every_live_component() {
        let (drops, make) = counter();
        {
            let mut w = World::new();
            for i in 0..50 {
                let e = w.spawn((A(i),));
                w.insert(e, make(i));
            }
            let dead = w.spawn(());
            w.insert(dead, make(99));
            w.despawn(dead);
            assert_eq!(drops.get(), 1);
        }
        assert_eq!(drops.get(), 51, "fifty live + one already dropped");
    }
}
