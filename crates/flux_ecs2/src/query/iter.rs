use crate::grant::AccessGrant;
use crate::query::data::{ChunkView, QueryData};
use crate::query::filter::QueryFilter;
use crate::query::state::QueryState;
use crate::registry::Registry;
use crate::storage::archetype::Archetypes;
use crate::storage::chunks::Chunks;

/// A prepared query over one world, consumed by iteration.
///
/// Constructed by [`World::query`](crate::World::query); a shape that aliases
/// a component mutably (e.g. `(&A, &mut A)`) fails to compile there.
pub struct Query<'w, 's, D: QueryData, F: QueryFilter = ()> {
    pub(crate) chunks: &'w Chunks,
    pub(crate) archetypes: &'w Archetypes,
    pub(crate) reg: &'w Registry,
    pub(crate) grant: AccessGrant,
    pub(crate) state: &'s QueryState<D, F>,
    pub(crate) last_seen: u64,
}

impl<'w, 's, D: QueryData, F: QueryFilter> Query<'w, 's, D, F> {
    /// Iterates the matched chunks, yielding each chunk's columns.
    pub fn chunks(self) -> ChunkIter<'w, 's, D, F> {
        ChunkIter {
            query: self,
            archetype_index: 0,
            chunk_index: 0,
            plan: None,
        }
    }

    /// Calls `f` once per matched row.
    ///
    /// Sugar over [`chunks`](Self::chunks): identical work, one row at a
    /// time. Use `chunks` when whole columns are wanted at once.
    pub fn for_each(self, mut f: impl FnMut(D::Item<'_>)) {
        let mut iter = self.chunks();
        while let Some((mut columns, len)) = iter.next_chunk() {
            for row in 0..len {
                f(D::row(&mut columns, row));
            }
        }
    }
}

/// Iterator over a query's matched chunks.
///
/// Yielded columns borrow the world for `'w`; the exclusive world borrow
/// taken by [`World::query`](crate::World::query) is what excludes aliasing
/// between queries.
pub struct ChunkIter<'w, 's, D: QueryData, F: QueryFilter = ()> {
    query: Query<'w, 's, D, F>,
    /// Position in the state's matched-archetype list.
    archetype_index: usize,
    /// Position in the current archetype's chunk list.
    chunk_index: usize,
    /// Fetch plan for the current archetype, resolved on entering it.
    plan: Option<D::Plan>,
}

impl<'w, D: QueryData, F: QueryFilter> ChunkIter<'w, '_, D, F> {
    pub(crate) fn next_chunk(&mut self) -> Option<(D::Columns<'w>, usize)> {
        loop {
            let arch_id = *self.query.state.matched().get(self.archetype_index)?;
            let arch = self.query.archetypes.get(arch_id);

            let Some(&chunk) = arch.chunks.get(self.chunk_index) else {
                // This archetype is exhausted; move to the next one.
                self.archetype_index += 1;
                self.chunk_index = 0;
                self.plan = None;
                continue;
            };
            self.chunk_index += 1;
            let view = ChunkView {
                chunks: self.query.chunks,
                layout: &arch.layout,
                reg: self.query.reg,
                chunk,
            };

            if !F::keep_chunk(&view, self.query.last_seen) {
                continue;
            }

            let plan = match self.plan {
                Some(plan) => plan,
                None => {
                    let plan = D::plan(&arch.layout, self.query.reg)
                        .expect("state only matches archetypes D can serve");
                    self.plan = Some(plan);
                    plan
                }
            };

            let len = view.len();

            // SAFETY: `chunk` belongs to `arch`, which the state matched
            // against D's requirements; `plan` was resolved from its layout.
            if let Some(columns) = unsafe { D::fetch(&view, plan, &mut self.query.grant) } {
                return Some((columns, len));
            }
        }
    }
}

impl<'w, D: QueryData, F: QueryFilter> Iterator for ChunkIter<'w, '_, D, F> {
    type Item = D::Columns<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_chunk().map(|(columns, _)| columns)
    }
}

#[cfg(test)]
mod tests {
    use crate::component::{Component, ComponentKey};
    use crate::query::state::QueryState;
    use crate::{Changed, Entity, With, World};

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("query::iter::tests::", stringify!($name)));
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

    // ------------------------------------------------------------- for_each

    #[test]
    fn for_each_visits_every_row_exactly_once() {
        let mut world = World::new();
        // spans several chunks and two archetypes
        for i in 0..40u64 {
            world.spawn((Big([0; 512]), A(i)));
        }
        for i in 40..50u64 {
            world.spawn((A(i), B(0)));
        }
        let mut by_chunks = QueryState::<&A>::new();
        let chunk_sum: u64 = world
            .query(&mut by_chunks)
            .chunks()
            .map(|a| a.iter().map(|v| v.0).sum::<u64>())
            .sum();

        let mut by_rows = QueryState::<&A>::new();
        let mut row_sum = 0;
        let mut rows = 0;
        world.query(&mut by_rows).for_each(|a| {
            row_sum += a.0;
            rows += 1;
        });
        assert_eq!(row_sum, chunk_sum, "both iteration forms see the same data");
        assert_eq!(rows, 50);
    }

    #[test]
    fn for_each_mutations_land() {
        let mut world = World::new();
        let entities: Vec<_> = (0..5u64).map(|i| world.spawn((A(i), B(i as u16)))).collect();
        let mut writer = QueryState::<(&mut A, &B)>::new();
        world.query(&mut writer).for_each(|(a, b)| {
            a.0 += 100 * u64::from(b.0);
        });
        for (i, e) in entities.iter().enumerate() {
            let i = i as u64;
            assert_eq!(world.get::<A>(*e), Some(&A(i + 100 * i)));
        }
    }

    #[test]
    fn for_each_projects_entities_and_optional_members() {
        let mut world = World::new();
        let with_b = world.spawn((A(1), B(7)));
        let without_b = world.spawn((A(2),));
        let mut state = QueryState::<(Entity, &A, Option<&B>)>::new();
        let mut rows = Vec::new();
        world.query(&mut state).for_each(|(e, a, b)| {
            rows.push((e, a.0, b.map(|b| b.0)));
        });
        rows.sort_by_key(|r| r.1);
        assert_eq!(
            rows,
            vec![(with_b, 1, Some(7)), (without_b, 2, None)],
            "rows in optional-less chunks are still visited"
        );
    }

    #[test]
    fn for_each_composes_with_filters() {
        let mut world = World::new();
        world.spawn((A(1), B(0)));
        let plain = world.spawn((A(2),));
        let mut state = QueryState::<&A, With<B>>::new();
        let mut sum = 0;
        world.query(&mut state).for_each(|a| sum += a.0);
        assert_eq!(sum, 1);

        let mut changed = QueryState::<&A, Changed<A>>::new();
        world.query(&mut changed).for_each(|_| {});
        world.get_mut::<A>(plain).unwrap().0 = 20;
        let mut seen = 0;
        world.query(&mut changed).for_each(|a| seen += a.0);
        assert_eq!(seen, 20, "change filter applies to row iteration too");
    }

    #[test]
    fn for_each_on_no_matches_never_calls_the_closure() {
        let mut world = World::new();
        world.spawn((B(1),));
        let mut state = QueryState::<&A>::new();
        world.query(&mut state).for_each(|_| unreachable!("no A rows exist"));
    }
}
