use crate::access::AccessList;
use crate::grant::AccessGrant;
use crate::query::data::QueryData;
use crate::world::WorldCells;
use crate::{Component, Query, QueryFilter, QueryState, World};
use std::ops::{Deref, DerefMut};

/// A value a system receives per run, fetched from the world.
///
/// # Safety
///
/// `ACCESS` must declare every component access the fetched item can perform.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a system parameter",
    note = "system functions take `Query<..>`, `Single<..>`, or `Local<..>` arguments"
)]
pub unsafe trait SystemParam {
    /// Access this parameter declares; checked against a system's other
    /// parameters at compile time.
    const ACCESS: AccessList;

    /// Per-system state persisting across runs.
    type State: 'static;

    /// What the system function receives.
    type Item<'w, 's>;

    fn init(world: &mut World) -> Self::State;

    /// Fetches the item at `version`, the world version of the current run.
    ///
    /// # Safety
    ///
    /// The caller guarantees the union of all simultaneously fetched
    /// parameters' access lists is conflict-free.
    unsafe fn fetch<'w, 's>(
        state: &'s mut Self::State,
        cells: &WorldCells<'w>,
        version: u64,
    ) -> Self::Item<'w, 's>;
}

unsafe impl<D, F> SystemParam for Query<'_, '_, D, F>
where
    D: QueryData + 'static,
    F: QueryFilter + 'static,
{
    const ACCESS: AccessList = D::ACCESS;
    type State = QueryState<D, F>;
    type Item<'w, 's> = Query<'w, 's, D, F>;

    fn init(_world: &mut World) -> Self::State {
        QueryState::new()
    }

    unsafe fn fetch<'w, 's>(
        state: &'s mut Self::State,
        cells: &WorldCells<'w>,
        version: u64,
    ) -> Self::Item<'w, 's> {
        let last_seen = state.advance_cursor(version);
        state.refresh(cells.archetypes, cells.reg);
        Query {
            chunks: cells.chunks,
            archetypes: cells.archetypes,
            reg: cells.reg,
            grant: AccessGrant::at_version(D::ACCESS, version),
            state,
            last_seen,
        }
    }
}

/// Per-system state: defaulted on the system's first run, persisting across
/// its runs, independent between system instances.
pub struct Local<'s, T>(&'s mut T);

impl<'s, T> Deref for Local<'s, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'s, T> DerefMut for Local<'s, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
    }
}

unsafe impl<T: Default + Send + Sync + 'static> SystemParam for Local<'_, T> {
    const ACCESS: AccessList = AccessList::EMPTY;
    type State = T;
    type Item<'w, 's> = Local<'s, T>;

    fn init(_world: &mut World) -> Self::State {
        T::default()
    }

    unsafe fn fetch<'w, 's>(
        state: &'s mut Self::State,
        _cells: &WorldCells<'w>,
        _version: u64,
    ) -> Self::Item<'w, 's> {
        Local(state)
    }
}

/// The reference shapes [`Single`] supports.
pub trait SingleData {
    type Target: Component;
    type Ref<'w>;

    fn as_ref<'w>(r: &'w Self::Ref<'_>) -> &'w Self::Target;
}

/// The mutable subset of [`SingleData`].
pub trait SingleDataMut: SingleData {
    fn as_mut<'w>(r: &'w mut Self::Ref<'_>) -> &'w mut Self::Target;
}

impl<T: Component> SingleData for &T {
    type Target = T;
    type Ref<'w> = &'w T;

    fn as_ref<'w>(r: &'w Self::Ref<'_>) -> &'w Self::Target {
        r
    }
}

impl<T: Component> SingleData for &mut T {
    type Target = T;
    type Ref<'w> = &'w mut T;

    fn as_ref<'w>(r: &'w Self::Ref<'_>) -> &'w Self::Target {
        r
    }
}

impl<T: Component> SingleDataMut for &mut T {
    fn as_mut<'w>(r: &'w mut Self::Ref<'_>) -> &'w mut Self::Target {
        r
    }
}

/// The single entity holding a `T`, as a system parameter.
///
/// `Single<&T>` yields shared access and `Single<&mut T>` mutable access,
/// via `Deref`/`DerefMut`. The world must contain exactly one entity with a
/// `T` when the system runs; any other count is a panic naming the component.
/// [`World::insert_singleton`] maintains that contract.
pub struct Single<'w, D: SingleData> {
    item: D::Ref<'w>,
}

impl<D: SingleData> Deref for Single<'_, D> {
    type Target = D::Target;

    fn deref(&self) -> &Self::Target {
        D::as_ref(&self.item)
    }
}

impl<D: SingleDataMut> DerefMut for Single<'_, D> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        D::as_mut(&mut self.item)
    }
}

/// Walks `state`'s matched chunks under a grant minted from `declared` — the
/// parameter's own declaration, so an under-declared access fails the fetch.
///
/// # Panics
///
/// Unless exactly one row exists across all matched chunks.
fn single_row<'w, D, F>(
    state: &mut QueryState<D, F>,
    cells: &WorldCells<'w>,
    version: u64,
    declared: AccessList,
    name: &str,
) -> D::Columns<'w>
where
    D: QueryData + 'static,
    F: QueryFilter + 'static,
{
    state.refresh(cells.archetypes, cells.reg);
    let query: Query<'w, '_, D, F> = Query {
        chunks: cells.chunks,
        archetypes: cells.archetypes,
        reg: cells.reg,
        grant: AccessGrant::at_version(declared, version),
        state,
        last_seen: 0,
    };

    let mut found = None;
    let mut count = 0usize;
    let mut iter = query.chunks();
    while let Some((columns, len)) = iter.next_chunk() {
        count += len;
        if len == 1 && found.is_none() {
            found = Some(columns);
        }
    }
    match (found, count) {
        (Some(columns), 1) => columns,
        (_, n) => panic!("expected exactly one entity with a '{name}', found {n}"),
    }
}

unsafe impl<T: Component> SystemParam for Single<'_, &T> {
    const ACCESS: AccessList = AccessList::read(T::KEY);
    type State = QueryState<&'static T>;
    type Item<'w, 's> = Single<'w, &'w T>;

    fn init(_world: &mut World) -> Self::State {
        QueryState::new()
    }

    unsafe fn fetch<'w, 's>(
        state: &'s mut Self::State,
        cells: &WorldCells<'w>,
        version: u64,
    ) -> Self::Item<'w, 's> {
        let columns = single_row(
            state,
            cells,
            version,
            Self::ACCESS,
            std::any::type_name::<T>(),
        );
        Single { item: &columns[0] }
    }
}

unsafe impl<T: Component> SystemParam for Single<'_, &mut T> {
    const ACCESS: AccessList = AccessList::write(T::KEY);
    type State = QueryState<&'static mut T>;
    type Item<'w, 's> = Single<'w, &'w mut T>;

    fn init(_world: &mut World) -> Self::State {
        QueryState::new()
    }

    unsafe fn fetch<'w, 's>(
        state: &'s mut Self::State,
        cells: &WorldCells<'w>,
        version: u64,
    ) -> Self::Item<'w, 's> {
        let columns = single_row(
            state,
            cells,
            version,
            Self::ACCESS,
            std::any::type_name::<T>(),
        );
        #[allow(clippy::into_iter_without_iter, clippy::explicit_into_iter_loop)]
        #[allow(clippy::useless_conversion, reason = "into_iter consumes the slice reference, keeping the world lifetime; iter_mut would reborrow locally")]
        let row = IntoIterator::into_iter(columns).next().expect("exactly one row");
        Single { item: row }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{Component, ComponentKey};
    use crate::{IntoSystem, Local, Query, System, World};

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("system::param::tests::", stringify!($name)));
            }
        };
    }

    #[derive(Copy, Clone, PartialEq, Debug)]
    struct Time(u64);
    component!(Time);
    #[derive(Copy, Clone, PartialEq, Debug)]
    struct A(u64);
    component!(A);

    // ------------------------------------------------------------- world API

    #[test]
    fn insert_singleton_spawns_once_and_replaces_after() {
        let mut world = World::new();
        let first = world.insert_singleton(Time(1));
        assert_eq!(world.singleton::<Time>(), Some(&Time(1)));

        let second = world.insert_singleton(Time(2));
        assert_eq!(first, second, "one entity carries the singleton");
        assert_eq!(world.singleton::<Time>(), Some(&Time(2)));
        assert_eq!(world.len(), 1);
    }

    #[test]
    fn singleton_accessors_on_an_absent_component_are_none() {
        let mut world = World::new();
        assert_eq!(world.singleton::<Time>(), None);
        assert_eq!(world.singleton_mut::<Time>(), None);
    }

    #[test]
    fn singleton_mut_writes_are_visible() {
        let mut world = World::new();
        world.insert_singleton(Time(5));
        world.singleton_mut::<Time>().unwrap().0 = 6;
        assert_eq!(world.singleton::<Time>(), Some(&Time(6)));
    }

    // ---------------------------------------------------------- Single param

    #[test]
    fn single_reads_the_singleton() {
        fn scaled(time: Single<&Time>, query: Query<&mut A>) {
            let dt = time.0;
            query.for_each(|a| a.0 *= dt);
        }
        let mut world = World::new();
        world.insert_singleton(Time(3));
        let e = world.spawn((A(7),));
        let mut system = scaled.into_system();
        world.run(&mut system);
        assert_eq!(world.get::<A>(e), Some(&A(21)));
    }

    #[test]
    fn single_mut_writes_the_singleton() {
        fn tick(mut time: Single<&mut Time>) {
            time.0 += 1;
        }
        let mut world = World::new();
        world.insert_singleton(Time(10));
        let mut system = tick.into_system();
        world.run(&mut system);
        world.run(&mut system);
        assert_eq!(world.singleton::<Time>(), Some(&Time(12)));
    }

    #[test]
    fn single_finds_a_manually_spawned_unique_component() {
        // Uniqueness is what Single requires — not that insert_singleton was used.
        fn read(time: Single<&Time>, query: Query<&mut A>) {
            let t = time.0;
            query.for_each(|a| a.0 = t);
        }
        let mut world = World::new();
        world.spawn((Time(9), A(0))); // singleton living on a composite entity
        let probe = world.spawn((A(0),));
        let mut system = read.into_system();
        world.run(&mut system);
        assert_eq!(world.get::<A>(probe), Some(&A(9)));
    }

    #[test]
    #[should_panic(expected = "exactly one")]
    fn single_panics_when_the_component_is_absent() {
        fn wants(_time: Single<&Time>) {}
        let mut world = World::new();
        wants.into_system().run(&mut world);
    }

    #[test]
    #[should_panic(expected = "exactly one")]
    fn single_panics_when_the_component_is_duplicated() {
        fn wants(_time: Single<&Time>) {}
        let mut world = World::new();
        world.spawn((Time(1),));
        world.spawn((Time(2),));
        wants.into_system().run(&mut world);
    }

    #[test]
    #[should_panic(expected = "exactly one")]
    fn single_panics_when_duplicates_span_archetypes() {
        fn wants(_time: Single<&Time>) {}
        let mut world = World::new();
        world.spawn((Time(1),));
        world.spawn((Time(2), A(0))); // a second Time in a different archetype
        wants.into_system().run(&mut world);
    }

    #[test]
    fn single_declares_access_and_conflicts_are_caught() {
        // read-only Single alongside a query of the same component is fine
        fn peaceful(_time: Single<&Time>, _also: Query<&Time>) {}
        let mut world = World::new();
        world.insert_singleton(Time(0));
        peaceful.into_system().run(&mut world);
        // (&mut Single + Query of same component is a compile error; pinned
        // in the compile-fail doctests on World::run's docs.)
    }

    #[test]
    fn changed_singleton_is_observable() {
        use crate::Changed;
        fn bump(mut time: Single<&mut Time>) {
            time.0 += 1;
        }
        fn watch(changed: Query<&Time, Changed<Time>>, out: Query<&mut A>, mut seen: Local<u64>) {
            let mut n = 0;
            changed.for_each(|_| n += 1);
            *seen += n;
            let seen = *seen;
            out.for_each(|a| a.0 = seen);
        }
        let mut world = World::new();
        world.insert_singleton(Time(0));
        let probe = world.spawn((A(0),));
        let mut b = bump.into_system();
        let mut w = watch.into_system();
        world.run(&mut w); // sees the insert
        world.run(&mut b); // writes through Single<&mut _>
        world.run(&mut w); // must observe the singleton write
        assert_eq!(world.get::<A>(probe), Some(&A(2)));
    }
}
