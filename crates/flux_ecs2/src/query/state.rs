use crate::query::data::QueryData;
use crate::registry::Registry;
use crate::storage::archetype::{ArchetypeId, Archetypes};
use std::marker::PhantomData;

/// Cached archetype matches of one query shape, reusable across calls.
///
/// Refreshing scans only archetypes created since the last refresh, so a
/// long-lived state stays current at O(new archetypes) per use.
pub struct QueryState<D: QueryData> {
    matched: Vec<ArchetypeId>,
    seen: usize,
    _data: PhantomData<fn() -> D>,
}

impl<D: QueryData> Default for QueryState<D> {
    fn default() -> Self {
        Self {
            matched: Vec::new(),
            seen: 0,
            _data: PhantomData,
        }
    }
}

impl<D: QueryData> QueryState<D> {
    /// Creates a state with no cached matches.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends archetypes created since the last refresh that `D` matches.
    pub(crate) fn refresh(&mut self, archetypes: &Archetypes, reg: &Registry) {
        for index in self.seen..archetypes.len() {
            let id = ArchetypeId(index as u32);
            if D::matches(archetypes.get(id).signature(), reg) {
                self.matched.push(id);
            }
        }
        self.seen = archetypes.len();
    }

    /// The matched archetypes, in creation order.
    pub(crate) fn matched(&self) -> &[ArchetypeId] {
        &self.matched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{Component, ComponentKey};
    use crate::{Entity, World};

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("query::state::tests::", stringify!($name)));
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

    // ------------------------------------------------------------- match cache

    #[test]
    fn archetypes_created_after_first_use_are_picked_up() {
        // Regression guard: a stale match cache silently under-iterates.
        let mut world = World::new();
        world.spawn((A(1),));
        let mut state = QueryState::<&A>::new();

        let first: u64 = world
            .query(&mut state)
            .chunks()
            .map(|a| a.iter().map(|v| v.0).sum::<u64>())
            .sum();
        assert_eq!(first, 1);

        world.spawn((A(10), B(0))); // NEW archetype {A, B}, created after the state existed
        let second: u64 = world
            .query(&mut state)
            .chunks()
            .map(|a| a.iter().map(|v| v.0).sum::<u64>())
            .sum();
        assert_eq!(
            second, 11,
            "rows in archetypes created since the last use must appear"
        );
    }

    #[test]
    fn refresh_scans_only_new_archetypes() {
        let mut world = World::new();
        world.spawn((A(1),));
        world.spawn((A(1), B(2)));
        let mut state = QueryState::<&A>::new();
        let _ = world.query(&mut state);
        let matched_after_first = state.matched().len();
        assert_eq!(matched_after_first, 2);

        // No new archetypes: refresh must be a no-op, not a rescan that
        // duplicates matches.
        let _ = world.query(&mut state);
        assert_eq!(
            state.matched().len(),
            2,
            "re-refresh must not duplicate matches"
        );
    }

    #[test]
    fn non_matching_archetypes_are_not_cached() {
        let mut world = World::new();
        world.spawn((A(1),));
        world.spawn((B(1),));
        world.spawn(());
        let mut state = QueryState::<(&A,)>::new();
        let _ = world.query(&mut state);
        assert_eq!(state.matched().len(), 1, "only {{A}} matches (&A,)");
    }

    // -------------------------------------------------------------- iteration

    #[test]
    fn query_visits_exactly_the_matching_rows() {
        let mut world = World::new();
        world.spawn((A(1),));
        world.spawn((A(2), B(20)));
        world.spawn((B(30),));

        let mut a_state = QueryState::<&A>::new();
        let a_sum: u64 = world
            .query(&mut a_state)
            .chunks()
            .map(|a| a.iter().map(|v| v.0).sum::<u64>())
            .sum();
        assert_eq!(a_sum, 3, "&A sees both archetypes containing A");

        let mut ab_state = QueryState::<(&A, &B)>::new();
        let mut pairs = Vec::new();
        for (a, b) in world.query(&mut ab_state).chunks() {
            for (av, bv) in a.iter().zip(b.iter()) {
                pairs.push((av.0, bv.0));
            }
        }
        assert_eq!(
            pairs,
            vec![(2, 20)],
            "(&A, &B) sees only the {{A, B}} archetype"
        );
    }

    #[test]
    fn iteration_spans_multiple_chunks() {
        let mut world = World::new();
        for i in 0..100u64 {
            world.spawn((Big([i; 64]), A(i)));
        }
        let mut state = QueryState::<&A>::new();
        let mut seen: Vec<u64> = world
            .query(&mut state)
            .chunks()
            .flat_map(|a| a.iter().map(|v| v.0).collect::<Vec<_>>())
            .collect();
        seen.sort();
        assert_eq!(
            seen,
            (0..100).collect::<Vec<_>>(),
            "every row across every chunk, exactly once"
        );
    }

    #[test]
    fn mutable_query_updates_every_matched_row() {
        let mut world = World::new();
        for i in 0..10u64 {
            world.spawn((A(i), B(i as u16)));
        }
        let mut w_state = QueryState::<(&mut A, &B)>::new();
        for (a, b) in world.query(&mut w_state).chunks() {
            for (av, bv) in a.iter_mut().zip(b.iter()) {
                av.0 += 1000 * bv.0 as u64;
            }
        }
        let mut r_state = QueryState::<&A>::new();
        let sum: u64 = world
            .query(&mut r_state)
            .chunks()
            .map(|a| a.iter().map(|v| v.0).sum::<u64>())
            .sum();
        assert_eq!(sum, (0..10u64).map(|i| i + 1000 * i).sum::<u64>());
    }

    #[test]
    fn entity_and_option_members_iterate() {
        let mut world = World::new();
        let with_b = world.spawn((A(1), B(7)));
        let without_b = world.spawn((A(2),));

        let mut state = QueryState::<(Entity, &A, Option<&B>)>::new();
        let mut rows = Vec::new();
        for (entities, a, b) in world.query(&mut state).chunks() {
            for (i, e) in entities.iter().enumerate() {
                rows.push((*e, a[i].0, b.map(|b| b[i].0)));
            }
        }
        rows.sort_by_key(|r| r.1);
        assert_eq!(rows, vec![(with_b, 1, Some(7)), (without_b, 2, None)]);
    }

    #[test]
    fn no_matches_yields_nothing() {
        let mut world = World::new();
        world.spawn((B(1),));
        let mut state = QueryState::<&A>::new();
        assert_eq!(world.query(&mut state).chunks().count(), 0);

        let mut empty_world = World::new();
        let mut state = QueryState::<&A>::new();
        assert_eq!(empty_world.query(&mut state).chunks().count(), 0);
    }

    #[test]
    fn a_fresh_query_can_run_immediately_after_another() {
        let mut world = World::new();
        world.spawn((A(5),));
        let mut state = QueryState::<&mut A>::new();
        for a in world.query(&mut state).chunks() {
            a[0].0 += 1;
        }
        for a in world.query(&mut state).chunks() {
            a[0].0 += 1;
        }
        let mut read = QueryState::<&A>::new();
        let v: u64 = world.query(&mut read).chunks().map(|a| a[0].0).sum();
        assert_eq!(v, 7, "two mutable passes then a read");
    }

    #[test]
    fn despawned_rows_disappear_from_iteration() {
        let mut world = World::new();
        let keep = world.spawn((A(1),));
        let kill = world.spawn((A(2),));
        world.despawn(kill);
        let mut state = QueryState::<(Entity, &A)>::new();
        let mut rows = Vec::new();
        for (entities, a) in world.query(&mut state).chunks() {
            for (i, e) in entities.iter().enumerate() {
                rows.push((*e, a[i].0));
            }
        }
        assert_eq!(rows, vec![(keep, 1)]);
    }
}
