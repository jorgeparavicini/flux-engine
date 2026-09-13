use crate::grant::AccessGrant;
use crate::registry::Registry;
use crate::storage::alloc::ChunkAlloc;
use crate::storage::archetype::{Archetype, ArchetypeId};
use crate::storage::chunks::{ChunkId, Chunks};
use crate::storage::layout::{ArchetypeLayout, NO_COLUMN};
use crate::{Component, Entity};

/// Allocates a row for `entity` in `arch`: reuses `non_full`, else creates
/// a chunk (registering it with `arch_id`). Writes the entity column and
/// bumps len + order_version.
///
/// Safety: after return, every sized column of the row is uninitialized;
/// the caller must fully initialize them before the row is read or removed
/// with `drop_values`.
pub(crate) unsafe fn alloc_row(
    arch: &mut Archetype,
    arch_id: ArchetypeId,
    chunks: &mut Chunks,
    alloc: &mut ChunkAlloc,
    entity: Entity,
) -> (ChunkId, u16) {
    let chunk = match arch.non_full {
        Some(chunk) => chunk,
        None => {
            let chunk = chunks.create(alloc, arch_id, arch.layout.components.len());
            arch.chunks.push(chunk);
            chunk
        }
    };
    let row = chunks.len(chunk);
    let ptr = unsafe { entity_slot_ptr(chunks, &arch.layout, chunk, row) };
    unsafe { ptr.write(entity) };
    chunks.set_len(chunk, row + 1);
    chunks.bump_order_version(chunk);
    chunks.enable_row(chunk, row);
    arch.non_full = (chunks.len(chunk) < arch.layout.capacity).then_some(chunk);
    (chunk, row)
}

/// The entity stored at (`chunk`, `row`). Safety: row < len.
pub(crate) unsafe fn entity_at(
    chunks: &Chunks,
    layout: &ArchetypeLayout,
    chunk: ChunkId,
    row: u16,
) -> Entity {
    unsafe { entity_slot_ptr(chunks, layout, chunk, row).read() }
}

unsafe fn entity_slot_ptr(
    chunks: &Chunks,
    layout: &ArchetypeLayout,
    chunk: ChunkId,
    row: u16,
) -> *mut Entity {
    unsafe {
        chunks
            .base(chunk)
            .as_ptr()
            .add(layout.entity_offset as usize + row as usize * size_of::<Entity>())
            .cast::<Entity>()
    }
}

/// Moves the value at `src` into column `column` at `row`. Ownership
/// transfers; the caller must not drop or reuse the source value.
/// Zero-sized columns (offset NO_COLUMN) are a no-op.
///
/// Safety: `src` reads `size_of(component)` bytes; row < len; the slot is
/// treated as uninitialized (no drop of a previous value).
pub(crate) unsafe fn write_component(
    chunks: &Chunks,
    layout: &ArchetypeLayout,
    reg: &Registry,
    chunk: ChunkId,
    column: usize,
    row: u16,
    src: *const u8,
) {
    // Zero sized types have nothing to move.
    if layout.offsets[column] == NO_COLUMN {
        return;
    }
    let size = reg.info(layout.components[column]).size;
    unsafe {
        std::ptr::copy_nonoverlapping(
            src,
            component_ptr(chunks, layout, reg, chunk, column, row),
            size,
        );
    }
}

/// Pointer to the component value at (`chunk`, `row`, `column`), or a
/// dangling-but-aligned pointer for zero-sized columns.
/// Safety: chunk belongs to an archetype described by `layout`; row < len;
/// column indexes the signature.
pub(crate) unsafe fn component_ptr(
    chunks: &Chunks,
    layout: &ArchetypeLayout,
    reg: &Registry,
    chunk: ChunkId,
    column: usize,
    row: u16,
) -> *mut u8 {
    let offset = layout.offsets[column];
    if offset == NO_COLUMN {
        return std::ptr::without_provenance_mut(reg.info(layout.components[column]).align);
    }
    let size = reg.info(layout.components[column]).size;
    unsafe {
        chunks
            .base(chunk)
            .as_ptr()
            .add(offset as usize + row as usize * size)
    }
}

/// Removes a row. If `drop_values`, runs each column's drop_fn on the
/// removed values; otherwise ownership has already moved elsewhere. If the
/// removed row was not the tail, the tail row (every column + entity) is
/// moved into its place and that entity is returned — the caller must
/// update its slot. An emptied chunk is destroyed and removed from `arch`.
///
/// Safety: row < len; the row's sized columns are initialized (when
/// drop_values) or already moved out (when not).
pub(crate) unsafe fn swap_remove_row(
    arch: &mut Archetype,
    chunks: &mut Chunks,
    alloc: &mut ChunkAlloc,
    reg: &Registry,
    chunk: ChunkId,
    row: u16,
    drop_values: bool,
) -> Option<Entity> {
    let last = chunks.len(chunk) - 1;

    if drop_values {
        for (column, component_id) in arch.layout.components.iter().enumerate() {
            if let Some(drop_fn) = reg.info(*component_id).drop_fn {
                unsafe {
                    drop_fn(
                        component_ptr(chunks, &arch.layout, reg, chunk, column, row),
                        1,
                    )
                };
            }
        }
    }

    let swapped = if row != last {
        for (column, component_id) in arch.layout.components.iter().enumerate() {
            if arch.layout.offsets[column] == NO_COLUMN {
                continue;
            }
            let size = reg.info(*component_id).size;
            unsafe {
                std::ptr::copy_nonoverlapping(
                    component_ptr(chunks, &arch.layout, reg, chunk, column, last),
                    component_ptr(chunks, &arch.layout, reg, chunk, column, row),
                    size,
                );
            }
        }
        let entity = unsafe { entity_at(chunks, &arch.layout, chunk, last) };
        unsafe { entity_slot_ptr(chunks, &arch.layout, chunk, row).write(entity) };
        chunks.move_enabled_bit(chunk, last, row);
        Some(entity)
    } else {
        None
    };

    chunks.set_len(chunk, last);
    chunks.bump_order_version(chunk);

    if last == 0 {
        chunks.destroy(alloc, chunk);
        let position = arch
            .chunks
            .iter()
            .position(|c| *c == chunk)
            .expect("chunk belongs to its archetype");
        arch.chunks.swap_remove(position);
        if arch.non_full == Some(chunk) {
            arch.non_full = None;
        }
    } else {
        arch.non_full = Some(chunk);
    }
    swapped
}

/// Moves one row from `src` to `dst`: intersection columns transfer
/// ownership, dst-only columns are left uninitialized for the caller (same
/// contract as `alloc_row`). Source-only columns are dropped when
/// `drop_source_only` is true; otherwise their ownership has already been
/// taken by the caller and they are abandoned without dropping. Returns the
/// destination (chunk, row) and the entity swapped into the vacated src row,
/// if any.
///
/// Safety: `src_arch`/`dst_arch` are distinct archetypes; src_row < len;
/// the source row is fully initialized, except that when `drop_source_only`
/// is false every source-only value must already have been moved out.
#[allow(
    clippy::too_many_arguments,
    reason = "internal primitive behind the World API, which owns every argument"
)]
pub(crate) unsafe fn move_row(
    src_arch: &mut Archetype,
    dst_arch: &mut Archetype,
    dst_id: ArchetypeId,
    chunks: &mut Chunks,
    alloc: &mut ChunkAlloc,
    reg: &Registry,
    src_chunk: ChunkId,
    src_row: u16,
    drop_source_only: bool,
) -> (ChunkId, u16, Option<Entity>) {
    let entity = unsafe { entity_at(chunks, &src_arch.layout, src_chunk, src_row) };
    let (dst_chunk, dst_row) = unsafe { alloc_row(dst_arch, dst_id, chunks, alloc, entity) };

    // Merge the two sorted signatures: equal ids transfer, source-only ids
    // drop, destination-only ids stay uninitialized for the caller.
    let src_sig = &src_arch.layout.components;
    let dst_sig = &dst_arch.layout.components;
    let (mut src_col, mut dst_col) = (0, 0);
    while src_col < src_sig.len() {
        if dst_col < dst_sig.len() && src_sig[src_col] == dst_sig[dst_col] {
            if src_arch.layout.offsets[src_col] != NO_COLUMN {
                let size = reg.info(src_sig[src_col]).size;
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        component_ptr(chunks, &src_arch.layout, reg, src_chunk, src_col, src_row),
                        component_ptr(chunks, &dst_arch.layout, reg, dst_chunk, dst_col, dst_row),
                        size,
                    );
                }
            }
            if reg.info(src_sig[src_col]).toggleable {
                chunks.transfer_enabled_bit(
                    (src_chunk, src_col, src_row),
                    (dst_chunk, dst_col, dst_row),
                    dst_arch.layout.capacity,
                );
            }
            src_col += 1;
            dst_col += 1;
        } else if dst_col < dst_sig.len() && dst_sig[dst_col] < src_sig[src_col] {
            dst_col += 1;
        } else {
            if drop_source_only && let Some(drop_fn) = reg.info(src_sig[src_col]).drop_fn {
                unsafe {
                    drop_fn(
                        component_ptr(chunks, &src_arch.layout, reg, src_chunk, src_col, src_row),
                        1,
                    )
                };
            }
            src_col += 1;
        }
    }

    let swapped =
        unsafe { swap_remove_row(src_arch, chunks, alloc, reg, src_chunk, src_row, false) };
    (dst_chunk, dst_row, swapped)
}

/// The chunk's `T` column as a slice of the occupied rows.
///
/// None if the grant does not allow reading `T`, or `column` is not `T`'s
/// column. Zero-sized columns yield a valid slice of `len` values.
///
/// # Safety
///
/// `chunk` belongs to an archetype described by `layout`; `column` indexes
/// the signature; rows `0..len` are initialized.
pub(crate) unsafe fn column<'w, T: Component>(
    chunks: &'w Chunks,
    layout: &ArchetypeLayout,
    reg: &Registry,
    chunk: ChunkId,
    column: usize,
    grant: &AccessGrant,
) -> Option<&'w [T]> {
    if !grant.allows_read(T::KEY) {
        return None;
    }

    if reg.info(layout.components[column]).key != T::KEY {
        return None;
    }

    let len = chunks.len(chunk) as usize;
    unsafe {
        Some(std::slice::from_raw_parts(
            component_ptr(chunks, layout, reg, chunk, column, 0).cast::<T>(),
            len,
        ))
    }
}

/// Mutable variant of [`column`](column()): requires write permission and an unspent
/// claim on the (chunk, component) pair, which it records on success.
///
/// # Safety
///
/// As [`column`](column()). The returned `&mut` is manufactured from `&Chunks`: sound
/// because the pointer's provenance is the raw chunk allocation, never a
/// shared reference to the data, and the claim discipline guarantees no
/// second mutable view exists through this grant.
#[allow(
    clippy::mut_from_ref,
    reason = "provenance is the raw allocation; aliasing is excluded by grant claims"
)]
pub(crate) unsafe fn column_mut<'w, T: Component>(
    chunks: &'w Chunks,
    layout: &ArchetypeLayout,
    reg: &Registry,
    chunk: ChunkId,
    column: usize,
    grant: &mut AccessGrant,
) -> Option<&'w mut [T]> {
    let id = layout.components[column];
    if !grant.allows_write(T::KEY) || reg.info(id).key != T::KEY || !grant.claim_mut(chunk, id) {
        return None;
    }
    chunks.stamp_write_version(chunk, column, grant.version());

    let len = chunks.len(chunk) as usize;
    unsafe {
        Some(std::slice::from_raw_parts_mut(
            component_ptr(chunks, layout, reg, chunk, column, 0).cast::<T>(),
            len,
        ))
    }
}

/// The chunk's entity id column as a slice of the occupied rows.
///
/// # Safety
///
/// `chunk` belongs to an archetype described by `layout`; rows `0..len` are
/// initialized.
pub(crate) unsafe fn entity_column<'w>(
    chunks: &'w Chunks,
    layout: &ArchetypeLayout,
    chunk: ChunkId,
) -> &'w [Entity] {
    let len = chunks.len(chunk) as usize;
    unsafe { std::slice::from_raw_parts(entity_slot_ptr(chunks, layout, chunk, 0), len) }
}

/// Bulk-moves every row of `src_chunk` into `dst_arch`, appending them and
/// range-copying each intersection column and the entity column. Does not
/// remove the rows from the source: the caller destroys `src_chunk` after.
///
/// Returns, per source row in order, the destination `(chunk, row)`.
///
/// # Safety
///
/// `src_chunk` belongs to an archetype whose signature is a subset of
/// `dst_arch`'s (an insert), and its rows are fully initialized.
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn move_full_chunk(
    src_layout: &ArchetypeLayout,
    dst_arch: &mut Archetype,
    dst_id: ArchetypeId,
    chunks: &mut Chunks,
    alloc: &mut ChunkAlloc,
    reg: &Registry,
    src_chunk: ChunkId,
    out: &mut Vec<(ChunkId, u16)>,
) {
    let total = chunks.len(src_chunk) as usize;
    let mut moved = 0usize;
    while moved < total {
        // Find or create a destination chunk with room, append a contiguous run.
        let dst_chunk = match dst_arch.non_full {
            Some(c) if chunks.len(c) < dst_arch.layout.capacity => c,
            _ => {
                let c = chunks.create(alloc, dst_id, dst_arch.layout.components.len());
                dst_arch.chunks.push(c);
                c
            }
        };
        let dst_start = chunks.len(dst_chunk) as usize;
        let run = (dst_arch.layout.capacity as usize - dst_start).min(total - moved);

        // Copy the entity column range.
        unsafe {
            let src = entity_slot_ptr(chunks, src_layout, src_chunk, moved as u16).cast_const();
            let dst = entity_slot_ptr(chunks, &dst_arch.layout, dst_chunk, dst_start as u16);
            std::ptr::copy_nonoverlapping(src, dst, run);
        }
        // Copy each intersection column range (two-pointer merge of signatures).
        let (src_sig, dst_sig) = (&src_layout.components, &dst_arch.layout.components);
        let (mut i, mut j) = (0, 0);
        while i < src_sig.len() {
            if j < dst_sig.len() && src_sig[i] == dst_sig[j] {
                if src_layout.offsets[i] != NO_COLUMN {
                    let size = reg.info(src_sig[i]).size;
                    unsafe {
                        let src =
                            component_ptr(chunks, src_layout, reg, src_chunk, i, moved as u16);
                        let dst = component_ptr(
                            chunks,
                            &dst_arch.layout,
                            reg,
                            dst_chunk,
                            j,
                            dst_start as u16,
                        );
                        std::ptr::copy_nonoverlapping(src, dst, size * run);
                    }
                }
                if reg.info(src_sig[i]).toggleable {
                    for k in 0..run {
                        chunks.transfer_enabled_bit(
                            (src_chunk, i, (moved + k) as u16),
                            (dst_chunk, j, (dst_start + k) as u16),
                            dst_arch.layout.capacity,
                        );
                    }
                }
                i += 1;
                j += 1;
            } else if j < dst_sig.len() && dst_sig[j] < src_sig[i] {
                j += 1; // dst-only: left for the caller
            } else {
                i += 1; // src-only cannot occur for an insert (subset), but skip safely
            }
        }
        for k in 0..run {
            out.push((dst_chunk, (dst_start + k) as u16));
        }
        chunks.set_len(dst_chunk, (dst_start + run) as u16);
        chunks.bump_order_version(dst_chunk);
        dst_arch.non_full = (chunks.len(dst_chunk) < dst_arch.layout.capacity).then_some(dst_chunk);
        moved += run;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{Component, ComponentKey, StorageClass};
    use crate::entity::{Entities, Entity};
    use crate::registry::{ComponentId, Registry};
    use crate::storage::alloc::ChunkAlloc;
    use crate::storage::archetype::{Archetype, ArchetypeId};
    use crate::storage::chunks::{ChunkId, Chunks};
    use std::cell::Cell;
    use std::mem::forget;
    use std::rc::Rc;

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("ops::tests::", stringify!($name)));
            }
        };
    }

    #[derive(Copy, Clone, PartialEq, Debug)]
    struct A(u64);
    component!(A);
    #[derive(Copy, Clone, PartialEq, Debug)]
    struct B(u16);
    component!(B);
    struct Zst;
    impl Component for Zst {
        const KEY: ComponentKey = ComponentKey::from_path("ops::tests::Zst");
        const STORAGE: StorageClass = StorageClass::Tag;
    }
    struct DropCounter(#[allow(dead_code)] Rc<Cell<usize>>, u64);
    component!(DropCounter);
    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    /// One test world's storage, minus entity slots.
    struct Bench {
        reg: Registry,
        chunks: Chunks,
        alloc: ChunkAlloc,
        entities: Entities,
        a: ComponentId,
        b: ComponentId,
        zst: ComponentId,
        drop: ComponentId,
    }

    impl Bench {
        fn new() -> Self {
            let mut reg = Registry::new();
            let a = reg.register::<A>();
            let b = reg.register::<B>();
            let zst = reg.register::<Zst>();
            let drop = reg.register::<DropCounter>();
            Self {
                reg,
                chunks: Chunks::default(),
                alloc: ChunkAlloc::default(),
                entities: Entities::new(),
                a,
                b,
                zst,
                drop,
            }
        }

        fn archetype(&self, ids: &[ComponentId]) -> Archetype {
            Archetype::new(ids, &self.reg).unwrap()
        }

        fn col(arch: &Archetype, id: ComponentId) -> usize {
            arch.signature().iter().position(|c| *c == id).unwrap()
        }

        /// Spawn a row and write `value` into the single sized column `col`.
        unsafe fn put<T>(
            &mut self,
            arch: &mut Archetype,
            arch_id: ArchetypeId,
            col: usize,
            value: T,
        ) -> (ChunkId, u16, Entity) {
            let entity = self.entities.alloc();
            unsafe {
                let (chunk, row) =
                    alloc_row(arch, arch_id, &mut self.chunks, &mut self.alloc, entity);
                write_component(
                    &self.chunks,
                    &arch.layout,
                    &self.reg,
                    chunk,
                    col,
                    row,
                    (&raw const value).cast(),
                );
                forget(value);
                (chunk, row, entity)
            }
        }

        /// Writes `value` into (`chunk`, `row`, `col`), transferring ownership.
        unsafe fn write_val<T>(
            &self,
            layout: &ArchetypeLayout,
            chunk: ChunkId,
            col: usize,
            row: u16,
            value: T,
        ) {
            unsafe {
                write_component(
                    &self.chunks,
                    layout,
                    &self.reg,
                    chunk,
                    col,
                    row,
                    (&raw const value).cast(),
                )
            };
            forget(value);
        }

        unsafe fn read<T: Copy>(
            &self,
            arch: &Archetype,
            chunk: ChunkId,
            row: u16,
            col: usize,
        ) -> T {
            unsafe {
                component_ptr(&self.chunks, &arch.layout, &self.reg, chunk, col, row)
                    .cast::<T>()
                    .read()
            }
        }
    }

    /// Structural invariants that must hold after every op.
    fn assert_consistent(bench: &Bench, arch: &Archetype, expected_rows: usize) {
        let total: usize = arch
            .chunks
            .iter()
            .map(|c| bench.chunks.len(*c) as usize)
            .sum();
        assert_eq!(total, expected_rows, "row count conserved");
        for chunk in &arch.chunks {
            assert!(bench.chunks.len(*chunk) as usize <= arch.layout.capacity as usize);
            assert!(
                bench.chunks.len(*chunk) > 0,
                "empty chunks must be destroyed, not kept"
            );
        }
        if let Some(nf) = arch.non_full {
            assert!(
                arch.chunks.contains(&nf),
                "non_full must reference a live chunk"
            );
            assert!(
                bench.chunks.len(nf) < arch.layout.capacity,
                "non_full must actually have space"
            );
        }
        assert!(
            bench.alloc.live() >= arch.chunks.len(),
            "every live chunk owns an allocator block"
        );
    }

    const ARCH: ArchetypeId = ArchetypeId(0);
    const DST: ArchetypeId = ArchetypeId(1);

    // ------------------------------------------------------------- allocation

    #[test]
    fn alloc_row_fills_a_chunk_then_opens_the_next() {
        let mut bench = Bench::new();
        let mut arch = bench.archetype(&[bench.a]);
        let col = Bench::col(&arch, bench.a);
        let cap = arch.layout.capacity as usize;

        let mut placed = Vec::new();
        for i in 0..cap {
            let (chunk, row, entity) = unsafe { bench.put(&mut arch, ARCH, col, A(i as u64)) };
            placed.push((chunk, row, entity));
        }
        assert_eq!(arch.chunks.len(), 1, "exactly one chunk until it is full");
        assert_consistent(&bench, &arch, cap);

        let (chunk2, row2, _) = unsafe { bench.put(&mut arch, ARCH, col, A(999)) };
        assert_eq!(arch.chunks.len(), 2, "overflow opens a second chunk");
        assert_ne!(chunk2, placed[0].0);
        assert_eq!(row2, 0, "new chunk starts at row 0");
        assert_consistent(&bench, &arch, cap + 1);

        // entity column and data written correctly at both ends
        for &(chunk, row, entity) in [&placed[0], &placed[cap - 1]] {
            assert_eq!(
                unsafe { entity_at(&bench.chunks, &arch.layout, chunk, row) },
                entity
            );
        }
        assert_eq!(
            unsafe { bench.read::<A>(&arch, placed[7].0, placed[7].1, col) },
            A(7)
        );
        assert_eq!(unsafe { bench.read::<A>(&arch, chunk2, row2, col) }, A(999));

        // teardown: pop every row (tail-first keeps rows stable)
        for i in (0..cap).rev() {
            unsafe {
                swap_remove_row(
                    &mut arch,
                    &mut bench.chunks,
                    &mut bench.alloc,
                    &bench.reg,
                    placed[i].0,
                    placed[i].1,
                    true,
                )
            };
        }
        unsafe {
            swap_remove_row(
                &mut arch,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk2,
                0,
                true,
            )
        };
        assert_consistent(&bench, &arch, 0);
    }

    #[test]
    fn multi_column_write_and_read_roundtrip() {
        let mut bench = Bench::new();
        let mut arch = bench.archetype(&[bench.a, bench.b, bench.zst]);
        let (a_col, b_col) = (Bench::col(&arch, bench.a), Bench::col(&arch, bench.b));

        let entity = bench.entities.alloc();
        let (chunk, row) = unsafe {
            let pair = alloc_row(&mut arch, ARCH, &mut bench.chunks, &mut bench.alloc, entity);
            bench.write_val(&arch.layout, pair.0, a_col, pair.1, A(42));
            bench.write_val(&arch.layout, pair.0, b_col, pair.1, B(7));
            pair
        };
        assert_eq!(unsafe { bench.read::<A>(&arch, chunk, row, a_col) }, A(42));
        assert_eq!(unsafe { bench.read::<B>(&arch, chunk, row, b_col) }, B(7));
        assert_eq!(
            unsafe { entity_at(&bench.chunks, &arch.layout, chunk, row) },
            entity
        );
        assert_consistent(&bench, &arch, 1);

        unsafe {
            swap_remove_row(
                &mut arch,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk,
                row,
                true,
            )
        };
        assert_consistent(&bench, &arch, 0);
    }

    // ---------------------------------------------------------------- removal

    #[test]
    fn removing_the_tail_returns_no_swapped_entity() {
        let mut bench = Bench::new();
        let mut arch = bench.archetype(&[bench.a]);
        let col = Bench::col(&arch, bench.a);
        let (chunk, ..) = unsafe { bench.put(&mut arch, ARCH, col, A(0)) };
        let (_, tail_row, _) = unsafe { bench.put(&mut arch, ARCH, col, A(1)) };

        let swapped = unsafe {
            swap_remove_row(
                &mut arch,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk,
                tail_row,
                true,
            )
        };
        assert_eq!(swapped, None);
        assert_eq!(bench.chunks.len(chunk), 1);
        assert_eq!(
            unsafe { bench.read::<A>(&arch, chunk, 0, col) },
            A(0),
            "row 0 untouched"
        );
        assert_consistent(&bench, &arch, 1);
        unsafe {
            swap_remove_row(
                &mut arch,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk,
                0,
                true,
            )
        };
    }

    #[test]
    fn removing_a_middle_row_moves_the_tail_into_the_hole() {
        let mut bench = Bench::new();
        let mut arch = bench.archetype(&[bench.a, bench.b]);
        let (a_col, b_col) = (Bench::col(&arch, bench.a), Bench::col(&arch, bench.b));

        let mut entities = Vec::new();
        let mut chunk = ChunkId(0);
        for i in 0..5u64 {
            let entity = bench.entities.alloc();
            let (c, row) =
                unsafe { alloc_row(&mut arch, ARCH, &mut bench.chunks, &mut bench.alloc, entity) };
            unsafe {
                bench.write_val(&arch.layout, c, a_col, row, A(i * 10));
                bench.write_val(&arch.layout, c, b_col, row, B(i as u16));
            }
            chunk = c;
            entities.push(entity);
        }

        // remove row 1; row 4 (the tail) must take its place in EVERY column
        let swapped = unsafe {
            swap_remove_row(
                &mut arch,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk,
                1,
                true,
            )
        };
        assert_eq!(
            swapped,
            Some(entities[4]),
            "the displaced tail entity is reported"
        );
        assert_eq!(bench.chunks.len(chunk), 4);
        assert_eq!(
            unsafe { bench.read::<A>(&arch, chunk, 1, a_col) },
            A(40),
            "tail A moved into the hole"
        );
        assert_eq!(
            unsafe { bench.read::<B>(&arch, chunk, 1, b_col) },
            B(4),
            "tail B moved into the hole"
        );
        assert_eq!(
            unsafe { entity_at(&bench.chunks, &arch.layout, chunk, 1) },
            entities[4],
            "entity column follows"
        );
        // untouched rows stay put
        assert_eq!(unsafe { bench.read::<A>(&arch, chunk, 0, a_col) }, A(0));
        assert_eq!(unsafe { bench.read::<A>(&arch, chunk, 2, a_col) }, A(20));
        assert_eq!(unsafe { bench.read::<A>(&arch, chunk, 3, a_col) }, A(30));
        assert_consistent(&bench, &arch, 4);

        for row in (0..4).rev() {
            unsafe {
                swap_remove_row(
                    &mut arch,
                    &mut bench.chunks,
                    &mut bench.alloc,
                    &bench.reg,
                    chunk,
                    row,
                    true,
                )
            };
        }
    }

    #[test]
    fn drop_values_true_drops_removed_row_exactly_once_and_never_the_moved_tail() {
        let mut bench = Bench::new();
        let mut arch = bench.archetype(&[bench.drop]);
        let col = Bench::col(&arch, bench.drop);
        let drops = Rc::new(Cell::new(0));

        let (chunk, ..) =
            unsafe { bench.put(&mut arch, ARCH, col, DropCounter(Rc::clone(&drops), 0)) };
        unsafe { bench.put(&mut arch, ARCH, col, DropCounter(Rc::clone(&drops), 1)) };

        // middle removal: row 0 dropped once, tail MOVED (not dropped)
        unsafe {
            swap_remove_row(
                &mut arch,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk,
                0,
                true,
            )
        };
        assert_eq!(
            drops.get(),
            1,
            "removed value dropped exactly once; moved tail not dropped"
        );
        let payload = unsafe {
            (*component_ptr(&bench.chunks, &arch.layout, &bench.reg, chunk, col, 0)
                .cast::<DropCounter>())
            .1
        };
        assert_eq!(payload, 1, "tail payload moved into the hole");

        unsafe {
            swap_remove_row(
                &mut arch,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk,
                0,
                true,
            )
        };
        assert_eq!(drops.get(), 2);
        assert_consistent(&bench, &arch, 0);
    }

    #[test]
    fn drop_values_false_transfers_ownership() {
        let mut bench = Bench::new();
        let mut arch = bench.archetype(&[bench.drop]);
        let col = Bench::col(&arch, bench.drop);
        let drops = Rc::new(Cell::new(0));
        let (chunk, row, _) =
            unsafe { bench.put(&mut arch, ARCH, col, DropCounter(Rc::clone(&drops), 7)) };

        // move the value out manually, then remove without dropping
        let taken: DropCounter = unsafe {
            component_ptr(&bench.chunks, &arch.layout, &bench.reg, chunk, col, row)
                .cast::<DropCounter>()
                .read()
        };
        unsafe {
            swap_remove_row(
                &mut arch,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk,
                row,
                false,
            )
        };
        assert_eq!(
            drops.get(),
            0,
            "storage must not drop a value that moved out"
        );
        assert_eq!(taken.1, 7);
        drop(taken);
        assert_eq!(drops.get(), 1);
        assert_consistent(&bench, &arch, 0);
    }

    #[test]
    fn emptied_chunk_is_destroyed_and_forgotten() {
        let mut bench = Bench::new();
        let mut arch = bench.archetype(&[bench.a]);
        let col = Bench::col(&arch, bench.a);
        let (chunk, row, _) = unsafe { bench.put(&mut arch, ARCH, col, A(1)) };
        assert_eq!(bench.alloc.live(), 1);

        unsafe {
            swap_remove_row(
                &mut arch,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk,
                row,
                true,
            )
        };
        assert!(arch.chunks.is_empty());
        assert_eq!(
            arch.non_full, None,
            "non_full must not dangle to a destroyed chunk"
        );
        assert_eq!(
            bench.alloc.live(),
            0,
            "the block went back to the allocator"
        );
    }

    #[test]
    fn order_version_bumps_on_alloc_and_remove() {
        let mut bench = Bench::new();
        let mut arch = bench.archetype(&[bench.a]);
        let col = Bench::col(&arch, bench.a);
        let (chunk, ..) = unsafe { bench.put(&mut arch, ARCH, col, A(0)) };
        let after_first = bench.chunks.order_version(chunk);

        unsafe { bench.put(&mut arch, ARCH, col, A(1)) };
        let after_second = bench.chunks.order_version(chunk);
        assert!(
            after_second > after_first,
            "adding a row is a structural change"
        );

        unsafe {
            swap_remove_row(
                &mut arch,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk,
                1,
                true,
            )
        };
        assert!(
            bench.chunks.order_version(chunk) > after_second,
            "removing a row is a structural change"
        );
        unsafe {
            swap_remove_row(
                &mut arch,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk,
                0,
                true,
            )
        };
    }

    // ------------------------------------------------------------------- move

    #[test]
    fn move_row_transfers_intersection_and_drops_source_only_columns() {
        let mut bench = Bench::new();
        // src {A, B, DropCounter} → dst {A, B}: DropCounter is dropped by the move
        let mut src = bench.archetype(&[bench.a, bench.b, bench.drop]);
        let mut dst = bench.archetype(&[bench.a, bench.b]);
        let drops = Rc::new(Cell::new(0));

        let entity = bench.entities.alloc();
        let (src_chunk, src_row) =
            unsafe { alloc_row(&mut src, ARCH, &mut bench.chunks, &mut bench.alloc, entity) };
        unsafe {
            bench.write_val(
                &src.layout,
                src_chunk,
                Bench::col(&src, bench.a),
                src_row,
                A(11),
            );
            bench.write_val(
                &src.layout,
                src_chunk,
                Bench::col(&src, bench.b),
                src_row,
                B(22),
            );
            bench.write_val(
                &src.layout,
                src_chunk,
                Bench::col(&src, bench.drop),
                src_row,
                DropCounter(Rc::clone(&drops), 5),
            );
        }

        let (dst_chunk, dst_row, swapped) = unsafe {
            move_row(
                &mut src,
                &mut dst,
                DST,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                src_chunk,
                src_row,
                true,
            )
        };
        assert_eq!(swapped, None, "single-row source has no tail to swap");
        assert_eq!(
            drops.get(),
            1,
            "source-only component dropped exactly once by the move"
        );
        assert!(src.chunks.is_empty(), "source chunk emptied and destroyed");
        assert_eq!(
            unsafe { bench.read::<A>(&dst, dst_chunk, dst_row, Bench::col(&dst, bench.a)) },
            A(11)
        );
        assert_eq!(
            unsafe { bench.read::<B>(&dst, dst_chunk, dst_row, Bench::col(&dst, bench.b)) },
            B(22)
        );
        assert_eq!(
            unsafe { entity_at(&bench.chunks, &dst.layout, dst_chunk, dst_row) },
            entity
        );
        assert_consistent(&bench, &dst, 1);

        unsafe {
            swap_remove_row(
                &mut dst,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                dst_chunk,
                dst_row,
                true,
            )
        };
    }

    #[test]
    fn move_row_growing_the_signature_leaves_new_column_for_the_caller() {
        let mut bench = Bench::new();
        // src {A} → dst {A, B}: caller writes B afterwards (insert path)
        let mut src = bench.archetype(&[bench.a]);
        let mut dst = bench.archetype(&[bench.a, bench.b]);

        let a_src_col = Bench::col(&src, bench.a);
        let (src_chunk, src_row, entity) = unsafe { bench.put(&mut src, ARCH, a_src_col, A(77)) };
        let (dst_chunk, dst_row, _) = unsafe {
            move_row(
                &mut src,
                &mut dst,
                DST,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                src_chunk,
                src_row,
                true,
            )
        };
        unsafe {
            bench.write_val(
                &dst.layout,
                dst_chunk,
                Bench::col(&dst, bench.b),
                dst_row,
                B(88),
            );
        }
        assert_eq!(
            unsafe { bench.read::<A>(&dst, dst_chunk, dst_row, Bench::col(&dst, bench.a)) },
            A(77),
            "intersection preserved"
        );
        assert_eq!(
            unsafe { bench.read::<B>(&dst, dst_chunk, dst_row, Bench::col(&dst, bench.b)) },
            B(88)
        );
        assert_eq!(
            unsafe { entity_at(&bench.chunks, &dst.layout, dst_chunk, dst_row) },
            entity
        );
        assert_consistent(&bench, &dst, 1);

        unsafe {
            swap_remove_row(
                &mut dst,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                dst_chunk,
                dst_row,
                true,
            )
        };
    }

    #[test]
    fn move_row_does_not_drop_transferred_ownership() {
        let mut bench = Bench::new();
        // src {DropCounter} → dst {B, DropCounter}: the counter must MOVE, not drop
        let mut src = bench.archetype(&[bench.drop]);
        let mut dst = bench.archetype(&[bench.b, bench.drop]);
        let drops = Rc::new(Cell::new(0));

        let drop_src_col = Bench::col(&src, bench.drop);
        let (src_chunk, src_row, _) = unsafe {
            bench.put(
                &mut src,
                ARCH,
                drop_src_col,
                DropCounter(Rc::clone(&drops), 9),
            )
        };
        let (dst_chunk, dst_row, _) = unsafe {
            move_row(
                &mut src,
                &mut dst,
                DST,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                src_chunk,
                src_row,
                true,
            )
        };
        unsafe {
            bench.write_val(
                &dst.layout,
                dst_chunk,
                Bench::col(&dst, bench.b),
                dst_row,
                B(1),
            );
        }
        assert_eq!(
            drops.get(),
            0,
            "moving between archetypes must not drop the value"
        );
        let payload = unsafe {
            (*component_ptr(
                &bench.chunks,
                &dst.layout,
                &bench.reg,
                dst_chunk,
                Bench::col(&dst, bench.drop),
                dst_row,
            )
            .cast::<DropCounter>())
            .1
        };
        assert_eq!(payload, 9, "payload intact after the move");

        unsafe {
            swap_remove_row(
                &mut dst,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                dst_chunk,
                dst_row,
                true,
            )
        };
        assert_eq!(drops.get(), 1, "dropped exactly once, at final removal");
    }

    #[test]
    fn move_row_without_dropping_leaves_source_only_ownership_with_the_caller() {
        let mut bench = Bench::new();
        // src {A, DropCounter} → dst {A}: caller takes the counter first
        let mut src = bench.archetype(&[bench.a, bench.drop]);
        let mut dst = bench.archetype(&[bench.a]);
        let drops = Rc::new(Cell::new(0));

        let entity = bench.entities.alloc();
        let (src_chunk, src_row) =
            unsafe { alloc_row(&mut src, ARCH, &mut bench.chunks, &mut bench.alloc, entity) };
        unsafe {
            bench.write_val(
                &src.layout,
                src_chunk,
                Bench::col(&src, bench.a),
                src_row,
                A(3),
            );
            bench.write_val(
                &src.layout,
                src_chunk,
                Bench::col(&src, bench.drop),
                src_row,
                DropCounter(Rc::clone(&drops), 4),
            );
        }

        let taken: DropCounter = unsafe {
            component_ptr(
                &bench.chunks,
                &src.layout,
                &bench.reg,
                src_chunk,
                Bench::col(&src, bench.drop),
                src_row,
            )
            .cast::<DropCounter>()
            .read()
        };
        let (dst_chunk, dst_row, _) = unsafe {
            move_row(
                &mut src,
                &mut dst,
                DST,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                src_chunk,
                src_row,
                false,
            )
        };
        assert_eq!(
            drops.get(),
            0,
            "abandoned source-only value must not be dropped by the move"
        );
        assert_eq!(taken.1, 4);
        drop(taken);
        assert_eq!(drops.get(), 1);
        assert_eq!(
            unsafe { bench.read::<A>(&dst, dst_chunk, dst_row, Bench::col(&dst, bench.a)) },
            A(3)
        );

        unsafe {
            swap_remove_row(
                &mut dst,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                dst_chunk,
                dst_row,
                true,
            )
        };
    }

    #[test]
    fn move_row_reports_the_swapped_source_entity() {
        let mut bench = Bench::new();
        let mut src = bench.archetype(&[bench.a]);
        let mut dst = bench.archetype(&[bench.a, bench.b]);
        let col = Bench::col(&src, bench.a);

        let (chunk, _, _e0) = unsafe { bench.put(&mut src, ARCH, col, A(0)) };
        let (_, row1, _e1) = unsafe { bench.put(&mut src, ARCH, col, A(1)) };
        let (_, _, e2) = unsafe { bench.put(&mut src, ARCH, col, A(2)) };

        // move the middle row out; the tail (e2) fills it
        let (.., swapped) = unsafe {
            move_row(
                &mut src,
                &mut dst,
                DST,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk,
                row1,
                true,
            )
        };
        assert_eq!(swapped, Some(e2));
        assert_eq!(
            unsafe { bench.read::<A>(&src, chunk, row1, col) },
            A(2),
            "tail moved into the vacated row"
        );
        assert_consistent(&bench, &src, 2);

        // teardown
        unsafe {
            swap_remove_row(
                &mut src,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk,
                1,
                true,
            );
            swap_remove_row(
                &mut src,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                chunk,
                0,
                true,
            );
            let dst_chunk = dst.chunks[0];
            bench.write_val(&dst.layout, dst_chunk, Bench::col(&dst, bench.b), 0, B(0));
            swap_remove_row(
                &mut dst,
                &mut bench.chunks,
                &mut bench.alloc,
                &bench.reg,
                dst_chunk,
                0,
                true,
            );
        }
    }
}
