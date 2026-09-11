use crate::registry::{ComponentId, Registry};
use crate::storage::chunks::ChunkId;
use crate::storage::layout::{ArchetypeLayout, LayoutError};
use std::collections::HashMap;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub(crate) struct ArchetypeId(pub(crate) u32);

/// One archetype: the storage for all entities sharing a component signature.
pub(crate) struct Archetype {
    /// Byte map shared by every chunk of this archetype.
    pub layout: ArchetypeLayout,
    /// The chunks holding this archetype's entities.
    pub chunks: Vec<ChunkId>,
    /// Chunk to try first when inserting a row; `None` means none is known
    /// to have free rows.
    pub non_full: Option<ChunkId>,
    /// Cached target archetype for "this signature plus that component".
    /// Filled lazily; absence means the target has not been looked up yet.
    pub edge_add: HashMap<ComponentId, ArchetypeId>,
    /// Cached target archetype for "this signature minus that component".
    /// Filled lazily; absence means the target has not been looked up yet.
    pub edge_remove: HashMap<ComponentId, ArchetypeId>,
}

impl Archetype {
    /// Creates an empty archetype for the signature `ids`, which must be
    /// sorted ascending without duplicates and registered in `reg`.
    ///
    /// # Errors
    ///
    /// Propagates [`LayoutError`] when the signature has no valid chunk layout.
    pub fn new(ids: &[ComponentId], reg: &Registry) -> Result<Self, LayoutError> {
        let layout = ArchetypeLayout::new(ids, reg)?;
        Ok(Self {
            layout,
            chunks: Vec::new(),
            non_full: None,
            edge_add: HashMap::new(),
            edge_remove: HashMap::new(),
        })
    }

    /// The component ids of this archetype, sorted ascending.
    pub fn signature(&self) -> &[ComponentId] {
        &self.layout.components
    }
}

/// Index of every archetype in a world, keyed by signature.
#[derive(Default)]
pub(crate) struct Archetypes {
    list: Vec<Archetype>,
    by_signature: HashMap<Box<[ComponentId]>, ArchetypeId>,
    generation: u32,
}

impl Archetypes {
    /// Returns the archetype with signature `ids`, creating it if absent.
    ///
    /// `ids` must be sorted ascending without duplicates and registered in
    /// `reg`. Creates at most the one archetype asked for, and bumps
    /// [`generation`](Self::generation) only in that case.
    ///
    /// # Errors
    ///
    /// Propagates [`LayoutError`] when the signature has no valid chunk
    /// layout; the index is left unchanged.
    pub fn get_or_create(
        &mut self,
        ids: &[ComponentId],
        reg: &Registry,
    ) -> Result<ArchetypeId, LayoutError> {
        if let Some(id) = self.by_signature.get(ids) {
            return Ok(*id);
        }

        let archetype = Archetype::new(ids, reg)?;
        let id = ArchetypeId(u32::try_from(self.list.len()).expect("archetype id space exhausted"));
        self.list.push(archetype);
        self.by_signature.insert(ids.into(), id);
        self.generation += 1;
        Ok(id)
    }

    pub fn get(&self, id: ArchetypeId) -> &Archetype {
        &self.list[id.0 as usize]
    }

    pub fn get_mut(&mut self, id: ArchetypeId) -> &mut Archetype {
        &mut self.list[id.0 as usize]
    }

    pub fn get_pair_mut(
        &mut self,
        first_id: ArchetypeId,
        second_id: ArchetypeId,
    ) -> (&mut Archetype, &mut Archetype) {
        assert_ne!(first_id, second_id, "archetype ids must be distinct");
        let (low, high) = (first_id.min(second_id), first_id.max(second_id));
        let (head, tail) = self.list.split_at_mut(high.0 as usize);
        let (low_ref, high_ref) = (&mut head[low.0 as usize], &mut tail[0]);
        if first_id < second_id {
            (low_ref, high_ref)
        } else {
            (high_ref, low_ref)
        }
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// Increases exactly when an archetype is created. Compare a remembered
    /// value against the current one to detect archetypes added since.
    #[cfg(test)]
    pub fn generation(&self) -> u32 {
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{Component, ComponentKey, StorageClass};
    use crate::registry::{ComponentId, Registry};
    use crate::storage::layout::LayoutError;

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("archetype::tests::", stringify!($name)));
            }
        };
    }

    struct A(#[allow(dead_code)] u32);
    component!(A);
    struct B(#[allow(dead_code)] u64);
    component!(B);
    struct C(#[allow(dead_code)] u8);
    component!(C);
    struct D(#[allow(dead_code)] [u64; 2]);
    component!(D);
    struct E(#[allow(dead_code)] u16);
    component!(E);
    struct Zst;
    impl Component for Zst {
        const KEY: ComponentKey = ComponentKey::from_path("archetype::tests::Zst");
        const STORAGE: StorageClass = StorageClass::Tag;
    }
    struct Huge(#[allow(dead_code)] [u8; 70_000]);
    component!(Huge);

    fn setup() -> (Registry, [ComponentId; 5]) {
        let mut reg = Registry::new();
        let ids = [
            reg.register::<A>(),
            reg.register::<B>(),
            reg.register::<C>(),
            reg.register::<D>(),
            reg.register::<E>(),
        ];
        (reg, ids)
    }

    // ------------------------------------------------------------ get_or_create

    #[test]
    fn same_signature_yields_same_archetype() {
        let (reg, [a, b, ..]) = setup();
        let mut arch = Archetypes::default();
        let first = arch.get_or_create(&[a, b], &reg).unwrap();
        let second = arch.get_or_create(&[a, b], &reg).unwrap();
        assert_eq!(first, second);
        assert_eq!(arch.len(), 1, "lookup must not create a duplicate");
    }

    #[test]
    fn different_signatures_yield_different_archetypes() {
        let (reg, [a, b, c, ..]) = setup();
        let mut arch = Archetypes::default();
        let ab = arch.get_or_create(&[a, b], &reg).unwrap();
        let ac = arch.get_or_create(&[a, c], &reg).unwrap();
        let abc = arch.get_or_create(&[a, b, c], &reg).unwrap();
        let empty = arch.get_or_create(&[], &reg).unwrap();
        let ids = [ab, ac, abc, empty];
        assert_eq!(arch.len(), 4);
        for (i, x) in ids.iter().enumerate() {
            for y in &ids[i + 1..] {
                assert_ne!(x, y);
            }
        }
    }

    #[test]
    fn creating_an_n_component_archetype_creates_exactly_one() {
        // Regression guard: the signature index is a direct map, never a
        // subset lattice. Five components must yield one archetype, not 2^5.
        let (reg, ids) = setup();
        let mut arch = Archetypes::default();
        arch.get_or_create(&ids, &reg).unwrap();
        assert_eq!(arch.len(), 1);
    }

    #[test]
    fn zst_membership_distinguishes_signatures() {
        let (mut reg, [a, ..]) = setup();
        let zst = reg.register::<Zst>();
        let mut arch = Archetypes::default();
        let plain = arch.get_or_create(&[a], &reg).unwrap();
        let tagged = arch.get_or_create(&[a, zst], &reg).unwrap();
        assert_ne!(plain, tagged, "tag components are part of the identity");
        assert_eq!(arch.len(), 2);
    }

    #[test]
    fn layout_error_propagates_and_creates_nothing() {
        let (mut reg, _) = setup();
        let huge = reg.register::<Huge>();
        let mut arch = Archetypes::default();
        assert!(matches!(
            arch.get_or_create(&[huge], &reg),
            Err(LayoutError::EntityTooLarge)
        ));
        assert_eq!(arch.len(), 0, "a failed creation must not be recorded");
        assert_eq!(arch.generation(), 0);
    }

    // ---------------------------------------------------------------- accessors

    #[test]
    fn new_archetype_starts_empty_with_its_layout() {
        let (reg, [a, b, ..]) = setup();
        let mut arch = Archetypes::default();
        let id = arch.get_or_create(&[a, b], &reg).unwrap();
        let at = arch.get(id);
        assert_eq!(at.signature(), &[a, b]);
        assert!(at.layout.capacity >= 1);
        assert!(at.chunks.is_empty());
        assert!(at.non_full.is_none());
        assert!(at.edge_add.is_empty());
        assert!(at.edge_remove.is_empty());
    }

    #[test]
    fn get_mut_changes_persist() {
        let (reg, [a, b, ..]) = setup();
        let mut arch = Archetypes::default();
        let ab = arch.get_or_create(&[a, b], &reg).unwrap();
        let bee = arch.get_or_create(&[b], &reg).unwrap();

        arch.get_mut(ab).edge_remove.insert(a, bee);
        assert_eq!(arch.get(ab).edge_remove.get(&a), Some(&bee));
        assert!(
            arch.get(bee).edge_remove.is_empty(),
            "edges are per-archetype"
        );
    }

    // --------------------------------------------------------------- generation

    #[test]
    fn generation_bumps_only_on_creation() {
        let (reg, [a, b, ..]) = setup();
        let mut arch = Archetypes::default();
        assert_eq!(arch.generation(), 0);

        arch.get_or_create(&[a], &reg).unwrap();
        let after_first = arch.generation();
        arch.get_or_create(&[a, b], &reg).unwrap();
        let after_second = arch.generation();
        assert!(after_first > 0);
        assert!(after_second > after_first);

        arch.get_or_create(&[a], &reg).unwrap();
        arch.get_or_create(&[a, b], &reg).unwrap();
        assert_eq!(
            arch.generation(),
            after_second,
            "lookups must not bump the generation"
        );
    }

    #[test]
    fn get_pair_mut_returns_two_distinct_archetypes() {
        let (reg, [a, b, ..]) = setup();
        let mut arch = Archetypes::default();
        let first = arch.get_or_create(&[a], &reg).unwrap();
        let second = arch.get_or_create(&[a, b], &reg).unwrap();

        // both orders must work
        for (x, y) in [(first, second), (second, first)] {
            let (ax, ay) = arch.get_pair_mut(x, y);
            assert_eq!(ax.signature().len(), if x == first { 1 } else { 2 });
            assert_eq!(ay.signature().len(), if y == first { 1 } else { 2 });
            ax.non_full = None;
            ay.non_full = None; // proves both are usable mutably at once
        }
    }

    #[test]
    #[should_panic(expected = "distinct")]
    #[cfg(debug_assertions)]
    fn get_pair_mut_rejects_identical_ids() {
        let (reg, [a, ..]) = setup();
        let mut arch = Archetypes::default();
        let id = arch.get_or_create(&[a], &reg).unwrap();
        let _ = arch.get_pair_mut(id, id);
    }

    #[test]
    fn empty_index() {
        let arch = Archetypes::default();
        assert_eq!(arch.len(), 0);
        assert!(arch.is_empty());
        assert_eq!(arch.generation(), 0);
    }
}
