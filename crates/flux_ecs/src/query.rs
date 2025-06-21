use crate::archetype::{Archetype, ArchetypeId};
use crate::component::{Component, ComponentId};
use crate::entity::Entity;
use crate::system::parameter::SystemParam;
use crate::world::World;
use std::marker::PhantomData;
use variadics_please::all_tuples;

pub unsafe trait QueryData {
    type Item<'w>;
    type Fetch<'w>;

    unsafe fn new_fetch<'w>(world: &'w World, archetype: &'w Archetype) -> Option<Self::Fetch<'w>>;

    unsafe fn fetch<'w>(fetch: &mut Self::Fetch<'w>, row: usize) -> Self::Item<'w>;

    /// Gets the component accesses required by this query. Used for archetype matching and safety checks.
    /// # Returns
    /// A vector of tuples where each tuple containing:
    /// - `ComponentId`: The ID of the component.
    /// - `bool`: Whether the component is mutable (`true`) or read-only (`false`).
    fn get_access(world: &mut World) -> Vec<(ComponentId, bool)>;
}

#[doc(hidden)]
pub struct ReadFetch<'w, T: Component> {
    column_ptr: *const T,
    _marker: PhantomData<&'w ()>,
}

unsafe impl<T: Component> QueryData for &T {
    type Item<'w> = &'w T;
    type Fetch<'w> = ReadFetch<'w, T>;

    unsafe fn new_fetch<'w>(world: &'w World, archetype: &'w Archetype) -> Option<Self::Fetch<'w>> {
        let component_id = world.component_registry.get_id::<T>()?;
        let column = archetype.columns().get(&component_id)?;

        Some(ReadFetch {
            column_ptr: column.get_ptr(0).cast::<T>(),
            _marker: PhantomData,
        })
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &mut Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        unsafe { &*fetch.column_ptr.add(row) }
    }

    fn get_access(world: &mut World) -> Vec<(ComponentId, bool)> {
        vec![(world.component_registry.register::<T>(), false)]
    }
}

#[doc(hidden)]
pub struct WriteFetch<'w, T: Component> {
    column_ptr: *mut T,
    _marker: PhantomData<&'w mut ()>,
}

unsafe impl<T: Component> QueryData for &mut T {
    type Item<'w> = &'w mut T;
    type Fetch<'w> = WriteFetch<'w, T>;

    unsafe fn new_fetch<'w>(world: &'w World, archetype: &'w Archetype) -> Option<Self::Fetch<'w>> {
        let component_id = world.component_registry.get_id::<T>()?;
        let column = archetype.columns().get(&component_id)?;

        Some(WriteFetch {
            // TODO: Maybe we have to use as *const T here?
            column_ptr: column.get_mut_ptr(0).cast::<T>(),
            _marker: PhantomData,
        })
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &mut Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        unsafe { &mut *fetch.column_ptr.add(row) }
    }

    fn get_access(world: &mut World) -> Vec<(ComponentId, bool)> {
        vec![(world.component_registry.register::<T>(), true)]
    }
}

unsafe impl QueryData for Entity {
    type Item<'w> = Entity;
    type Fetch<'w> = *const Entity;

    unsafe fn new_fetch<'w>(
        _world: &'w World,
        archetype: &'w Archetype,
    ) -> Option<Self::Fetch<'w>> {
        Some(archetype.entities().as_ptr())
    }

    #[inline]
    unsafe fn fetch<'w>(fetch: &mut Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        unsafe { *fetch.add(row) }
    }

    fn get_access(_world: &mut World) -> Vec<(ComponentId, bool)> {
        Vec::new()
    }
}

macro_rules! impl_query_data_for_tuple {
    ($($T:ident),+) => {
        #[allow(non_snake_case)]
        unsafe impl<$($T: QueryData),+> QueryData for ($($T,)+) {
            type Item<'w> = ($($T::Item<'w>,)+);
            type Fetch<'w> = ($($T::Fetch<'w>,)+);

            unsafe fn new_fetch<'w>(world: &'w World, archetype: &'w Archetype) -> Option<Self::Fetch<'w>> {
                unsafe {
                    $(
                    let $T = $T::new_fetch(world, archetype)?;
                    )+
                    Some(($($T,)+))
                }
            }

            #[inline]
            unsafe fn fetch<'w>(fetch: &mut Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
                unsafe {
                    let ($($T,)+) = fetch;
                    ($($T::fetch($T, row),)+)
                }
            }

            fn get_access(world: &mut World) -> Vec<(ComponentId, bool)> {
                let mut access = Vec::new();
                $(access.extend($T::get_access(world));)+

                // TODO: Check for mutability conflicts
                access
            }
        }
    }
}

all_tuples!(impl_query_data_for_tuple, 1, 15, T);

pub struct QueryState<Q: QueryData> {
    matching_archetypes: Vec<ArchetypeId>,
    _marker: PhantomData<Q>,
}

impl<Q: QueryData> QueryState<Q> {
    pub fn new(world: &mut World) -> Self {
        let required_access = Q::get_access(world);
        let required_ids = required_access
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();

        let matching_archetypes = world
            .archetypes()
            .iter()
            .filter(|archetype| {
                required_ids
                    .iter()
                    .all(|req_id| archetype.has_component(*req_id))
            })
            .map(Archetype::id)
            .collect();

        Self {
            matching_archetypes,
            _marker: PhantomData,
        }
    }
}

pub struct Query<'world, 'state, Q: QueryData> {
    world: &'world World,
    state: &'state QueryState<Q>,
}

impl<'world, 'state, Q: QueryData> IntoIterator for Query<'world, 'state, Q> {
    type Item = Q::Item<'world>;
    type IntoIter = QueryIter<'world, 'state, Q>;

    fn into_iter(self) -> Self::IntoIter {
        QueryIter {
            world: self.world,
            state: self.state,
            archetype_index: 0,
            current_fetch: None,
            current_archetype_len: 0,
            row_index: 0,
        }
    }
}


pub struct QueryIter<'w, 's, Q: QueryData> {
    world: &'w World,
    state: &'s QueryState<Q>,
    archetype_index: usize,
    current_fetch: Option<Q::Fetch<'w>>,
    current_archetype_len: usize,
    row_index: usize,
}

impl<'w, 's, Q: QueryData> Iterator for QueryIter<'w, 's, Q> {
    type Item = Q::Item<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ref mut fetch) = self.current_fetch {
                if self.row_index < self.current_archetype_len {
                    let item = unsafe { Q::fetch(fetch, self.row_index) };
                    self.row_index += 1;
                    return Some(item);
                }
            }

            if self.archetype_index == self.state.matching_archetypes.len() {
                return None;
            }

            let archetype_id = self.state.matching_archetypes[self.archetype_index];
            self.archetype_index += 1;

            let archetype = self
                .world
                .archetypes()
                .get(archetype_id)
                .expect("Archetype not found");

            self.current_fetch = unsafe { Q::new_fetch(self.world, archetype) };
            if self.current_fetch.is_some() {
                self.row_index = 0;
                self.current_archetype_len = archetype.len();
            }
        }
    }
}

impl<Q: QueryData + 'static> SystemParam for Query<'_, '_, Q> {
    type State = QueryState<Q>;
    type Item<'world, 'state> = Query<'world, 'state, Q>;

    fn init_state(world: &mut World) -> Self::State {
        QueryState::new(world)
    }

    fn get_param<'world, 'state>(
        state: &'state Self::State,
        world: &'world mut World,
    ) -> Self::Item<'world, 'state> {
        Query { world, state }
    }
}
