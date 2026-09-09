use std::num::NonZeroU32;

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Entity {
    index: u32,
    generation: NonZeroU32,
}

impl Entity {
    pub(crate) const fn index(&self) -> u32 {
        self.index
    }
}

pub(crate) struct EntitySlot {
    pub generation: NonZeroU32,
    pub chunk: u32,
    pub row: u16,
    pub flags: u16,
}

#[derive(Default)]
pub struct Entities {
    slots: Vec<EntitySlot>,
    free: Vec<u32>,
}

impl Entities {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc(&mut self) -> Entity {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            debug_assert!(slot.generation.get().is_multiple_of(2), "free-list slot must be dead (even)");
            slot.generation = Self::next_generation(slot.generation);
            return Entity {
                index,
                generation: slot.generation,
            };
        }

        let generation = NonZeroU32::MIN;
        let slot = EntitySlot {
            generation,
            chunk: 0,
            row: 0,
            flags: 0,
        };
        let index = u32::try_from(self.slots.len()).expect("entity index space exhausted");
        self.slots.push(slot);
        Entity { index, generation }
    }

    pub fn dealloc(&mut self, entity: Entity) -> bool {
        if let Some(slot) = self.slots.get_mut(entity.index as usize)
            && entity.generation == slot.generation
        {
            debug_assert!(!slot.generation.get().is_multiple_of(2), "live slot must have odd generation");
            slot.generation = Self::next_generation(slot.generation);
            slot.chunk = 0;
            slot.row = 0;
            slot.flags = 0;
            self.free.push(entity.index);

            true
        } else {
            false
        }
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        self.slot(entity).is_some()
    }

    pub(crate) fn slot(&self, entity: Entity) -> Option<&EntitySlot> {
        self.slots
            .get(entity.index as usize)
            .filter(|slot| slot.generation == entity.generation)
    }

    pub(crate) fn slot_mut(&mut self, entity: Entity) -> Option<&mut EntitySlot> {
        self.slots
            .get_mut(entity.index as usize)
            .filter(|slot| slot.generation == entity.generation)
    }
    
    pub(crate) fn live_count(&self) -> usize {
        self.slots.len() - self.free.len()
    }

    const fn next_generation(generation: NonZeroU32) -> NonZeroU32 {
        match generation.checked_add(1) {
            Some(generation) => generation,
            None => NonZeroU32::new(2).unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashSet;
    use std::mem::size_of;

    fn nz(g: u32) -> NonZeroU32 {
        NonZeroU32::new(g).expect("test generation must be non-zero")
    }

    /// Build a handle by hand — only possible inside the module. Used to
    /// probe stale / out-of-range handles the public API can't construct.
    fn handle(index: u32, generation: u32) -> Entity {
        Entity {
            index,
            generation: nz(generation),
        }
    }

    // ---------------------------------------------------------------- layout

    #[test]
    fn entity_is_8_bytes_and_option_entity_has_a_niche() {
        assert_eq!(size_of::<Entity>(), 8);
        assert_eq!(size_of::<Option<Entity>>(), 8);
    }

    #[test]
    fn slot_is_12_bytes() {
        assert_eq!(size_of::<EntitySlot>(), 12);
    }

    // -------------------------------------------------------- basic lifecycle

    #[test]
    fn fresh_world_has_nothing_alive() {
        let e = Entities::new();
        assert!(!e.is_alive(handle(0, 1)));
        assert!(e.slot(handle(0, 1)).is_none());
    }

    #[test]
    fn alloc_returns_alive_entity_with_odd_generation() {
        let mut e = Entities::new();
        let a = e.alloc();
        assert!(e.is_alive(a));
        assert_eq!(a.generation.get() % 2, 1, "alive generations are odd");
        assert_eq!(
            a.generation.get(),
            1,
            "first generation of a fresh slot is 1"
        );
    }

    #[test]
    fn alloc_yields_distinct_entities() {
        let mut e = Entities::new();
        let n = 10_000;
        let all: Vec<Entity> = (0..n).map(|_| e.alloc()).collect();
        let indices: HashSet<u32> = all.iter().map(|x| x.index).collect();
        assert_eq!(indices.len(), n, "no two live entities share an index");
        assert!(all.iter().all(|x| e.is_alive(*x)));
        assert!(all.iter().all(|x| x.generation.get() % 2 == 1));
    }

    #[test]
    fn dealloc_kills_and_returns_true() {
        let mut e = Entities::new();
        let a = e.alloc();
        assert!(e.dealloc(a));
        assert!(!e.is_alive(a));
        assert!(e.slot(a).is_none());
        assert!(e.slot_mut(a).is_none());
    }

    #[test]
    fn dealloc_flips_slot_generation_to_even() {
        let mut e = Entities::new();
        let a = e.alloc();
        e.dealloc(a);
        let g = e.slots[a.index as usize].generation.get();
        assert_eq!(g % 2, 0, "dead slots have even generation");
        assert_eq!(
            g,
            a.generation.get() + 1,
            "despawn increments by exactly one"
        );
    }

    #[test]
    fn dealloc_only_affects_its_own_slot() {
        let mut e = Entities::new();
        let a = e.alloc();
        let b = e.alloc();
        let c = e.alloc();
        assert!(e.dealloc(b));
        assert!(e.is_alive(a));
        assert!(!e.is_alive(b));
        assert!(e.is_alive(c));
    }

    // ------------------------------------------------------ stale / bad handles

    #[test]
    fn double_dealloc_returns_false() {
        let mut e = Entities::new();
        let a = e.alloc();
        assert!(e.dealloc(a));
        assert!(
            !e.dealloc(a),
            "second dealloc of the same handle is a no-op"
        );
    }

    #[test]
    fn double_dealloc_does_not_duplicate_free_list_entry() {
        // If a double dealloc pushed the index twice, two later allocs would
        // hand out the SAME index to two live entities — catastrophic aliasing.
        let mut e = Entities::new();
        let a = e.alloc();
        e.dealloc(a);
        e.dealloc(a);
        let x = e.alloc();
        let y = e.alloc();
        assert_ne!(
            x.index, y.index,
            "free list must not contain duplicate indices"
        );
        assert!(e.is_alive(x));
        assert!(e.is_alive(y));
    }

    #[test]
    fn stale_handle_is_dead_after_index_is_recycled() {
        let mut e = Entities::new();
        let old = e.alloc();
        e.dealloc(old);
        let new = e.alloc();
        assert_eq!(new.index, old.index, "single freed index is recycled");
        assert_ne!(new, old, "recycled handle differs by generation");
        assert!(e.is_alive(new));
        assert!(!e.is_alive(old), "stale handle must not alias the live one");
        assert!(e.slot(old).is_none());
        assert!(
            !e.dealloc(old),
            "stale handle cannot despawn the new occupant"
        );
        assert!(
            e.is_alive(new),
            "new occupant survived the stale dealloc attempt"
        );
    }

    #[test]
    fn recycled_generation_is_old_plus_two() {
        let mut e = Entities::new();
        let old = e.alloc();
        e.dealloc(old);
        let new = e.alloc();
        assert_eq!(new.generation.get(), old.generation.get() + 2);
        assert_eq!(new.generation.get() % 2, 1);
    }

    #[test]
    fn handle_with_wrong_generation_on_live_slot_is_dead() {
        let mut e = Entities::new();
        let a = e.alloc(); // gen 1
        let forged_future = handle(a.index, 3);
        let forged_even = handle(a.index, 2);
        assert!(!e.is_alive(forged_future));
        assert!(!e.is_alive(forged_even));
        assert!(e.slot(forged_future).is_none());
        assert!(!e.dealloc(forged_future));
        assert!(
            e.is_alive(a),
            "forged handles must not disturb the live entity"
        );
    }

    #[test]
    fn out_of_range_index_is_dead_not_a_panic() {
        let mut e = Entities::new();
        let _ = e.alloc();
        let far = handle(u32::MAX, 1);
        let just_past = handle(1, 1);
        assert!(!e.is_alive(far));
        assert!(!e.is_alive(just_past));
        assert!(e.slot(far).is_none());
        assert!(e.slot_mut(just_past).is_none());
        assert!(!e.dealloc(far));
        assert!(!e.dealloc(just_past));
    }

    // -------------------------------------------------------------- free list

    #[test]
    fn freed_indices_are_all_reused_before_growing() {
        let mut e = Entities::new();
        let all: Vec<Entity> = (0..8).map(|_| e.alloc()).collect();
        let freed: HashSet<u32> = [1u32, 3, 5, 6].into_iter().collect();
        for x in &all {
            if freed.contains(&x.index) {
                assert!(e.dealloc(*x));
            }
        }
        let reused: HashSet<u32> = (0..freed.len()).map(|_| e.alloc().index).collect();
        assert_eq!(
            reused, freed,
            "every freed index is recycled before any new one"
        );
        let fresh = e.alloc();
        assert_eq!(
            fresh.index, 8,
            "only after the free list is drained does the world grow"
        );
    }

    #[test]
    fn slot_count_does_not_grow_while_recycling() {
        let mut e = Entities::new();
        for _ in 0..1_000 {
            let x = e.alloc();
            e.dealloc(x);
        }
        assert_eq!(
            e.slots.len(),
            1,
            "alloc/dealloc churn on one slot must not grow storage"
        );
    }

    // ----------------------------------------------------------- slot access

    #[test]
    fn slot_mut_writes_are_visible_through_slot() {
        let mut e = Entities::new();
        let a = e.alloc();
        {
            let s = e.slot_mut(a).expect("live entity has a slot");
            s.chunk = 42;
            s.row = 7;
            s.flags = 0b1010;
        }
        let s = e.slot(a).expect("live entity has a slot");
        assert_eq!((s.chunk, s.row, s.flags), (42, 7, 0b1010));
        assert_eq!(s.generation, a.generation);
    }

    #[test]
    fn slot_generation_matches_handle_generation_while_alive() {
        let mut e = Entities::new();
        for _ in 0..100 {
            let a = e.alloc();
            assert_eq!(e.slot(a).unwrap().generation, a.generation);
        }
    }

    // ----------------------------------------------------------- generation wrap
    //
    // The wrap rule: u32::MAX is odd (alive). Despawning from it
    // must skip 0 (NonZeroU32) AND land even (dead parity) → 2. Respawn → 3.

    #[test]
    fn despawn_from_max_generation_wraps_to_two() {
        let mut e = Entities::new();
        let a = e.alloc();
        e.slots[a.index as usize].generation = nz(u32::MAX);
        let at_max = handle(a.index, u32::MAX);
        assert!(e.is_alive(at_max));

        assert!(e.dealloc(at_max));
        let g = e.slots[a.index as usize].generation.get();
        assert_eq!(g, 2, "u32::MAX + 1 must wrap to 2: non-zero and even");
        assert!(!e.is_alive(at_max));
    }

    #[test]
    fn respawn_after_wrap_yields_generation_three_and_kills_prewrap_handle() {
        let mut e = Entities::new();
        let a = e.alloc();
        e.slots[a.index as usize].generation = nz(u32::MAX);
        let at_max = handle(a.index, u32::MAX);
        e.dealloc(at_max);

        let reborn = e.alloc();
        assert_eq!(reborn.index, a.index);
        assert_eq!(reborn.generation.get(), 3);
        assert!(e.is_alive(reborn));
        assert!(
            !e.is_alive(at_max),
            "pre-wrap handle must be stale after wrap"
        );
        assert!(!e.is_alive(a), "original gen-1 handle must be stale too");
        assert!(!e.dealloc(at_max));
        assert!(e.is_alive(reborn));
    }

    #[test]
    fn alloc_from_dead_slot_at_max_minus_one_reaches_max() {
        // Even value u32::MAX - 1 is a valid dead generation; the next alive
        // generation is u32::MAX itself, which must be representable.
        let mut e = Entities::new();
        let a = e.alloc();
        e.dealloc(a);
        e.slots[a.index as usize].generation = nz(u32::MAX - 1);

        let b = e.alloc();
        assert_eq!(b.index, a.index);
        assert_eq!(b.generation.get(), u32::MAX);
        assert!(e.is_alive(b));

        assert!(e.dealloc(b));
        assert_eq!(e.slots[a.index as usize].generation.get(), 2);
    }

    #[test]
    fn generations_never_hit_zero_over_a_long_churn_across_the_wrap() {
        let mut e = Entities::new();
        let a = e.alloc();
        e.slots[a.index as usize].generation = nz(u32::MAX - 6);
        assert!(e.dealloc(handle(a.index, u32::MAX - 6))); // MAX-6 is odd → alive
        for _ in 0..50 {
            let x = e.alloc();
            assert_ne!(x.generation.get(), 0);
            assert_eq!(x.generation.get() % 2, 1);
            assert!(e.dealloc(x));
            let dead = e.slots[a.index as usize].generation.get();
            assert_ne!(dead, 0);
            assert_eq!(dead % 2, 0);
        }
    }

    // ------------------------------------------------------------- equality

    #[test]
    fn entities_with_same_index_but_different_generation_are_not_equal() {
        let x = handle(5, 1);
        let y = handle(5, 3);
        assert_ne!(x, y);
        let set: HashSet<Entity> = [x, y].into_iter().collect();
        assert_eq!(set.len(), 2, "Hash must distinguish generations too");
    }

    // -------------------------------------------------------- model-based test
    //
    // Random alloc/dealloc sequences against a trivial model. Ops address
    // entities by index into the log of every handle ever created, so stale
    // handles are hit constantly. Invariants:
    //   * is_alive agrees with the model
    //   * dealloc returns true iff the handle was live in the model
    //   * no two live handles share an index
    //   * alive generations are odd

    #[derive(Debug, Clone)]
    enum Op {
        Alloc,
        Dealloc(usize), // index into `log`
    }

    fn op_strategy() -> impl Strategy<Value=Op> {
        prop_oneof![
            3 => Just(Op::Alloc),
            2 => (0usize..64).prop_map(Op::Dealloc),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        #[cfg_attr(miri, ignore = "proptest is too slow under miri; covered by deterministic tests")]
        fn alloc_dealloc_sequences_match_model(ops in prop::collection::vec(op_strategy(), 1..200)) {
            let mut e = Entities::new();
            let mut log: Vec<Entity> = Vec::new();
            let mut live: HashSet<Entity> = HashSet::new();

            for op in ops {
                match op {
                    Op::Alloc => {
                        let x = e.alloc();
                        prop_assert!(!live.iter().any(|l| l.index == x.index),
                            "fresh entity {:?} shares an index with a live one", x);
                        prop_assert_eq!(x.generation.get() % 2, 1);
                        live.insert(x);
                        log.push(x);
                    }
                    Op::Dealloc(i) => {
                        if log.is_empty() { continue; }
                        let x = log[i % log.len()];
                        let expected = live.remove(&x);
                        prop_assert_eq!(e.dealloc(x), expected, "dealloc({:?})", x);
                    }
                }

                for x in &log {
                    prop_assert_eq!(e.is_alive(*x), live.contains(x), "is_alive({:?})", x);
                    prop_assert_eq!(e.slot(*x).is_some(), live.contains(x));
                }
            }
        }
    }
}
