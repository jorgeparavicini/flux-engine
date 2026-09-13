use crate::access::AccessList;
use crate::grant::AccessGrant;
use crate::registry::Registry;
use crate::relation::Relation;
use crate::storage::chunks::{ChunkId, Chunks};
use crate::storage::layout::ArchetypeLayout;
use crate::storage::ops;
use crate::{Component, ComponentId, ComponentKey, Entity};
use std::marker::PhantomData;

/// One chunk of one archetype, as a query sees it.
#[derive(Copy, Clone)]
pub struct ChunkView<'w> {
    pub(crate) chunks: &'w Chunks,
    pub(crate) layout: &'w ArchetypeLayout,
    pub(crate) reg: &'w Registry,
    pub(crate) chunk: ChunkId,
}

impl ChunkView<'_> {
    /// Occupied rows.
    pub fn len(&self) -> usize {
        self.chunks.len(self.chunk) as usize
    }

    /// The signature position of the component registered under `key`, if
    /// this chunk's archetype has it.
    pub fn column_index(&self, key: ComponentKey) -> Option<usize> {
        let id = self.reg.lookup(key)?;
        self.layout.components.binary_search(&id).ok()
    }
}

/// What a query fetches from each matched chunk.
///
/// # Safety
///
/// `ACCESS` must declare every component `columns` touches, with `write`
/// for every mutable access. `matches` must return true only for
/// signatures `columns` can fully serve.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be queried",
    note = "query components by reference — `&T` or `&mut T` — or use `Entity`, `Option<&T>`, or a tuple of these"
)]
pub unsafe trait QueryData {
    /// Access this data declares, checked for self-conflicts at compile
    /// time by the query machinery.
    const ACCESS: AccessList;

    /// The borrowed columns of one chunk.
    type Columns<'w>;

    /// Resolved column positions for one archetype, shared by all of its
    /// chunks.
    type Plan: Copy;

    type Item<'a>;

    /// Whether an archetype with this signature is visited at all.
    fn matches(signature: &[ComponentId], reg: &Registry) -> bool;

    /// Resolves the fetch plan for an archetype. None when the archetype
    /// cannot serve this data; never None for an archetype `matches` accepts.
    fn plan(layout: &ArchetypeLayout, reg: &Registry) -> Option<Self::Plan>;

    /// Fetches the columns through the grant, using a plan resolved for this
    /// chunk's archetype. None when the grant denies an access (a spent
    /// claim included). Claims last for the grant's lifetime, so a grant
    /// serves at most one mutable fetch per (chunk, component).
    ///
    /// # Safety
    ///
    /// `view.chunk` belongs to an archetype matching `Self` per `matches`,
    /// and `plan` was resolved from that archetype's layout.
    unsafe fn fetch<'w>(
        view: &ChunkView<'w>,
        plan: Self::Plan,
        grant: &mut AccessGrant,
    ) -> Option<Self::Columns<'w>>;

    /// Plans and fetches in one step.
    ///
    /// # Safety
    ///
    /// As [`fetch`](Self::fetch).
    unsafe fn columns<'w>(
        view: &ChunkView<'w>,
        grant: &mut AccessGrant,
    ) -> Option<Self::Columns<'w>> {
        let plan = Self::plan(view.layout, view.reg)?;
        unsafe { Self::fetch(view, plan, grant) }
    }

    fn row<'w, 'a>(columns: &'a mut Self::Columns<'w>, row: usize) -> Self::Item<'a>;

    /// Combined enabled mask over `view` for this data's toggleable
    /// components. `None` means every row is enabled; otherwise a set bit
    /// marks an enabled row. Masks for multiple components are AND-ed.
    #[allow(unused_variables)]
    fn enabled_mask(view: &ChunkView<'_>) -> Option<Vec<u64>> {
        None
    }
}

/// The enabled mask for `T`'s column in `view`, present only when `T` is
/// toggleable and the chunk has materialized a mask.
fn toggleable_mask<T: Component>(view: &ChunkView<'_>) -> Option<Vec<u64>> {
    if !T::TOGGLEABLE {
        return None;
    }
    let column = view.column_index(T::KEY)?;
    view.chunks
        .enabled_mask(view.chunk, column)
        .map(<[u64]>::to_vec)
}

/// AND two optional masks, treating `None` as the all-enabled identity.
fn and_masks(a: Option<Vec<u64>>, b: Option<Vec<u64>>) -> Option<Vec<u64>> {
    match (a, b) {
        (None, m) | (m, None) => m,
        (Some(mut a), Some(b)) => {
            for (x, y) in a.iter_mut().zip(b.iter()) {
                *x &= *y;
            }
            Some(a)
        }
    }
}

/// The signature position of `key`'s column in `layout`, if present.
fn column_of(layout: &ArchetypeLayout, reg: &Registry, key: ComponentKey) -> Option<usize> {
    let id = reg.lookup(key)?;
    layout.components.binary_search(&id).ok()
}

fn has<T: Component>(signature: &[ComponentId], reg: &Registry) -> bool {
    reg.lookup(T::KEY)
        .is_some_and(|id| signature.binary_search(&id).is_ok())
}

unsafe impl<T: Component> QueryData for &T {
    const ACCESS: AccessList = AccessList::read(T::KEY);
    type Columns<'w> = &'w [T];
    type Plan = usize;
    type Item<'a> = &'a T;

    fn matches(signature: &[ComponentId], reg: &Registry) -> bool {
        has::<T>(signature, reg)
    }

    fn plan(layout: &ArchetypeLayout, reg: &Registry) -> Option<Self::Plan> {
        column_of(layout, reg, T::KEY)
    }

    unsafe fn fetch<'w>(
        view: &ChunkView<'w>,
        plan: Self::Plan,
        grant: &mut AccessGrant,
    ) -> Option<Self::Columns<'w>> {
        unsafe { ops::column::<T>(view.chunks, view.layout, view.reg, view.chunk, plan, grant) }
    }

    fn row<'w, 'a>(columns: &'a mut Self::Columns<'w>, row: usize) -> Self::Item<'a> {
        &columns[row]
    }

    fn enabled_mask(view: &ChunkView<'_>) -> Option<Vec<u64>> {
        toggleable_mask::<T>(view)
    }
}

unsafe impl<T: Component> QueryData for &mut T {
    const ACCESS: AccessList = AccessList::write(T::KEY);
    type Columns<'w> = &'w mut [T];
    type Plan = usize;
    type Item<'a> = &'a mut T;

    fn matches(signature: &[ComponentId], reg: &Registry) -> bool {
        has::<T>(signature, reg)
    }

    fn plan(layout: &ArchetypeLayout, reg: &Registry) -> Option<Self::Plan> {
        column_of(layout, reg, T::KEY)
    }

    unsafe fn fetch<'w>(
        view: &ChunkView<'w>,
        plan: Self::Plan,
        grant: &mut AccessGrant,
    ) -> Option<Self::Columns<'w>> {
        unsafe { ops::column_mut(view.chunks, view.layout, view.reg, view.chunk, plan, grant) }
    }

    fn row<'w, 'a>(columns: &'a mut Self::Columns<'w>, row: usize) -> Self::Item<'a> {
        &mut columns[row]
    }

    fn enabled_mask(view: &ChunkView<'_>) -> Option<Vec<u64>> {
        toggleable_mask::<T>(view)
    }
}

/// Yields the entity ids of each row; declares no component access.
unsafe impl QueryData for Entity {
    const ACCESS: AccessList = AccessList::EMPTY;
    type Columns<'w> = &'w [Entity];
    type Plan = ();
    type Item<'a> = Entity;

    fn matches(_signature: &[ComponentId], _reg: &Registry) -> bool {
        true
    }

    fn plan(_layout: &ArchetypeLayout, _reg: &Registry) -> Option<Self::Plan> {
        Some(())
    }

    unsafe fn fetch<'w>(
        view: &ChunkView<'w>,
        _plan: Self::Plan,
        _grant: &mut AccessGrant,
    ) -> Option<Self::Columns<'w>> {
        unsafe { Some(ops::entity_column(view.chunks, view.layout, view.chunk)) }
    }

    fn row<'w, 'a>(columns: &'a mut Self::Columns<'w>, row: usize) -> Self::Item<'a> {
        columns[row]
    }
}

/// Yields the target of each row's `R` relation.
///
/// Matches archetypes that carry an `(R, *)` pair. The target is fixed for a
/// whole archetype, so every row in a chunk yields the same entity; a query
/// like `(Entity, Related<ChildOf>)` walks every child paired with its parent.
pub struct Related<R: Relation>(PhantomData<R>);

unsafe impl<R: Relation> QueryData for Related<R> {
    const ACCESS: AccessList = AccessList::EMPTY;
    type Columns<'w> = Entity;
    type Plan = Entity;
    type Item<'a> = Entity;

    fn matches(signature: &[ComponentId], reg: &Registry) -> bool {
        signature
            .iter()
            .any(|&id| reg.info(id).relation.is_some_and(|r| r.relation == R::KEY))
    }

    fn plan(layout: &ArchetypeLayout, reg: &Registry) -> Option<Self::Plan> {
        layout.components.iter().find_map(|&id| {
            let rel = reg.info(id).relation?;
            (rel.relation == R::KEY).then_some(rel.target)
        })
    }

    unsafe fn fetch<'w>(
        _view: &ChunkView<'w>,
        plan: Self::Plan,
        _grant: &mut AccessGrant,
    ) -> Option<Self::Columns<'w>> {
        Some(plan)
    }

    fn row<'w, 'a>(columns: &'a mut Self::Columns<'w>, _row: usize) -> Self::Item<'a> {
        *columns
    }
}

/// Matches every archetype; resolves to `Some`/`None` per chunk, never per
/// entity.
unsafe impl<T: Component> QueryData for Option<&T> {
    const ACCESS: AccessList = AccessList::read(T::KEY);
    type Columns<'w> = Option<&'w [T]>;
    type Plan = Option<usize>;
    type Item<'a> = Option<&'a T>;

    fn matches(_signature: &[ComponentId], _reg: &Registry) -> bool {
        true
    }

    fn plan(layout: &ArchetypeLayout, reg: &Registry) -> Option<Self::Plan> {
        Some(column_of(layout, reg, T::KEY))
    }

    unsafe fn fetch<'w>(
        view: &ChunkView<'w>,
        plan: Self::Plan,
        grant: &mut AccessGrant,
    ) -> Option<Self::Columns<'w>> {
        match plan {
            Some(column) => {
                let col = unsafe {
                    ops::column::<T>(
                        view.chunks,
                        view.layout,
                        view.reg,
                        view.chunk,
                        column,
                        grant,
                    )
                }?;
                Some(Some(col))
            }
            None => Some(None),
        }
    }

    fn row<'w, 'a>(columns: &'a mut Self::Columns<'w>, row: usize) -> Self::Item<'a> {
        columns.map(|col| &col[row])
    }
}

macro_rules! tuple_query_data {
    ($($t:ident),+) => {
        unsafe impl<$($t: QueryData),+> QueryData for ($($t,)+) {
            const ACCESS: AccessList = {
                let list = AccessList::EMPTY;
                $( let list = list.concat($t::ACCESS); )+
                list
            };

            type Columns<'w> = ($($t::Columns<'w>,)+);

            type Plan = ($($t::Plan,)+);

            type Item<'a> = ($($t::Item<'a>,)+);

            fn matches(signature: &[ComponentId], reg: &Registry) -> bool {
                $( if !$t::matches(signature, reg) { return false; } )+
                true
            }

            fn plan(layout: &ArchetypeLayout, reg: &Registry) -> Option<Self::Plan> {
                Some(($( $t::plan(layout, reg)?, )+))
            }

            #[allow(non_snake_case)]
            unsafe fn fetch<'w>(view: &ChunkView<'w>, plan: Self::Plan, grant: &mut AccessGrant) -> Option<Self::Columns<'w>> {
                let ($($t,)+) = plan;
                Some(($( unsafe { $t::fetch(view, $t, grant) }?, )+))
            }

            #[allow(non_snake_case)]
            fn row<'w, 'a>(columns: &'a mut Self::Columns<'w>, row: usize) -> Self::Item<'a> {
                let ($($t,)+) = columns;
                ($($t::row($t, row),)+)
            }

            fn enabled_mask(view: &ChunkView<'_>) -> Option<Vec<u64>> {
                let mask = None;
                $( let mask = and_masks(mask, $t::enabled_mask(view)); )+
                mask
            }
        }
    }
}

tuple_query_data!(T1);
tuple_query_data!(T1, T2);
tuple_query_data!(T1, T2, T3);
tuple_query_data!(T1, T2, T3, T4);
tuple_query_data!(T1, T2, T3, T4, T5);
tuple_query_data!(T1, T2, T3, T4, T5, T6);
tuple_query_data!(T1, T2, T3, T4, T5, T6, T7);
tuple_query_data!(T1, T2, T3, T4, T5, T6, T7, T8);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::AccessList;
    use crate::component::{Component, ComponentKey, StorageClass};
    use crate::entity::{Entities, Entity};
    use crate::grant::AccessGrant;
    use crate::registry::Registry;
    use crate::storage::alloc::ChunkAlloc;
    use crate::storage::archetype::{Archetype, ArchetypeId};
    use crate::storage::chunks::{ChunkId, Chunks};
    use crate::storage::ops;

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("query::data::tests::", stringify!($name)));
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
    struct Absent(u8);
    component!(Absent);
    struct Marker;
    impl Component for Marker {
        const KEY: ComponentKey = ComponentKey::from_path("query::data::tests::Marker");
        const STORAGE: StorageClass = StorageClass::Tag;
    }

    /// One archetype {A, B, Marker} with `n` rows of (A(i), B(i)).
    struct Bench {
        reg: Registry,
        chunks: Chunks,
        alloc: ChunkAlloc,
        arch: Archetype,
        chunk: ChunkId,
        entities: Vec<Entity>,
    }

    impl Bench {
        fn new(n: u64) -> Self {
            let mut reg = Registry::new();
            let a = reg.register::<A>();
            let b = reg.register::<B>();
            let m = reg.register::<Marker>();
            let mut ids = vec![a, b, m];
            ids.sort();
            let mut arch = Archetype::new(&ids, &reg).unwrap();
            let mut chunks = Chunks::default();
            let mut alloc = ChunkAlloc::default();
            let mut slots = Entities::new();
            let mut chunk = ChunkId(0);
            let sig = arch.signature().to_vec();
            let (a_col, b_col) = (
                sig.iter().position(|c| *c == a).unwrap(),
                sig.iter().position(|c| *c == b).unwrap(),
            );
            let mut entities = Vec::new();
            for i in 0..n {
                let e = slots.alloc();
                let (c, row) = unsafe {
                    ops::alloc_row(&mut arch, ArchetypeId(0), &mut chunks, &mut alloc, e)
                };
                chunk = c;
                unsafe {
                    let av = A(i);
                    ops::write_component(
                        &chunks,
                        &arch.layout,
                        &reg,
                        c,
                        a_col,
                        row,
                        (&raw const av).cast(),
                    );
                    let bv = B(i as u16);
                    ops::write_component(
                        &chunks,
                        &arch.layout,
                        &reg,
                        c,
                        b_col,
                        row,
                        (&raw const bv).cast(),
                    );
                }
                entities.push(e);
            }
            Self {
                reg,
                chunks,
                alloc,
                arch,
                chunk,
                entities,
            }
        }

        fn view(&self) -> ChunkView<'_> {
            ChunkView {
                chunks: &self.chunks,
                layout: &self.arch.layout,
                reg: &self.reg,
                chunk: self.chunk,
            }
        }

        fn grant_for<D: QueryData>(&self) -> AccessGrant {
            AccessGrant::new(D::ACCESS)
        }
    }

    impl Drop for Bench {
        fn drop(&mut self) {
            while self.chunks.len(self.chunk) > 0 {
                let row = self.chunks.len(self.chunk) - 1;
                unsafe {
                    ops::swap_remove_row(
                        &mut self.arch,
                        &mut self.chunks,
                        &mut self.alloc,
                        &self.reg,
                        self.chunk,
                        row,
                        true,
                    )
                };
            }
        }
    }

    // ------------------------------------------------------------------ access

    #[test]
    fn access_declarations_match_the_shape() {
        let read = <&A as QueryData>::ACCESS;
        assert_eq!(read.len(), 1);
        assert!(!read.terms()[0].write);
        assert_eq!(read.terms()[0].key, A::KEY);

        let write = <&mut A as QueryData>::ACCESS;
        assert!(write.terms()[0].write);

        assert!(<Entity as QueryData>::ACCESS.is_empty());
        assert_eq!(<Option<&A> as QueryData>::ACCESS.terms()[0].key, A::KEY);

        let tuple = <(&A, &mut B) as QueryData>::ACCESS;
        assert_eq!(tuple.len(), 2);
        assert_eq!(tuple.terms()[0].key, A::KEY);
        assert!(!tuple.terms()[0].write);
        assert_eq!(tuple.terms()[1].key, B::KEY);
        assert!(tuple.terms()[1].write);
    }

    #[test]
    fn aliasing_tuples_are_detectable_at_compile_time() {
        const CONFLICT: bool = <(&A, &mut A) as QueryData>::ACCESS.self_conflicting();
        const OK: bool = <(&A, &mut B) as QueryData>::ACCESS.self_conflicting();
        const _: () = assert!(CONFLICT);
        const _: () = assert!(!OK);
    }

    // ----------------------------------------------------------------- matches

    #[test]
    fn matches_requires_every_named_component() {
        let bench = Bench::new(1);
        let sig = bench.arch.signature();
        assert!(<&A as QueryData>::matches(sig, &bench.reg));
        assert!(<(&A, &mut B) as QueryData>::matches(sig, &bench.reg));
        assert!(
            <&Marker as QueryData>::matches(sig, &bench.reg),
            "tag components match"
        );
        assert!(!<&Absent as QueryData>::matches(sig, &bench.reg));
        assert!(
            !<(&A, &Absent) as QueryData>::matches(sig, &bench.reg),
            "one missing member fails the tuple"
        );
    }

    #[test]
    fn entity_and_option_match_everything() {
        let bench = Bench::new(1);
        let sig = bench.arch.signature();
        assert!(<Entity as QueryData>::matches(sig, &bench.reg));
        assert!(<Option<&Absent> as QueryData>::matches(sig, &bench.reg));
        assert!(<(Entity, Option<&Absent>) as QueryData>::matches(
            sig, &bench.reg
        ));
    }

    #[test]
    fn matches_of_unregistered_component_is_false_without_registering() {
        struct NeverSeen(#[allow(dead_code)] u8);
        impl Component for NeverSeen {
            const KEY: ComponentKey = ComponentKey::from_path("query::data::tests::NeverSeen");
        }
        let bench = Bench::new(1);
        let before = bench.reg.len();
        assert!(!<&NeverSeen as QueryData>::matches(
            bench.arch.signature(),
            &bench.reg
        ));
        assert_eq!(bench.reg.len(), before, "matching must not register types");
    }

    // ------------------------------------------------------------------- fetch

    #[test]
    fn fetching_reads_the_stored_values() {
        let bench = Bench::new(5);
        let mut grant = bench.grant_for::<(&A, &B)>();
        let (a, b) = unsafe { <(&A, &B) as QueryData>::columns(&bench.view(), &mut grant) }
            .expect("grant covers the tuple");
        assert_eq!(a.len(), 5);
        assert_eq!(b.len(), 5);
        assert_eq!(a[3], A(3));
        assert_eq!(b[4], B(4));
    }

    #[test]
    fn mutable_fetch_writes_land_in_storage() {
        let bench = Bench::new(3);
        let mut grant = bench.grant_for::<(&mut A, &B)>();
        {
            let (a, b) = unsafe { <(&mut A, &B) as QueryData>::columns(&bench.view(), &mut grant) }
                .expect("granted");
            for (av, bv) in a.iter_mut().zip(b.iter()) {
                av.0 += bv.0 as u64 * 100;
            }
        }
        grant.release_chunk(bench.chunk);
        let mut read = bench.grant_for::<&A>();
        let a = unsafe { <&A as QueryData>::columns(&bench.view(), &mut read) }.unwrap();
        assert_eq!(a[2], A(2 + 200));
    }

    #[test]
    fn entity_columns_yield_the_ids() {
        let bench = Bench::new(4);
        let mut grant = bench.grant_for::<(Entity, &A)>();
        let (entities, a) =
            unsafe { <(Entity, &A) as QueryData>::columns(&bench.view(), &mut grant) }
                .expect("granted");
        assert_eq!(entities.len(), 4);
        assert_eq!(a.len(), 4);
        assert_eq!(
            entities,
            &bench.entities[..],
            "entity column matches spawn order"
        );
    }

    #[test]
    fn option_resolves_per_chunk() {
        let bench = Bench::new(2);
        let mut grant =
            AccessGrant::new(AccessList::read(A::KEY).concat(AccessList::read(Absent::KEY)));
        let (a, missing) =
            unsafe { <(&A, Option<&Absent>) as QueryData>::columns(&bench.view(), &mut grant) }
                .expect("tuple fetch succeeds even with a missing optional member");
        assert_eq!(a.len(), 2);
        assert!(
            missing.is_none(),
            "absent component resolves to None for the whole chunk"
        );
    }

    #[test]
    fn option_member_denied_by_the_grant_fails_the_whole_fetch() {
        // A grant denial is a failure, not "component absent": it must not
        // masquerade as Some(None).
        let bench = Bench::new(1);
        let mut no_b = AccessGrant::new(AccessList::read(A::KEY));
        let denied = unsafe { <(&A, Option<&B>) as QueryData>::columns(&bench.view(), &mut no_b) };
        assert!(denied.is_none());
    }

    #[test]
    fn zst_columns_fetch_as_full_length_slices() {
        let bench = Bench::new(3);
        let mut grant = bench.grant_for::<&Marker>();
        let markers =
            unsafe { <&Marker as QueryData>::columns(&bench.view(), &mut grant) }.expect("granted");
        assert_eq!(markers.len(), 3);
    }

    #[test]
    fn fetch_without_permission_is_denied() {
        let bench = Bench::new(1);
        let mut wrong = AccessGrant::new(AccessList::read(B::KEY));
        let denied = unsafe { <&A as QueryData>::columns(&bench.view(), &mut wrong) };
        assert!(denied.is_none());

        let mut read_only = AccessGrant::new(AccessList::read(A::KEY));
        let denied = unsafe { <&mut A as QueryData>::columns(&bench.view(), &mut read_only) };
        assert!(
            denied.is_none(),
            "mutable fetch through a read grant is denied"
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    fn second_mutable_fetch_of_a_chunk_is_denied_until_release() {
        let bench = Bench::new(1);
        let mut grant = bench.grant_for::<&mut A>();
        let first = unsafe { <&mut A as QueryData>::columns(&bench.view(), &mut grant) };
        assert!(first.is_some());
        let second = unsafe { <&mut A as QueryData>::columns(&bench.view(), &mut grant) };
        assert!(second.is_none(), "claim already spent for this chunk");
        grant.release_chunk(bench.chunk);
        assert!(unsafe { <&mut A as QueryData>::columns(&bench.view(), &mut grant) }.is_some());
    }
}
