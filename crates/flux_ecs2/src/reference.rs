//! A naive, obviously-correct world used to verify the real implementation.
//!
//! Every operation is implemented the simplest possible way; differential
//! tests run random operation sequences against both worlds and require
//! identical observable state. Enabled by the `reference` feature; not part
//! of the crate's supported API.

use crate::{Component, Entities, Entity};
use std::any::{Any, TypeId};
use std::collections::HashMap;

#[derive(Default)]
/// The reference world. Mirrors [`World`](crate::World)'s API; entity ids
/// match the real world's for identical operation sequences.
pub struct RefWorld {
    entities: Entities,
    data: HashMap<Entity, HashMap<TypeId, Box<dyn Any>>>,
}

impl RefWorld {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn(&mut self) -> Entity {
        let entity = self.entities.alloc();
        self.data.insert(entity, HashMap::new());
        entity
    }

    pub fn despawn(&mut self, entity: Entity) -> bool {
        if !self.entities.dealloc(entity) { return false; }
        self.data.remove(&entity);
        true
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        self.entities.is_alive(entity)
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn insert<T: Component>(&mut self, entity: Entity, value: T) -> bool {
        match self.data.get_mut(&entity) {
            Some(bag) => {
                bag.insert(TypeId::of::<T>(), Box::new(value));
                true
            }
            None => false,
        }
    }

    pub fn remove<T: Component>(&mut self, entity: Entity) -> Option<T> {
        let boxed = self.data.get_mut(&entity)?.remove(&TypeId::of::<T>())?;
        Some(*boxed.downcast::<T>().expect("keyed by TypeId"))
    }

    pub fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        self.data
            .get(&entity)?
            .get(&TypeId::of::<T>())?
            .downcast_ref()
    }

    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        self.data
            .get_mut(&entity)?
            .get_mut(&TypeId::of::<T>())?
            .downcast_mut()
    }

    pub fn has<T: Component>(&self, entity: Entity) -> bool {
        self.get::<T>(entity).is_some()
    }

    pub fn entities(&self) -> Vec<Entity> {
        let mut entities = self.data.keys().copied().collect::<Vec<_>>();
        entities.sort();
        entities
    }

    pub fn entities_with<T: Component>(&self) -> Vec<Entity> {
        let mut entities = self.data.iter().filter_map(|(entity, bag)| {
            if bag.contains_key(&TypeId::of::<T>()) {
                Some(*entity)
            } else {
                None
            }
        }).collect::<Vec<_>>();
        entities.sort();
        entities
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Component, ComponentKey, Entity};
    use proptest::prelude::*;
    use std::cell::Cell;
    use std::collections::HashSet;
    use std::rc::Rc;

    // Manual impls: the derive's `::flux_ecs2::` paths do not resolve in-crate.
    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("reference::tests::", stringify!($name)));
            }
        };
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Pos(i32, i32);
    component!(Pos);

    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Vel(i32);
    component!(Vel);

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Marker;
    component!(Marker);

    /// Counts drops via a shared cell; a clone of the handle observes them.
    struct DropCounter(Rc<Cell<usize>>);
    component!(DropCounter);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    fn counter() -> (Rc<Cell<usize>>, impl Fn() -> DropCounter) {
        let cell = Rc::new(Cell::new(0));
        let c = Rc::clone(&cell);
        (cell, move || DropCounter(Rc::clone(&c)))
    }

    // -------------------------------------------------------------- lifecycle

    #[test]
    fn new_world_is_empty() {
        let w = RefWorld::new();
        assert_eq!(w.len(), 0);
        assert!(w.is_empty());
        assert!(w.entities().is_empty());
        let d = RefWorld::default();
        assert!(d.is_empty());
    }

    #[test]
    fn spawn_creates_live_entity_without_components() {
        let mut w = RefWorld::new();
        let e = w.spawn();
        assert!(w.is_alive(e));
        assert_eq!(w.len(), 1);
        assert!(!w.has::<Pos>(e));
        assert!(w.get::<Pos>(e).is_none());
        assert_eq!(w.entities(), vec![e]);
    }

    #[test]
    fn despawn_kills_and_returns_true_once() {
        let mut w = RefWorld::new();
        let e = w.spawn();
        assert!(w.despawn(e));
        assert!(!w.is_alive(e));
        assert_eq!(w.len(), 0);
        assert!(!w.despawn(e), "second despawn is a no-op");
        assert!(w.entities().is_empty());
    }

    #[test]
    fn despawn_only_affects_target() {
        let mut w = RefWorld::new();
        let a = w.spawn();
        let b = w.spawn();
        let c = w.spawn();
        w.insert(a, Pos(1, 1));
        w.insert(b, Pos(2, 2));
        w.insert(c, Pos(3, 3));
        assert!(w.despawn(b));
        assert_eq!(w.get::<Pos>(a), Some(&Pos(1, 1)));
        assert_eq!(w.get::<Pos>(c), Some(&Pos(3, 3)));
        assert_eq!(w.entities(), vec![a, c]);
    }

    // -------------------------------------------------------- entity identity

    #[test]
    fn entity_ids_follow_entities_semantics() {
        // The reference world must hand out the same ids as the real world
        // for the same op sequence, so both use the `Entities` allocator.
        let mut w = RefWorld::new();
        let mut reference = Entities::new();
        let a = w.spawn();
        let b = w.spawn();
        assert_eq!(a, reference.alloc());
        assert_eq!(b, reference.alloc());
        w.despawn(a);
        reference.dealloc(a);
        let c = w.spawn();
        assert_eq!(c, reference.alloc());
        assert_eq!(c.index(), a.index(), "index is recycled");
        assert_ne!(c, a, "generation differs");
    }

    #[test]
    fn stale_handle_cannot_see_or_touch_recycled_slot() {
        let mut w = RefWorld::new();
        let old = w.spawn();
        w.insert(old, Pos(1, 1));
        w.despawn(old);
        let new = w.spawn(); // same index, new generation
        w.insert(new, Vel(5));

        assert!(!w.is_alive(old));
        assert!(w.get::<Pos>(old).is_none());
        assert!(
            w.get::<Vel>(old).is_none(),
            "stale handle must not read the new occupant"
        );
        assert!(!w.has::<Vel>(old));
        assert!(
            !w.insert(old, Pos(9, 9)),
            "stale handle must not write to the new occupant"
        );
        assert!(
            w.remove::<Vel>(old).is_none(),
            "stale handle must not remove from the new occupant"
        );
        assert!(
            !w.despawn(old),
            "stale handle must not despawn the new occupant"
        );

        assert!(w.is_alive(new));
        assert_eq!(w.get::<Vel>(new), Some(&Vel(5)));
        assert!(
            !w.has::<Pos>(new),
            "old occupant's components must not leak into the new one"
        );
    }

    // ---------------------------------------------------------------- insert

    #[test]
    fn insert_then_get_roundtrips() {
        let mut w = RefWorld::new();
        let e = w.spawn();
        assert!(w.insert(e, Pos(3, 4)));
        assert!(w.has::<Pos>(e));
        assert_eq!(w.get::<Pos>(e), Some(&Pos(3, 4)));
        assert!(!w.has::<Vel>(e));
        assert!(w.get::<Vel>(e).is_none());
    }

    #[test]
    fn insert_on_dead_entity_fails_and_drops_value() {
        let mut w = RefWorld::new();
        let e = w.spawn();
        w.despawn(e);
        let (drops, make) = counter();
        assert!(!w.insert(e, make()));
        assert_eq!(drops.get(), 1, "rejected value must be dropped, not leaked");
        assert!(!w.insert(e, Pos(0, 0)));
    }

    #[test]
    fn insert_replaces_existing_component_and_drops_old_value() {
        let mut w = RefWorld::new();
        let e = w.spawn();
        assert!(w.insert(e, Pos(1, 1)));
        assert!(w.insert(e, Pos(2, 2)));
        assert_eq!(w.get::<Pos>(e), Some(&Pos(2, 2)));
        assert_eq!(w.entities_with::<Pos>(), vec![e], "still exactly one Pos");

        let (drops, make) = counter();
        w.insert(e, make());
        assert_eq!(drops.get(), 0);
        w.insert(e, make());
        assert_eq!(drops.get(), 1, "replaced value is dropped exactly once");
    }

    #[test]
    fn components_of_different_types_coexist() {
        let mut w = RefWorld::new();
        let e = w.spawn();
        w.insert(e, Pos(1, 2));
        w.insert(e, Vel(3));
        w.insert(e, Marker);
        assert_eq!(w.get::<Pos>(e), Some(&Pos(1, 2)));
        assert_eq!(w.get::<Vel>(e), Some(&Vel(3)));
        assert_eq!(w.get::<Marker>(e), Some(&Marker));
    }

    #[test]
    fn zero_sized_component_works() {
        let mut w = RefWorld::new();
        let a = w.spawn();
        let b = w.spawn();
        assert!(w.insert(a, Marker));
        assert!(w.has::<Marker>(a));
        assert!(!w.has::<Marker>(b));
        assert_eq!(w.entities_with::<Marker>(), vec![a]);
        assert_eq!(w.remove::<Marker>(a), Some(Marker));
        assert!(!w.has::<Marker>(a));
    }

    // ------------------------------------------------------------------- get

    #[test]
    fn get_mut_modifies_in_place() {
        let mut w = RefWorld::new();
        let e = w.spawn();
        w.insert(e, Pos(1, 1));
        w.get_mut::<Pos>(e).expect("present").0 = 10;
        assert_eq!(w.get::<Pos>(e), Some(&Pos(10, 1)));
        assert!(w.get_mut::<Vel>(e).is_none());
    }

    #[test]
    fn get_on_dead_entity_is_none() {
        let mut w = RefWorld::new();
        let e = w.spawn();
        w.insert(e, Pos(1, 1));
        w.despawn(e);
        assert!(w.get::<Pos>(e).is_none());
        assert!(w.get_mut::<Pos>(e).is_none());
        assert!(!w.has::<Pos>(e));
    }

    #[test]
    fn get_is_per_entity_not_per_type() {
        let mut w = RefWorld::new();
        let a = w.spawn();
        let b = w.spawn();
        w.insert(a, Pos(1, 1));
        w.insert(b, Pos(2, 2));
        assert_eq!(w.get::<Pos>(a), Some(&Pos(1, 1)));
        assert_eq!(w.get::<Pos>(b), Some(&Pos(2, 2)));
        w.get_mut::<Pos>(a).unwrap().0 = 99;
        assert_eq!(
            w.get::<Pos>(b),
            Some(&Pos(2, 2)),
            "mutating a must not affect b"
        );
    }

    // ---------------------------------------------------------------- remove

    #[test]
    fn remove_returns_value_and_clears_component() {
        let mut w = RefWorld::new();
        let e = w.spawn();
        w.insert(e, Pos(7, 8));
        w.insert(e, Vel(1));
        assert_eq!(w.remove::<Pos>(e), Some(Pos(7, 8)));
        assert!(!w.has::<Pos>(e));
        assert_eq!(w.get::<Vel>(e), Some(&Vel(1)), "other components untouched");
        assert!(w.is_alive(e), "removing a component does not despawn");
    }

    #[test]
    fn remove_absent_or_dead_is_none() {
        let mut w = RefWorld::new();
        let e = w.spawn();
        assert!(w.remove::<Pos>(e).is_none());
        w.insert(e, Pos(1, 1));
        assert!(w.remove::<Pos>(e).is_some());
        assert!(w.remove::<Pos>(e).is_none(), "second remove finds nothing");
        w.despawn(e);
        assert!(w.remove::<Pos>(e).is_none());
    }

    #[test]
    fn remove_transfers_ownership_without_dropping() {
        let mut w = RefWorld::new();
        let e = w.spawn();
        let (drops, make) = counter();
        w.insert(e, make());
        let taken = w.remove::<DropCounter>(e).expect("present");
        assert_eq!(drops.get(), 0, "removed value is moved out, not dropped");
        drop(taken);
        assert_eq!(drops.get(), 1);
    }

    // ------------------------------------------------------------------ drops

    #[test]
    fn despawn_drops_every_component_exactly_once() {
        let mut w = RefWorld::new();
        let e = w.spawn();
        let (drops, make) = counter();
        w.insert(e, make());
        w.insert(e, Pos(0, 0));
        assert_eq!(drops.get(), 0);
        w.despawn(e);
        assert_eq!(drops.get(), 1);
        w.despawn(e);
        assert_eq!(drops.get(), 1, "no double drop on repeated despawn");
    }

    #[test]
    fn dropping_the_world_drops_all_components() {
        let (drops, make) = counter();
        {
            let mut w = RefWorld::new();
            for _ in 0..5 {
                let e = w.spawn();
                w.insert(e, make());
            }
            let dead = w.spawn();
            w.insert(dead, make());
            w.despawn(dead);
            assert_eq!(drops.get(), 1);
        }
        assert_eq!(drops.get(), 6, "five live + one already dropped");
    }

    // ------------------------------------------------------------ enumeration

    #[test]
    fn entities_is_sorted_and_only_live() {
        let mut w = RefWorld::new();
        let all: Vec<Entity> = (0..10).map(|_| w.spawn()).collect();
        w.despawn(all[3]);
        w.despawn(all[7]);
        let recycled = w.spawn(); // reuses one of the freed indices, higher generation
        let got = w.entities();
        let mut expected: Vec<Entity> = all
            .iter()
            .copied()
            .filter(|e| *e != all[3] && *e != all[7])
            .chain(std::iter::once(recycled))
            .collect();
        expected.sort();
        assert_eq!(got, expected);
        assert!(got.windows(2).all(|p| p[0] < p[1]), "strictly ascending");
        assert_eq!(got.len(), w.len());
    }

    #[test]
    fn entities_with_filters_by_component_and_is_sorted() {
        let mut w = RefWorld::new();
        let a = w.spawn();
        let b = w.spawn();
        let c = w.spawn();
        let d = w.spawn();
        w.insert(a, Pos(0, 0));
        w.insert(c, Pos(0, 0));
        w.insert(c, Vel(0));
        w.insert(d, Vel(0));
        assert_eq!(w.entities_with::<Pos>(), vec![a, c]);
        assert_eq!(w.entities_with::<Vel>(), vec![c, d]);
        assert!(w.entities_with::<Marker>().is_empty());
        let _ = b;

        w.remove::<Pos>(c);
        assert_eq!(w.entities_with::<Pos>(), vec![a]);
        w.despawn(d);
        assert_eq!(w.entities_with::<Vel>(), vec![c]);
    }

    // ------------------------------------------------------------- invariants

    #[derive(Debug, Clone)]
    enum Op {
        Spawn,
        Despawn(usize),
        InsertPos(usize, i32),
        InsertVel(usize, i32),
        RemovePos(usize),
        RemoveVel(usize),
    }

    fn op_strategy() -> impl Strategy<Value=Op> {
        prop_oneof![
            3 => Just(Op::Spawn),
            2 => (0usize..64).prop_map(Op::Despawn),
            3 => (0usize..64, any::<i32>()).prop_map(|(i, v)| Op::InsertPos(i, v)),
            3 => (0usize..64, any::<i32>()).prop_map(|(i, v)| Op::InsertVel(i, v)),
            2 => (0usize..64).prop_map(Op::RemovePos),
            2 => (0usize..64).prop_map(Op::RemoveVel),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        #[test]
        #[cfg_attr(miri, ignore = "proptest is too slow under miri; covered by deterministic tests")]
        fn random_ops_keep_internal_invariants(ops in prop::collection::vec(op_strategy(), 1..200)) {
            // Shadow model: only liveness and per-entity Pos/Vel presence.
            let mut w = RefWorld::new();
            let mut log: Vec<Entity> = Vec::new();
            let mut live: HashSet<Entity> = HashSet::new();
            let mut pos: HashMap<Entity, i32> = Default::default();
            let mut vel: HashMap<Entity, i32> = Default::default();

            for op in ops {
                let pick = |i: usize, log: &Vec<Entity>| log.get(i % log.len().max(1)).copied();
                match op {
                    Op::Spawn => { let e = w.spawn(); live.insert(e); log.push(e); }
                    Op::Despawn(i) => if let Some(e) = pick(i, &log) {
                        let was_live = live.remove(&e);
                        prop_assert_eq!(w.despawn(e), was_live);
                        pos.remove(&e); vel.remove(&e);
                    },
                    Op::InsertPos(i, v) => if let Some(e) = pick(i, &log) {
                        let ok = w.insert(e, Pos(v, v));
                        prop_assert_eq!(ok, live.contains(&e));
                        if ok { pos.insert(e, v); }
                    },
                    Op::InsertVel(i, v) => if let Some(e) = pick(i, &log) {
                        let ok = w.insert(e, Vel(v));
                        prop_assert_eq!(ok, live.contains(&e));
                        if ok { vel.insert(e, v); }
                    },
                    Op::RemovePos(i) => if let Some(e) = pick(i, &log) {
                        prop_assert_eq!(w.remove::<Pos>(e).map(|p| p.0), pos.remove(&e));
                    },
                    Op::RemoveVel(i) => if let Some(e) = pick(i, &log) {
                        prop_assert_eq!(w.remove::<Vel>(e).map(|v| v.0), vel.remove(&e));
                    },
                }

                prop_assert_eq!(w.len(), live.len());
                let ents = w.entities();
                prop_assert!(ents.windows(2).all(|p| p[0] < p[1]), "entities() sorted");
                prop_assert_eq!(ents.iter().copied().collect::<HashSet<_>>(), live.clone());
                for e in &log {
                    prop_assert_eq!(w.is_alive(*e), live.contains(e));
                    prop_assert_eq!(w.get::<Pos>(*e).map(|p| p.0), pos.get(e).copied());
                    prop_assert_eq!(w.get::<Vel>(*e).map(|v| v.0), vel.get(e).copied());
                }
                let mut with_pos: Vec<Entity> = pos.keys().copied().collect();
                with_pos.sort();
                prop_assert_eq!(w.entities_with::<Pos>(), with_pos);
            }
        }
    }
}
