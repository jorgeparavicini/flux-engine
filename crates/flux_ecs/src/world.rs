use crate::grant::AccessGrant;
use crate::query::data::QueryData;
use crate::query::filter::QueryFilter;
use crate::registry::{ComponentId, KeyMap, Registry};
use crate::storage::alloc::ChunkAlloc;
use crate::storage::archetype::{ArchetypeId, Archetypes};
use crate::storage::chunks::{ChunkId, Chunks};
use crate::storage::ops;
use crate::{Bundle, Component, Entities, Entity, Query, QueryState};

/// A reactive callback run during a structural change.
///
/// Registered through [`Component::ON_ADD`] / [`Component::ON_REMOVE`], it
/// receives the world and the entities whose component set just changed, one
/// slice per structural change rather than one call per entity.
pub type Hook = fn(&mut World, &[Entity]);

/// A collection of entities and their components.
///
/// ```
/// use flux_ecs::{Component, World};
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
#[derive(Copy, Clone)]
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

        for &id in &signature {
            self.fire_on_add(id, &[entity]);
        }
    }

    /// Runs `id`'s add hook, if any, over `entities`.
    fn fire_on_add(&mut self, id: ComponentId, entities: &[Entity]) {
        let hook = self.registry.info(id).on_add;
        if let Some(hook) = hook {
            hook(self, entities);
        }
    }

    /// Runs `id`'s remove hook, if any, over `entities`.
    fn fire_on_remove(&mut self, id: ComponentId, entities: &[Entity]) {
        let hook = self.registry.info(id).on_remove;
        if let Some(hook) = hook {
            hook(self, entities);
        }
    }

    /// Despawns `entity`, dropping all of its components.
    ///
    /// Returns false on a dead or stale handle, leaving the world unchanged.
    pub fn despawn(&mut self, entity: Entity) -> bool {
        let Some(slot) = self.entities.slot(entity) else {
            return false;
        };
        let arch_id = self.chunks.archetype(ChunkId(slot.chunk));
        let signature: Vec<ComponentId> = self.archetypes.get(arch_id).signature().to_vec();
        if signature
            .iter()
            .any(|&id| self.registry.info(id).on_remove.is_some())
        {
            for &id in &signature {
                self.fire_on_remove(id, &[entity]);
            }
        }
        // A hook may have moved or already despawned the entity; re-resolve.
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

    /// Enables or disables `entity`'s toggleable `T` without an archetype
    /// move. A disabled component is skipped by iteration but still occupies
    /// its column; toggling bumps the chunk's change-detection version.
    ///
    /// Returns false if the entity is dead or lacks `T`.
    pub fn set_enabled<T: Component>(&mut self, entity: Entity, value: bool) -> bool {
        debug_assert!(T::TOGGLEABLE, "set_enabled requires a #[component(toggleable)] type");
        let Some((chunk, row, column, arch_id)) = self.locate::<T>(entity) else {
            return false;
        };
        let capacity = self.archetypes.get(arch_id).layout.capacity;
        self.chunks.set_enabled(chunk, column, row, capacity, value);
        self.version += 1;
        self.chunks.stamp_write_version(chunk, column, self.version);
        true
    }

    /// Enables `entity`'s toggleable `T`. See [`set_enabled`](Self::set_enabled).
    pub fn enable<T: Component>(&mut self, entity: Entity) -> bool {
        self.set_enabled::<T>(entity, true)
    }

    /// Disables `entity`'s toggleable `T`. See [`set_enabled`](Self::set_enabled).
    pub fn disable<T: Component>(&mut self, entity: Entity) -> bool {
        self.set_enabled::<T>(entity, false)
    }

    /// Whether `entity` has an enabled `T`. False if dead, absent, or disabled.
    pub fn is_enabled<T: Component>(&self, entity: Entity) -> bool {
        match self.locate::<T>(entity) {
            Some((chunk, row, column, _)) => self.chunks.is_enabled(chunk, column, row),
            None => false,
        }
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
        self.fire_on_add(id, &[entity]);
        true
    }

    /// Adds `T` to many entities at once, grouped by source archetype so the
    /// archetype transition is resolved once per group rather than per entity.
    ///
    /// Entities that are dead, or already have `T`, fall back to the
    /// per-entity path. Equivalent in effect to calling [`insert`](Self::insert)
    /// on each item.
    pub fn insert_batch<T: Component>(&mut self, items: impl IntoIterator<Item = (Entity, T)>) {
        let id = self.registry.register::<T>();
        let mut groups: std::collections::HashMap<ArchetypeId, Vec<(Entity, T)>> =
            std::collections::HashMap::new();
        let mut replaces: Vec<(Entity, T)> = Vec::new();
        for (entity, value) in items {
            let Some(slot) = self.entities.slot(entity) else {
                continue; // dead: value dropped
            };
            let src = self.chunks.archetype(ChunkId(slot.chunk));
            if self.archetypes.get(src).signature().binary_search(&id).is_ok() {
                replaces.push((entity, value));
            } else {
                groups.entry(src).or_default().push((entity, value));
            }
        }
        for (entity, value) in replaces {
            self.insert(entity, value);
        }
        if groups.is_empty() {
            return;
        }
        self.version += 1;
        let version = self.version;
        // Deterministic group order (archetype id) for stable results.
        let mut group_keys: Vec<ArchetypeId> = groups.keys().copied().collect();
        group_keys.sort();
        // Entities that gain `T` through the bulk path; hooked once at the end
        // so the add hook sees a single slice rather than one call per entity.
        let mut added: Vec<Entity> = Vec::new();
        for src_id in group_keys {
            let mut items = groups.remove(&src_id).expect("key present");
            // Collapse duplicate entities, keeping the last value, so the
            // coverage count below reflects distinct entities rather than
            // insertions. A superseded value is dropped here, exactly as a
            // repeated per-entity insert would drop it.
            {
                let mut seen: std::collections::HashMap<Entity, usize> =
                    std::collections::HashMap::with_capacity(items.len());
                let mut deduped: Vec<(Entity, T)> = Vec::with_capacity(items.len());
                for (entity, value) in items {
                    match seen.get(&entity) {
                        Some(&idx) => deduped[idx] = (entity, value),
                        None => {
                            seen.insert(entity, deduped.len());
                            deduped.push((entity, value));
                        }
                    }
                }
                items = deduped;
            }
            let src_chunks = self.archetypes.get(src_id).chunks.clone();
            let arch_total: usize = src_chunks
                .iter()
                .map(|&c| self.chunks.len(c) as usize)
                .sum();
            let dst_id = self.add_edge_target(src_id, id);

            if items.len() != arch_total {
                // Partial coverage: per-entity fallback.
                for (entity, value) in items {
                    self.insert(entity, value);
                }
                continue;
            }

            // Whole-archetype coverage: bulk range-copy chunk by chunk, then
            // destroy the source chunks (no per-entity swap_remove).
            //
            // Values are placed in a Vec keyed by entity index (dense, no
            // hashing) so lookup during the move is O(1).
            let capacity = items
                .iter()
                .map(|(e, _)| e.index() as usize + 1)
                .max()
                .unwrap_or(0);
            let mut values: Vec<Option<T>> = (0..capacity).map(|_| None).collect();
            for (entity, value) in items {
                values[entity.index() as usize] = Some(value);
            }
            let entities = &mut self.entities;
            let chunks = &mut self.chunks;
            let alloc = &mut self.alloc;
            let registry = &self.registry;
            let (src_arch, dst_arch) = self.archetypes.get_pair_mut(src_id, dst_id);
            let dst_column = dst_arch
                .signature()
                .binary_search(&id)
                .expect("inserted component is in the target signature");
            let dst_columns = dst_arch.signature().len();
            let mut touched: std::collections::HashSet<ChunkId> = std::collections::HashSet::new();

            for &src_chunk in &src_chunks {
                let len = chunks.len(src_chunk) as usize;
                let mut out: Vec<(ChunkId, u16)> = Vec::with_capacity(len);
                unsafe {
                    ops::move_full_chunk(
                        &src_arch.layout, dst_arch, dst_id, chunks, alloc, registry, src_chunk, &mut out,
                    );
                }
                for &(dst_chunk, dst_row) in &out {
                    let entity = unsafe { ops::entity_at(chunks, &dst_arch.layout, dst_chunk, dst_row) };
                    let value = values[entity.index() as usize].take().expect("entity is in the batch");
                    unsafe {
                        ops::component_ptr(chunks, &dst_arch.layout, registry, dst_chunk, dst_column, dst_row)
                            .cast::<T>()
                            .write(value);
                    }
                    let slot = entities.slot_mut(entity).expect("checked live");
                    slot.chunk = dst_chunk.0;
                    slot.row = dst_row;
                    touched.insert(dst_chunk);
                    added.push(entity);
                }
            }
            // Retire the emptied source chunks.
            for &src_chunk in &src_chunks {
                chunks.set_len(src_chunk, 0);
                chunks.destroy(alloc, src_chunk);
            }
            src_arch.chunks.clear();
            src_arch.non_full = None;
            // Stamp each touched destination chunk once.
            for chunk in touched {
                for column in 0..dst_columns {
                    chunks.stamp_write_version(chunk, column, version);
                    chunks.stamp_added_version(chunk, column, version);
                }
            }
        }
        if !added.is_empty() {
            self.fire_on_add(id, &added);
        }
    }

    /// Takes `T` off `entity` and returns it.
    ///
    /// None if the entity is dead or does not have the component.
    pub fn remove<T: Component>(&mut self, entity: Entity) -> Option<T> {
        let mut located = self.locate::<T>(entity)?;
        let id = self.registry.lookup(T::KEY).expect("located above");
        if self.registry.info(id).on_remove.is_some() {
            self.fire_on_remove(id, &[entity]);
            // The hook may have moved or already removed the component.
            located = self.locate::<T>(entity)?;
        }
        let (chunk, row, column, arch_id) = located;
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
    /// use flux_ecs::{Component, QueryState, World};
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
    /// use flux_ecs::{Component, QueryState, World};
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
    /// use flux_ecs::{Component, QueryState, World};
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
    use std::cell::{Cell, RefCell};
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
    struct C(u64);
    component!(C);
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
    fn insert_batch_matches_per_entity_insert() {
        // Two worlds, same spawns; one uses insert_batch, one the per-entity
        // loop. Every entity must end identical.
        let entities: Vec<u64> = (0..2000).collect();
        let mut batched = World::new();
        let mut per_entity = World::new();
        let mut b_ids = Vec::new();
        let mut p_ids = Vec::new();
        for &i in &entities {
            b_ids.push(batched.spawn((A(i),)));
            p_ids.push(per_entity.spawn((A(i),)));
        }
        batched.insert_batch(b_ids.iter().map(|&e| (e, B(7))));
        for &e in &p_ids {
            per_entity.insert(e, B(7));
        }
        for i in 0..entities.len() {
            assert_eq!(batched.get::<A>(b_ids[i]), Some(&A(entities[i])));
            assert_eq!(batched.get::<B>(b_ids[i]), Some(&B(7)));
            assert_eq!(batched.get::<A>(b_ids[i]), per_entity.get::<A>(p_ids[i]));
        }
        assert_eq!(batched.len(), per_entity.len());
    }

    #[test]
    fn insert_batch_groups_multiple_source_archetypes() {
        let mut w = World::new();
        let a_only: Vec<_> = (0..10).map(|i| w.spawn((A(i),))).collect();
        let ab: Vec<_> = (0..10).map(|i| w.spawn((A(i), B(0)))).collect();
        // insert C into both groups at once: {A}->{A,C} and {A,B}->{A,B,C}
        let all: Vec<_> = a_only.iter().chain(ab.iter()).map(|&e| (e, C(1))).collect();
        w.insert_batch(all);
        for &e in &a_only {
            assert!(w.has::<C>(e) && w.has::<A>(e) && !w.has::<B>(e));
        }
        for &e in &ab {
            assert!(w.has::<C>(e) && w.has::<A>(e) && w.has::<B>(e));
        }
    }

    #[test]
    fn insert_batch_handles_dead_and_replace() {
        let mut w = World::new();
        let live = w.spawn((A(1),));
        let has_b = w.spawn((A(2), B(0)));
        let dead = w.spawn((A(3),));
        w.despawn(dead);
        w.insert_batch([(live, B(9)), (has_b, B(9)), (dead, B(9))]);
        assert_eq!(w.get::<B>(live), Some(&B(9)), "added");
        assert_eq!(w.get::<B>(has_b), Some(&B(9)), "replaced in place");
        assert!(!w.is_alive(dead));
        assert_eq!(w.len(), 2);
    }

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

    // ---------------------------------------------------------- toggleable

    #[derive(Copy, Clone, PartialEq, Debug)]
    struct Toggle;
    impl Component for Toggle {
        const KEY: ComponentKey = ComponentKey::from_path("world::tests::Toggle");
        const STORAGE: StorageClass = StorageClass::Tag;
        const TOGGLEABLE: bool = true;
    }

    #[test]
    fn toggling_moves_no_entity() {
        let mut world = World::new();
        // Several chunks of a single archetype.
        let entities: Vec<Entity> = (0..5_000u64).map(|i| world.spawn((A(i), Toggle))).collect();
        let before: Vec<_> = entities.iter().map(|&e| world.locate::<A>(e)).collect();
        let archetypes = world.archetypes.len();

        for &e in &entities {
            assert!(world.disable::<Toggle>(e));
        }

        assert_eq!(world.archetypes.len(), archetypes, "no new archetype");
        for (&e, loc) in entities.iter().zip(&before) {
            assert_eq!(world.locate::<A>(e), *loc, "entity stayed in place");
            assert!(!world.is_enabled::<Toggle>(e));
        }
    }

    #[test]
    fn for_each_skips_disabled_rows() {
        let mut world = World::new();
        let entities: Vec<Entity> = (0..200u64).map(|i| world.spawn((A(i), Toggle))).collect();
        for (i, &e) in entities.iter().enumerate() {
            if i % 2 == 0 {
                world.disable::<Toggle>(e);
            }
        }

        let mut state = QueryState::<(&A, &Toggle)>::new();
        let mut seen: Vec<u64> = Vec::new();
        world.query(&mut state).for_each(|(a, _)| seen.push(a.0));

        let expected: Vec<u64> = (0..200u64).filter(|i| i % 2 == 1).collect();
        seen.sort_unstable();
        assert_eq!(seen, expected, "only enabled rows are visited");
    }

    #[test]
    fn re_enabling_restores_visibility() {
        let mut world = World::new();
        let e = world.spawn((A(7), Toggle));
        world.disable::<Toggle>(e);

        let mut state = QueryState::<(&A, &Toggle)>::new();
        let mut count = |world: &mut World| {
            let mut n = 0;
            world.query(&mut state).for_each(|_| n += 1);
            n
        };
        assert_eq!(count(&mut world), 0);
        world.enable::<Toggle>(e);
        assert_eq!(count(&mut world), 1);
    }

    #[test]
    fn disabled_state_survives_a_structural_move() {
        let mut world = World::new();
        let e = world.spawn((A(1), Toggle));
        world.disable::<Toggle>(e);
        // Growing the signature relocates the entity to a new archetype.
        world.insert(e, B(2));
        assert!(!world.is_enabled::<Toggle>(e), "still disabled after the move");
    }

    #[test]
    fn batched_insert_preserves_disabled_state() {
        let mut world = World::new();
        let entities: Vec<Entity> = (0..300u64).map(|i| world.spawn((A(i), Toggle))).collect();
        for (i, &e) in entities.iter().enumerate() {
            if i % 3 == 0 {
                world.disable::<Toggle>(e);
            }
        }
        let batch: Vec<(Entity, B)> = entities.iter().map(|&e| (e, B(0))).collect();
        world.insert_batch(batch);

        for (i, &e) in entities.iter().enumerate() {
            assert_eq!(
                world.is_enabled::<Toggle>(e),
                i % 3 != 0,
                "entity {i} kept its enabled state through the batch insert"
            );
        }
    }

    #[test]
    fn insert_batch_with_duplicate_entities_keeps_the_last_value() {
        let mut world = World::new();
        let a = world.spawn((A(1),));
        let b = world.spawn((A(2),));
        // `a` appears twice; the whole archetype is still covered.
        world.insert_batch(vec![(a, B(10)), (b, B(20)), (a, B(11))]);
        assert_eq!(world.get::<B>(a), Some(&B(11)));
        assert_eq!(world.get::<B>(b), Some(&B(20)));
    }

    #[test]
    fn insert_batch_drops_each_superseded_value_once() {
        let (drops, make) = counter();
        let mut world = World::new();
        let e = world.spawn((A(1),));
        world.insert_batch(vec![(e, make(0)), (e, make(1)), (e, make(2))]);
        // Two superseded values dropped; the survivor is still stored.
        assert_eq!(drops.get(), 2);
        assert_eq!(world.get::<DropCounter>(e).map(|d| d.1), Some(2));
    }

    // --------------------------------------------------------------- hooks

    thread_local! {
        static ADDED: RefCell<Vec<Vec<Entity>>> = const { RefCell::new(Vec::new()) };
        static REMOVED: RefCell<Vec<Vec<Entity>>> = const { RefCell::new(Vec::new()) };
        static REMOVE_SAW: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    }

    fn reset_hook_log() {
        ADDED.with(|l| l.borrow_mut().clear());
        REMOVED.with(|l| l.borrow_mut().clear());
        REMOVE_SAW.with(|l| l.borrow_mut().clear());
    }

    #[derive(Copy, Clone, PartialEq, Debug)]
    struct Tracked(u32);
    impl Component for Tracked {
        const KEY: ComponentKey = ComponentKey::from_path("world::tests::Tracked");
        const ON_ADD: Option<Hook> =
            Some(|_w, es| ADDED.with(|l| l.borrow_mut().push(es.to_vec())));
        const ON_REMOVE: Option<Hook> = Some(|w, es| {
            REMOVED.with(|l| l.borrow_mut().push(es.to_vec()));
            for &e in es {
                if let Some(t) = w.get::<Tracked>(e) {
                    REMOVE_SAW.with(|v| v.borrow_mut().push(t.0));
                }
            }
        });
    }

    #[derive(Copy, Clone)]
    struct AutoMark;
    impl Component for AutoMark {
        const KEY: ComponentKey = ComponentKey::from_path("world::tests::AutoMark");
        const STORAGE: StorageClass = StorageClass::Tag;
        const ON_ADD: Option<Hook> = Some(|w, es| {
            for &e in es {
                w.insert(e, B(9));
            }
        });
    }

    #[test]
    fn hooks_fire_on_spawn_and_remove() {
        reset_hook_log();
        let mut world = World::new();
        let e = world.spawn((Tracked(7),));
        assert_eq!(ADDED.with(|l| l.borrow().clone()), vec![vec![e]]);

        assert_eq!(world.remove::<Tracked>(e), Some(Tracked(7)));
        assert_eq!(REMOVED.with(|l| l.borrow().clone()), vec![vec![e]]);
        assert_eq!(
            REMOVE_SAW.with(|l| l.borrow().clone()),
            vec![7],
            "the remove hook saw the live value before removal"
        );
    }

    #[test]
    fn insert_fires_on_add() {
        reset_hook_log();
        let mut world = World::new();
        let e = world.spawn((A(1),));
        world.insert(e, Tracked(3));
        assert_eq!(ADDED.with(|l| l.borrow().clone()), vec![vec![e]]);
    }

    #[test]
    fn insert_batch_fires_one_add_slice() {
        reset_hook_log();
        let mut world = World::new();
        let entities: Vec<Entity> = (0..100u64).map(|i| world.spawn((A(i),))).collect();
        let batch: Vec<(Entity, Tracked)> = entities.iter().map(|&e| (e, Tracked(0))).collect();
        world.insert_batch(batch);

        let added = ADDED.with(|l| l.borrow().clone());
        assert_eq!(added.len(), 1, "one slice for the whole batch, not one per entity");
        let mut got = added[0].clone();
        got.sort_by_key(Entity::index);
        let mut want = entities.clone();
        want.sort_by_key(Entity::index);
        assert_eq!(got, want);
    }

    #[test]
    fn on_add_hook_can_mutate_the_world() {
        let mut world = World::new();
        let e = world.spawn((A(1), AutoMark));
        assert_eq!(world.get::<B>(e), Some(&B(9)), "the add hook inserted B");
    }

    #[test]
    fn despawn_fires_on_remove() {
        reset_hook_log();
        let mut world = World::new();
        let e = world.spawn((Tracked(5), A(1)));
        assert!(world.despawn(e));
        assert_eq!(REMOVED.with(|l| l.borrow().clone()), vec![vec![e]]);
        assert_eq!(REMOVE_SAW.with(|l| l.borrow().clone()), vec![5]);
    }
}
