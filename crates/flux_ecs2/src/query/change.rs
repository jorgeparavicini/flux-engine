use crate::query::data::ChunkView;
use crate::registry::Registry;
use crate::{Component, ComponentId, QueryFilter};
use std::marker::PhantomData;

/// Passes chunks whose `T` column was written since the reader's last run.
///
/// Detection is chunk-granular and conservative: every row that changed is
/// included, and rows that did not change may be included with them — a chunk
/// reappears whole. Taking mutable access counts as a write, whether or not
/// a value was stored.
pub struct Changed<T>(PhantomData<fn() -> T>);

impl<T: Component> QueryFilter for Changed<T> {
    fn filter_matches(signature: &[ComponentId], reg: &Registry) -> bool {
        let Some(id) = reg.lookup(T::KEY) else {
            return false;
        };
        signature.binary_search(&id).is_ok()
    }

    fn keep_chunk(view: &ChunkView<'_>, last_seen: u64) -> bool {
        let Some(column) = view.column_index(T::KEY) else {
            return false;
        };

        view.chunks.write_version(view.chunk, column) > last_seen
    }
}

/// Passes chunks where rows gained a `T` since the reader's last run: spawns,
/// inserts, and archetype moves into the chunk all count.
///
/// Chunk-granular and conservative, like [`Changed`]: rows that were already
/// present reappear along with the arrivals.
pub struct Added<T>(PhantomData<fn() -> T>);

impl<T: Component> QueryFilter for Added<T> {
    fn filter_matches(signature: &[ComponentId], reg: &Registry) -> bool {
        let Some(component_id) = reg.lookup(T::KEY) else {
            return false;
        };
        signature.binary_search(&component_id).is_ok()
    }

    fn keep_chunk(view: &ChunkView<'_>, last_seen: u64) -> bool {
        let Some(column) = view.column_index(T::KEY) else {
            return false;
        };

        view.chunks.added_version(view.chunk, column) > last_seen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{Component, ComponentKey};
    use crate::query::state::QueryState;
    use crate::{With, World};

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("query::change::tests::", stringify!($name)));
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
    struct Big([u64; 512]);
    component!(Big);

    /// Sum of A over the chunks a filtered reader currently sees.
    fn observe<F: QueryFilter>(world: &mut World, state: &mut QueryState<&A, F>) -> u64 {
        world
            .query(state)
            .chunks()
            .map(|a| a.iter().map(|v| v.0).sum::<u64>())
            .sum()
    }

    // ---------------------------------------------------------------- baseline

    #[test]
    fn fresh_readers_see_every_row_as_changed_and_added() {
        let mut world = World::new();
        world.spawn((A(1),));
        world.spawn((A(2), B(0)));
        let mut changed = QueryState::<&A, Changed<A>>::new();
        let mut added = QueryState::<&A, Added<A>>::new();
        assert_eq!(observe(&mut world, &mut changed), 3);
        assert_eq!(observe(&mut world, &mut added), 3);
    }

    #[test]
    fn a_reader_that_has_seen_everything_sees_nothing() {
        let mut world = World::new();
        world.spawn((A(1),));
        let mut changed = QueryState::<&A, Changed<A>>::new();
        let mut added = QueryState::<&A, Added<A>>::new();
        assert_eq!(observe(&mut world, &mut changed), 1);
        assert_eq!(
            observe(&mut world, &mut changed),
            0,
            "no writes between runs"
        );
        assert_eq!(observe(&mut world, &mut added), 1);
        assert_eq!(observe(&mut world, &mut added), 0);
    }

    // ---------------------------------------------------------------- writes

    #[test]
    fn get_mut_marks_only_the_touched_chunk() {
        let mut world = World::new();
        let touched = world.spawn((A(1),));
        world.spawn((A(2), B(0))); // different archetype → different chunk
        let mut changed = QueryState::<&A, Changed<A>>::new();
        observe(&mut world, &mut changed); // catch up

        world.get_mut::<A>(touched).unwrap().0 = 10;
        assert_eq!(
            observe(&mut world, &mut changed),
            10,
            "only the written chunk reappears"
        );
    }

    #[test]
    fn mutable_query_fetch_marks_fetched_chunks() {
        let mut world = World::new();
        world.spawn((A(1),));
        world.spawn((A(2), B(0)));
        let mut changed = QueryState::<&A, Changed<A>>::new();
        observe(&mut world, &mut changed);

        // Fetch &mut A only where B is present: one chunk stamped.
        let mut writer = QueryState::<&mut A, With<B>>::new();
        for a in world.query(&mut writer).chunks() {
            for v in a {
                v.0 += 100;
            }
        }
        assert_eq!(observe(&mut world, &mut changed), 102);
    }

    #[test]
    fn replacing_a_component_is_changed_but_not_added() {
        let mut world = World::new();
        let e = world.spawn((A(1),));
        let mut changed = QueryState::<&A, Changed<A>>::new();
        let mut added = QueryState::<&A, Added<A>>::new();
        observe(&mut world, &mut changed);
        observe(&mut world, &mut added);

        world.insert(e, A(5)); // replace in place
        assert_eq!(observe(&mut world, &mut changed), 5);
        assert_eq!(
            observe(&mut world, &mut added),
            0,
            "replacement adds nothing"
        );
    }

    #[test]
    fn writes_do_not_count_as_added() {
        let mut world = World::new();
        let e = world.spawn((A(1),));
        let mut added = QueryState::<&A, Added<A>>::new();
        observe(&mut world, &mut added);
        world.get_mut::<A>(e).unwrap().0 = 2;
        assert_eq!(observe(&mut world, &mut added), 0);

        // the mutable-query path must not count as added either
        let mut writer = QueryState::<&mut A>::new();
        for a in world.query(&mut writer).chunks() {
            for v in a {
                v.0 += 1;
            }
        }
        assert_eq!(observe(&mut world, &mut added), 0);
    }

    // ---------------------------------------------------------------- spawns

    #[test]
    fn new_spawns_are_added_and_changed() {
        let mut world = World::new();
        world.spawn((A(1),));
        let mut changed = QueryState::<&A, Changed<A>>::new();
        let mut added = QueryState::<&A, Added<A>>::new();
        observe(&mut world, &mut changed);
        observe(&mut world, &mut added);

        world.spawn((A(4),)); // same archetype, same chunk
        // chunk granularity: the whole chunk reappears
        assert_eq!(observe(&mut world, &mut added), 5);
        assert_eq!(observe(&mut world, &mut changed), 5);
    }

    #[test]
    fn archetype_move_counts_as_added_in_the_target() {
        let mut world = World::new();
        let mover = world.spawn((A(7),));
        world.spawn((A(1), B(0))); // target archetype pre-exists
        let mut added = QueryState::<&A, Added<A>>::new();
        observe(&mut world, &mut added);

        world.insert(mover, B(1)); // {A} -> {A, B}
        let seen = observe(&mut world, &mut added);
        assert_eq!(
            seen, 8,
            "the target chunk (both rows, chunk granularity) reappears"
        );
    }

    // ------------------------------------------------------------ granularity

    #[test]
    fn change_detection_is_chunk_granular() {
        let mut world = World::new();
        let touched = world.spawn((A(1),));
        world.spawn((A(2),)); // same chunk
        let mut changed = QueryState::<&A, Changed<A>>::new();
        observe(&mut world, &mut changed);

        world.get_mut::<A>(touched).unwrap().0 = 10;
        assert_eq!(
            observe(&mut world, &mut changed),
            12,
            "both rows of the stamped chunk reappear; per-row precision is not promised"
        );
    }

    #[test]
    fn chunks_in_the_same_archetype_are_stamped_independently() {
        let mut world = World::new();
        // Big caps rows per chunk low enough that 40 spawns span chunks.
        let entities: Vec<_> = (0..40u64)
            .map(|i| world.spawn((Big([0; 512]), A(i))))
            .collect();
        let mut changed = QueryState::<&A, Changed<A>>::new();
        observe(&mut world, &mut changed);

        world.get_mut::<A>(entities[0]).unwrap().0 = 1000;
        let seen = observe(&mut world, &mut changed);
        assert!(seen >= 1000, "the touched chunk reappears");
        let full: u64 = (1..40).sum::<u64>() + 1000;
        assert!(seen < full, "untouched chunks stay filtered out");
    }

    // ----------------------------------------------------------------- cursors

    #[test]
    fn readers_have_independent_cursors() {
        let mut world = World::new();
        let e = world.spawn((A(1),));
        let mut early = QueryState::<&A, Changed<A>>::new();
        let mut late = QueryState::<&A, Changed<A>>::new();
        observe(&mut world, &mut early);

        world.get_mut::<A>(e).unwrap().0 = 2;
        assert_eq!(
            observe(&mut world, &mut early),
            2,
            "early reader sees the write"
        );
        assert_eq!(
            observe(&mut world, &mut late),
            2,
            "late reader sees everything so far"
        );
        assert_eq!(observe(&mut world, &mut early), 0);
        assert_eq!(observe(&mut world, &mut late), 0);
    }

    #[test]
    fn an_infrequent_reader_accumulates_changes() {
        let mut world = World::new();
        let e = world.spawn((A(0),));
        let mut reader = QueryState::<&A, Changed<A>>::new();
        observe(&mut world, &mut reader);

        // several writes between reader runs collapse into one observation
        for i in 1..=3u64 {
            world.get_mut::<A>(e).unwrap().0 = i;
        }
        assert_eq!(
            observe(&mut world, &mut reader),
            3,
            "sees the latest state once"
        );
        assert_eq!(observe(&mut world, &mut reader), 0);
    }

    #[test]
    fn a_changed_writer_does_not_retrigger_itself() {
        // The classic feedback loop: a writer filtered by Changed must not
        // observe its own stamps on its next run.
        let mut world = World::new();
        world.spawn((A(1),));
        let mut writer = QueryState::<&mut A, Changed<A>>::new();
        let mut passes = 0;
        for _ in 0..3 {
            for a in world.query(&mut writer).chunks() {
                for v in a {
                    v.0 += 1;
                    passes += 1;
                }
            }
        }
        assert_eq!(passes, 1, "the write is observed once, then never again");
    }

    #[test]
    fn change_filters_compose_with_structural_filters() {
        let mut world = World::new();
        let with_b = world.spawn((A(1), B(0)));
        let plain = world.spawn((A(2),));
        let mut state = QueryState::<&A, (Changed<A>, With<B>)>::new();
        assert_eq!(
            observe(&mut world, &mut state),
            1,
            "structural filter still applies"
        );

        world.get_mut::<A>(plain).unwrap().0 = 20;
        world.get_mut::<A>(with_b).unwrap().0 = 10;
        assert_eq!(
            observe(&mut world, &mut state),
            10,
            "only the With<B> chunk is observed"
        );
    }
}
