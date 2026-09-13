use crate::storage::alloc::ChunkAlloc;
use crate::storage::archetype::ArchetypeId;
use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

/// Dense index of a chunk within a world. Ids are recycled after
/// [`Chunks::destroy`]; holding one across a destroy is invalid.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub(crate) struct ChunkId(pub(crate) u32);

/// Metadata for every chunk in a world, indexed by [`ChunkId`].
///
/// Owns no allocator: `create`/`destroy` borrow the [`ChunkAlloc`], and the
/// owner of both is responsible for destroying all live chunks before
/// dropping the allocator.
#[derive(Default)]
pub(crate) struct Chunks {
    /// Pointer to the chunk's 16 KiB block; valid while the chunk is live.
    base: Vec<NonNull<u8>>,
    /// Occupied rows. Rows `0..len` are initialized in every column.
    len: Vec<u16>,
    /// The archetype whose [`ArchetypeLayout`] describes this block.
    ///
    /// [`ArchetypeLayout`]: crate::storage::layout::ArchetypeLayout
    archetype: Vec<ArchetypeId>,
    /// Per column, the world version of the last mutable access to that
    /// column in this chunk. 0 means "never written since this id was bound".
    write_versions: Vec<Box<[AtomicU64]>>,
    added_versions: Vec<Box<[AtomicU64]>>,
    /// Bumped on every structural change: rows added/removed/moved and chunk
    /// create/destroy. Never reset, including across id recycling: a value
    /// observed for a [`ChunkId`] is stale iff the current value is greater.
    order_version: Vec<u64>,
    /// Dead ids awaiting reuse.
    free: Vec<ChunkId>,
    /// Per (chunk, toggleable column) enabled bitmasks, materialized on the
    /// first disable. Absent means every row is enabled. A set bit is enabled.
    enabled: HashMap<(ChunkId, usize), Vec<u64>>,
}

// SAFETY: shared (`&Chunks`) access exposes only atomic version stamps
// (thread-safe) and immutable reads of the base pointers and layout metadata.
// Structural mutation — create/destroy/set_len, which move the pointers and
// lengths — requires `&mut Chunks`, so it cannot occur while the value is
// shared. Column data reached through those base pointers is handed out as
// grant-checked slices whose disjointness the scheduler proves before sharing.
unsafe impl Sync for Chunks {}
unsafe impl Send for Chunks {}

impl Chunks {
    /// Allocates a block and binds a chunk id to it: `len` 0, all `columns`
    /// write stamps 0, `order_version` strictly greater than any earlier
    /// value this id has had. Recycles destroyed ids before growing.
    pub fn create(
        &mut self,
        alloc: &mut ChunkAlloc,
        archetype: ArchetypeId,
        columns: usize,
    ) -> ChunkId {
        let block = alloc.alloc();

        if let Some(free) = self.free.pop() {
            let id = free.0 as usize;
            self.base[id] = block;
            self.len[id] = 0;
            self.archetype[id] = archetype;
            self.write_versions[id] = Self::new_stamps(columns);
            self.added_versions[id] = Self::new_stamps(columns);
            self.order_version[id] += 1;
            free
        } else {
            let id = ChunkId(u32::try_from(self.base.len()).expect("too many chunks"));
            self.base.push(block);
            self.len.push(0);
            self.archetype.push(archetype);
            self.write_versions.push(Self::new_stamps(columns));
            self.added_versions.push(Self::new_stamps(columns));
            self.order_version.push(1);
            id
        }
    }

    /// Returns the chunk's block to the allocator and its id to the free
    /// list. The chunk must be empty: rows are dropped by storage ops, not
    /// here.
    pub fn destroy(&mut self, alloc: &mut ChunkAlloc, id: ChunkId) {
        let i = id.0 as usize;
        debug_assert!(
            self.len[i] == 0,
            "destroyed chunk {id:?} still holds {} rows (must be empty; rows are dropped by storage ops)",
            self.len[i]
        );
        debug_assert!(
            !self.free.contains(&id),
            "double destroy of chunk {id:?}: id is already on the free list"
        );
        self.order_version[i] += 1;
        self.enabled.retain(|&(chunk, _), _| chunk != id);
        unsafe { alloc.dealloc(self.base[i]) };
        self.free.push(id);
    }

    pub fn base(&self, id: ChunkId) -> NonNull<u8> {
        self.base[id.0 as usize]
    }

    pub fn len(&self, id: ChunkId) -> u16 {
        self.len[id.0 as usize]
    }

    pub fn set_len(&mut self, id: ChunkId, len: u16) {
        self.len[id.0 as usize] = len;
    }

    pub fn archetype(&self, id: ChunkId) -> ArchetypeId {
        self.archetype[id.0 as usize]
    }

    #[cfg(test)]
    pub fn order_version(&self, id: ChunkId) -> u64 {
        self.order_version[id.0 as usize]
    }

    /// Records a structural change; returns the new version.
    pub fn bump_order_version(&mut self, id: ChunkId) -> u64 {
        self.order_version[id.0 as usize] += 1;
        self.order_version[id.0 as usize]
    }

    /// World version of the last mutable access to a column, where `column`
    /// is the component's position in the archetype's signature.
    pub fn write_version(&self, id: ChunkId, column: usize) -> u64 {
        self.write_versions[id.0 as usize][column].load(Ordering::Relaxed)
    }

    pub fn added_version(&self, id: ChunkId, column: usize) -> u64 {
        self.added_versions[id.0 as usize][column].load(Ordering::Relaxed)
    }

    /// Records a mutable access to a column at the given world version, where
    /// `column` is the component's position in the archetype's signature.
    pub fn stamp_write_version(&self, id: ChunkId, column: usize, version: u64) {
        self.write_versions[id.0 as usize][column].store(version, Ordering::Relaxed);
    }

    pub fn stamp_added_version(&self, id: ChunkId, column: usize, version: u64) {
        self.added_versions[id.0 as usize][column].store(version, Ordering::Relaxed);
    }

    /// Whether `row`'s bit is set in `column`'s enabled mask (true if no mask).
    pub fn is_enabled(&self, id: ChunkId, column: usize, row: u16) -> bool {
        match self.enabled.get(&(id, column)) {
            Some(mask) => mask[row as usize / 64] & (1 << (row % 64)) != 0,
            None => true,
        }
    }

    /// Sets `row`'s enabled bit in `column`, materializing an all-enabled mask
    /// (`capacity` bits) on first use.
    pub fn set_enabled(
        &mut self,
        id: ChunkId,
        column: usize,
        row: u16,
        capacity: u16,
        value: bool,
    ) {
        let mask = self
            .enabled
            .entry((id, column))
            .or_insert_with(|| vec![u64::MAX; (capacity as usize).div_ceil(64)]);
        let (word, bit) = (row as usize / 64, row % 64);
        if value {
            mask[word] |= 1 << bit;
        } else {
            mask[word] &= !(1 << bit);
        }
    }

    /// The enabled mask words for `column`, if one exists.
    pub fn enabled_mask(&self, id: ChunkId, column: usize) -> Option<&[u64]> {
        self.enabled.get(&(id, column)).map(Vec::as_slice)
    }

    /// Copies `from`'s enabled bit into `to` for every mask on `chunk`
    /// (swap-remove maintenance).
    pub fn move_enabled_bit(&mut self, chunk: ChunkId, from: u16, to: u16) {
        for (&(c, _), mask) in self.enabled.iter_mut() {
            if c != chunk {
                continue;
            }
            let set = mask[from as usize / 64] & (1 << (from % 64)) != 0;
            let (word, bit) = (to as usize / 64, to % 64);
            if set {
                mask[word] |= 1 << bit;
            } else {
                mask[word] &= !(1 << bit);
            }
        }
    }

    /// Copies one column's enabled bit from a source `(chunk, column, row)` to
    /// a destination one, which may be in another chunk. A no-op when the
    /// source is enabled and the destination has no materialized mask, since
    /// rows default to enabled.
    pub fn transfer_enabled_bit(
        &mut self,
        src: (ChunkId, usize, u16),
        dst: (ChunkId, usize, u16),
        dst_capacity: u16,
    ) {
        let (src_chunk, src_col, src_row) = src;
        let (dst_chunk, dst_col, dst_row) = dst;
        let enabled = self.is_enabled(src_chunk, src_col, src_row);
        if enabled && !self.enabled.contains_key(&(dst_chunk, dst_col)) {
            return;
        }
        self.set_enabled(dst_chunk, dst_col, dst_row, dst_capacity, enabled);
    }

    /// Marks `row` enabled in every mask on `chunk` (new-row default).
    pub fn enable_row(&mut self, chunk: ChunkId, row: u16) {
        for (&(c, _), mask) in self.enabled.iter_mut() {
            if c == chunk {
                mask[row as usize / 64] |= 1 << (row % 64);
            }
        }
    }

    fn new_stamps(columns: usize) -> Box<[AtomicU64]> {
        (0..columns).map(|_| AtomicU64::new(0)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::alloc::ChunkAlloc;
    use crate::storage::archetype::ArchetypeId;

    const ARCH_A: ArchetypeId = ArchetypeId(0);
    const ARCH_B: ArchetypeId = ArchetypeId(1);

    // ---------------------------------------------------------------- creation

    #[test]
    fn create_assigns_dense_ids_and_fresh_metadata() {
        let mut alloc = ChunkAlloc::default();
        let mut chunks = Chunks::default();

        let a = chunks.create(&mut alloc, ARCH_A, 3);
        let b = chunks.create(&mut alloc, ARCH_B, 1);
        assert_eq!(a, ChunkId(0));
        assert_eq!(b, ChunkId(1));
        assert_eq!(alloc.live(), 2, "each chunk owns one block");

        assert_eq!(chunks.len(a), 0);
        assert_eq!(chunks.archetype(a), ARCH_A);
        assert_eq!(chunks.archetype(b), ARCH_B);
        assert_eq!(
            chunks.base(a).as_ptr() as usize % 64,
            0,
            "block is chunk-aligned"
        );
        assert_ne!(chunks.base(a), chunks.base(b));
        for col in 0..3 {
            assert_eq!(
                chunks.write_version(a, col),
                0,
                "fresh chunk: never written"
            );
        }

        chunks.destroy(&mut alloc, a);
        chunks.destroy(&mut alloc, b);
        assert_eq!(alloc.live(), 0);
    }

    #[test]
    fn len_and_write_versions_are_per_chunk() {
        let mut alloc = ChunkAlloc::default();
        let mut chunks = Chunks::default();
        let a = chunks.create(&mut alloc, ARCH_A, 2);
        let b = chunks.create(&mut alloc, ARCH_A, 2);

        chunks.set_len(a, 7);
        chunks.stamp_write_version(a, 1, 42);
        assert_eq!(chunks.len(a), 7);
        assert_eq!(chunks.len(b), 0, "b untouched");
        assert_eq!(chunks.write_version(a, 0), 0, "column 0 untouched");
        assert_eq!(chunks.write_version(a, 1), 42);
        assert_eq!(chunks.write_version(b, 1), 0, "b untouched");

        chunks.set_len(a, 0);
        chunks.destroy(&mut alloc, a);
        chunks.destroy(&mut alloc, b);
    }

    // --------------------------------------------------------------- recycling

    #[test]
    fn destroyed_id_is_recycled_with_fresh_life() {
        let mut alloc = ChunkAlloc::default();
        let mut chunks = Chunks::default();

        let old = chunks.create(&mut alloc, ARCH_A, 2);
        chunks.set_len(old, 5);
        chunks.stamp_write_version(old, 0, 9);
        chunks.stamp_write_version(old, 1, 9);
        chunks.set_len(old, 0);
        chunks.destroy(&mut alloc, old);
        assert_eq!(alloc.live(), 0);

        // Recycle under a DIFFERENT archetype with a different column count.
        let new = chunks.create(&mut alloc, ARCH_B, 4);
        assert_eq!(new, old, "chunk id is recycled");
        assert_eq!(chunks.len(new), 0, "recycled life starts empty");
        assert_eq!(chunks.archetype(new), ARCH_B, "archetype rebound");
        for col in 0..4 {
            assert_eq!(
                chunks.write_version(new, col),
                0,
                "write stamps reset for the new life"
            );
        }

        chunks.destroy(&mut alloc, new);
        assert_eq!(alloc.live(), 0);
    }

    #[test]
    fn order_version_survives_recycling_and_only_climbs() {
        // A cache that saw order_version = v for this id must never observe a
        // smaller-or-equal value afterwards unless the chunk is untouched.
        let mut alloc = ChunkAlloc::default();
        let mut chunks = Chunks::default();

        let id = chunks.create(&mut alloc, ARCH_A, 1);
        let v0 = chunks.order_version(id);
        chunks.bump_order_version(id);
        let v1 = chunks.order_version(id);
        assert!(v1 > v0);

        chunks.destroy(&mut alloc, id);
        let recycled = chunks.create(&mut alloc, ARCH_B, 2);
        assert_eq!(recycled, id);
        assert!(
            chunks.order_version(recycled) > v1,
            "recycled id must look CHANGED to any cache from its previous life"
        );

        chunks.destroy(&mut alloc, recycled);
    }

    #[test]
    fn ids_grow_only_after_free_list_is_drained() {
        let mut alloc = ChunkAlloc::default();
        let mut chunks = Chunks::default();
        let a = chunks.create(&mut alloc, ARCH_A, 1);
        let b = chunks.create(&mut alloc, ARCH_A, 1);
        let c = chunks.create(&mut alloc, ARCH_A, 1);
        chunks.destroy(&mut alloc, b);
        chunks.destroy(&mut alloc, c);

        let d = chunks.create(&mut alloc, ARCH_A, 1);
        let e = chunks.create(&mut alloc, ARCH_A, 1);
        let f = chunks.create(&mut alloc, ARCH_A, 1);
        let mut reused = [d, e];
        reused.sort();
        assert_eq!(reused, [ChunkId(1), ChunkId(2)], "freed ids reused first");
        assert_eq!(f, ChunkId(3), "growth only after the free list is drained");

        for id in [a, d, e, f] {
            chunks.destroy(&mut alloc, id);
        }
        assert_eq!(alloc.live(), 0);
    }

    // -------------------------------------------------------------- guardrails

    #[test]
    #[should_panic(expected = "empty")]
    #[cfg(debug_assertions)]
    #[cfg_attr(
        miri,
        ignore = "leaks the chunk block by design, which miri's leak checker rejects"
    )]
    fn destroying_a_non_empty_chunk_panics_in_debug() {
        let mut alloc = ChunkAlloc::default();
        let mut chunks = Chunks::default();
        let id = chunks.create(&mut alloc, ARCH_A, 1);
        chunks.set_len(id, 3);
        chunks.destroy(&mut alloc, id); // rows were never dropped: refuse
    }

    #[test]
    #[should_panic(expected = "double")]
    #[cfg(debug_assertions)]
    #[cfg_attr(
        miri,
        ignore = "leaks the chunk block by design, which miri's leak checker rejects"
    )]
    fn double_destroy_panics_in_debug() {
        let mut alloc = ChunkAlloc::default();
        let mut chunks = Chunks::default();
        let id = chunks.create(&mut alloc, ARCH_A, 1);
        chunks.destroy(&mut alloc, id);
        chunks.destroy(&mut alloc, id);
    }
}
