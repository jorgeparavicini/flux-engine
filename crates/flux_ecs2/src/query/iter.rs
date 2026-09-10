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
pub struct Query<'w, 's, D: QueryData, F: QueryFilter> {
    pub(crate) chunks: &'w Chunks,
    pub(crate) archetypes: &'w Archetypes,
    pub(crate) reg: &'w Registry,
    pub(crate) grant: AccessGrant,
    pub(crate) state: &'s QueryState<D, F>,
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
}

/// Iterator over a query's matched chunks.
///
/// Yielded columns borrow the world for `'w`; the exclusive world borrow
/// taken by [`World::query`](crate::World::query) is what excludes aliasing
/// between queries.
pub struct ChunkIter<'w, 's, D: QueryData, F: QueryFilter> {
    query: Query<'w, 's, D, F>,
    /// Position in the state's matched-archetype list.
    archetype_index: usize,
    /// Position in the current archetype's chunk list.
    chunk_index: usize,
    /// Fetch plan for the current archetype, resolved on entering it.
    plan: Option<D::Plan>,
}

impl<'w, D: QueryData, F: QueryFilter> Iterator for ChunkIter<'w, '_, D, F> {
    type Item = D::Columns<'w>;

    fn next(&mut self) -> Option<Self::Item> {
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

            let plan = match self.plan {
                Some(plan) => plan,
                None => {
                    let plan = D::plan(&arch.layout, self.query.reg)
                        .expect("state only matches archetypes D can serve");
                    self.plan = Some(plan);
                    plan
                }
            };
            let view = ChunkView {
                chunks: self.query.chunks,
                layout: &arch.layout,
                reg: self.query.reg,
                chunk,
            };
            // SAFETY: `chunk` belongs to `arch`, which the state matched
            // against D's requirements; `plan` was resolved from its layout.
            if let Some(columns) = unsafe { D::fetch(&view, plan, &mut self.query.grant) } {
                return Some(columns);
            }
        }
    }
}
