use crate::access::AccessList;
use crate::grant::AccessGrant;
use crate::query::data::QueryData;
use crate::world::WorldCells;
use crate::{Query, QueryFilter, QueryState, World};
use std::ops::{Deref, DerefMut};

/// A value a system receives per run, fetched from the world.
///
/// # Safety
///
/// `ACCESS` must declare every component access the fetched item can perform.
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
