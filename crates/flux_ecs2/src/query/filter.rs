use crate::query::data::ChunkView;
use crate::registry::Registry;
use crate::{Component, ComponentId};
use std::marker::PhantomData;

/// A structural condition on which archetypes a query visits.
///
/// Filters narrow matching only: they fetch nothing and declare no access.
/// The unit filter `()` passes everything; tuples of filters require every
/// member to pass.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a query filter",
    note = "filters are `With<T>`, `Without<T>`, `Changed<T>`, `Added<T>`, `()`, or a tuple of filters"
)]
pub trait QueryFilter {
    /// Whether an archetype with this signature passes.
    fn filter_matches(signature: &[ComponentId], reg: &Registry) -> bool;

    #[allow(unused_variables)]
    fn keep_chunk(view: &ChunkView<'_>, last_seen: u64) -> bool {
        true
    }
}

impl QueryFilter for () {
    fn filter_matches(_signature: &[ComponentId], _reg: &Registry) -> bool {
        true
    }
}

/// Passes archetypes that have a `T` component.
///
/// An archetype can only have a component the world has seen, so `With` of a
/// type never spawned or inserted anywhere passes nothing.
pub struct With<T>(PhantomData<fn() -> T>);

impl<T: Component> QueryFilter for With<T> {
    fn filter_matches(signature: &[ComponentId], reg: &Registry) -> bool {
        let Some(id) = reg.lookup(T::KEY) else {
            return false;
        };
        signature.binary_search(&id).is_ok()
    }
}

/// Passes archetypes that do not have a `T` component.
///
/// A type the world has never seen is absent everywhere, so `Without` of it
/// passes everything.
pub struct Without<T>(PhantomData<fn() -> T>);

impl<T: Component> QueryFilter for Without<T> {
    fn filter_matches(signature: &[ComponentId], reg: &Registry) -> bool {
        let Some(id) = reg.lookup(T::KEY) else {
            return true;
        };
        signature.binary_search(&id).is_err()
    }
}

macro_rules! tuple_query_filter {
    ($($t:ident),+) => {
        impl<$($t: QueryFilter),+> QueryFilter for ($($t,)+) {
            fn filter_matches(signature: &[ComponentId], reg: &Registry) -> bool {
                $($t::filter_matches(signature, reg))&&+
            }
            
            fn keep_chunk(view: &ChunkView<'_>, last_seen: u64) -> bool {
                $($t::keep_chunk(view, last_seen))&&+
            }
        }
    };
}

tuple_query_filter!(T1);
tuple_query_filter!(T1, T2);
tuple_query_filter!(T1, T2, T3);
tuple_query_filter!(T1, T2, T3, T4);
tuple_query_filter!(T1, T2, T3, T4, T5);
tuple_query_filter!(T1, T2, T3, T4, T5, T6);
tuple_query_filter!(T1, T2, T3, T4, T5, T6, T7);
tuple_query_filter!(T1, T2, T3, T4, T5, T6, T7, T8);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;
    use crate::component::{Component, ComponentKey, StorageClass};
    use crate::query::state::QueryState;

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("query::filter::tests::", stringify!($name)));
            }
        };
    }

    #[derive(Copy, Clone, PartialEq, Debug)]
    struct A(u64);
    component!(A);
    #[derive(Copy, Clone, PartialEq, Debug)]
    struct B(u16);
    component!(B);
    struct Frozen;
    impl Component for Frozen {
        const KEY: ComponentKey = ComponentKey::from_path("query::filter::tests::Frozen");
        const STORAGE: StorageClass = StorageClass::Tag;
    }
    #[derive(Copy, Clone)]
    struct NeverSpawned(#[allow(dead_code)] u8);
    component!(NeverSpawned);

    fn world() -> World {
        // {A}, {A, B}, {A, Frozen}: one entity each, A payloads 1, 2, 4.
        let mut w = World::new();
        w.spawn((A(1),));
        w.spawn((A(2), B(0)));
        w.spawn((A(4), Frozen));
        w
    }

    fn a_sum<F: QueryFilter>(w: &mut World) -> u64 {
        let mut state = QueryState::<&A, F>::new();
        w.query(&mut state).chunks().map(|a| a.iter().map(|v| v.0).sum::<u64>()).sum()
    }

    // -------------------------------------------------------------- predicates

    #[test]
    fn unit_filter_passes_everything() {
        assert_eq!(a_sum::<()>(&mut world()), 7);
    }

    #[test]
    fn with_requires_the_component() {
        assert_eq!(a_sum::<With<B>>(&mut world()), 2);
        assert_eq!(a_sum::<With<Frozen>>(&mut world()), 4, "tag components filter too");
    }

    #[test]
    fn without_rejects_the_component() {
        assert_eq!(a_sum::<Without<B>>(&mut world()), 5);
        assert_eq!(a_sum::<Without<Frozen>>(&mut world()), 3);
    }

    #[test]
    fn without_an_unknown_component_rejects_nothing() {
        assert_eq!(a_sum::<Without<NeverSpawned>>(&mut world()), 7);
        let mut w = world();
        let before = {
            let mut state = QueryState::<&A, Without<NeverSpawned>>::new();
            let _ = w.query(&mut state);
            // filtering must not register the type as a side effect
            w.query(&mut QueryState::<&A>::new()).chunks().count()
        };
        assert!(before > 0);
    }

    #[test]
    fn with_an_unknown_component_rejects_everything() {
        assert_eq!(a_sum::<With<NeverSpawned>>(&mut world()), 0);
    }

    #[test]
    fn tuple_filters_require_every_member() {
        assert_eq!(a_sum::<(Without<B>, Without<Frozen>)>(&mut world()), 1);
        assert_eq!(a_sum::<(With<B>, Without<Frozen>)>(&mut world()), 2);
        assert_eq!(a_sum::<(With<B>, With<Frozen>)>(&mut world()), 0, "no archetype has both");
    }

    // ------------------------------------------------------------- integration

    #[test]
    fn filters_do_not_change_what_is_fetched() {
        let mut w = world();
        let mut state = QueryState::<(&mut A, Option<&B>), Without<Frozen>>::new();
        for (a, b) in w.query(&mut state).chunks() {
            for (i, av) in a.iter_mut().enumerate() {
                av.0 += 10 * b.map_or(0, |b| u64::from(b[i].0) + 1);
            }
        }
        // {A}: untouched by the write rule (b is None → +0); {A,B}: 2 + 10*(0+1)
        assert_eq!(a_sum::<()>(&mut w), 1 + 12 + 4);
    }

    #[test]
    fn filtered_state_stays_incremental() {
        let mut w = World::new();
        w.spawn((A(1),));
        let mut state = QueryState::<&A, Without<B>>::new();
        assert_eq!(w.query(&mut state).chunks().map(|a| a.len()).sum::<usize>(), 1);

        w.spawn((A(2), B(0))); // new archetype AFTER first use; filtered out
        w.spawn((A(4),));      // same archetype as the first: no new match entry
        let rows: usize = w.query(&mut state).chunks().map(|a| a.len()).sum();
        assert_eq!(rows, 2, "unfiltered rows appear, the filtered archetype never does");
    }
}
