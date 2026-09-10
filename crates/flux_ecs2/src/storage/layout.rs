use crate::Entity;
use crate::registry::{ComponentId, Registry};
use crate::storage::alloc::{CHUNK_ALIGN, CHUNK_SIZE};

/// Offset sentinel for zero-sized components: present in the signature,
/// but owning no bytes in the chunk. Never a valid offset, since real
/// offsets are bounded by `CHUNK_SIZE`.
pub(crate) const NO_COLUMN: u32 = u32::MAX;

/// Byte map of one archetype's chunks: where each SoA column starts and how
/// many rows fit. Computed once per archetype; identical for all its chunks.
pub struct ArchetypeLayout {
    /// Rows per chunk; ≥ 1.
    pub capacity: u16,
    /// The archetype signature: component ids, sorted ascending, ZSTs included.
    pub components: Box<[ComponentId]>,
    /// Column start offsets, parallel to `components`; `NO_COLUMN` for ZSTs.
    pub offsets: Box<[u32]>,
    /// Start of the `Entity` id column.
    pub entity_offset: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LayoutError {
    /// One row of this archetype does not fit in a chunk.
    EntityTooLarge,
}

/// A column awaiting placement: `index` is the component's position in the
/// sorted id list (`usize::MAX` for the entity column), so the computed
/// offset lands in the right `offsets` slot regardless of placement order.
struct Column {
    index: usize,
    size: usize,
    align: usize,
}

const ENTITY_COLUMN: usize = usize::MAX;

impl ArchetypeLayout {
    /// Computes the chunk layout for the archetype with signature `ids`.
    ///
    /// Places one column per sized component plus the `Entity` id column into
    /// a [`CHUNK_SIZE`] chunk, maximizing the number of rows. Every column
    /// start is aligned to its component's alignment, given a chunk base
    /// aligned to [`CHUNK_ALIGN`]. Zero-sized components remain part of the
    /// signature but receive [`NO_COLUMN`].
    ///
    /// Every id must be registered in `reg`, and `ids` must be sorted
    /// ascending without duplicates.
    ///
    /// # Errors
    ///
    /// [`LayoutError::EntityTooLarge`] if a single row does not fit in a chunk.
    ///
    /// # Panics
    ///
    /// If a component's alignment exceeds [`CHUNK_ALIGN`].
    pub fn new(ids: &[ComponentId], reg: &Registry) -> Result<Self, LayoutError> {
        debug_assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "component ids must be sorted and unique"
        );

        // Single pass over the registry: gather placeable columns and the row
        // stride together.
        let mut columns = Vec::with_capacity(ids.len() + 1);
        columns.push(Column {
            index: ENTITY_COLUMN,
            size: size_of::<Entity>(),
            align: align_of::<Entity>(),
        });
        let mut stride = size_of::<Entity>();
        for (index, id) in ids.iter().enumerate() {
            let info = reg.info(*id);
            assert!(
                info.align <= CHUNK_ALIGN,
                "component `{}` alignment {} exceeds the chunk alignment {}",
                info.name,
                info.align,
                CHUNK_ALIGN
            );
            if info.size == 0 {
                continue;
            }
            columns.push(Column { index, size: info.size, align: info.align });
            stride += info.size;
        }

        let mut capacity = CHUNK_SIZE / stride;
        if capacity == 0 {
            return Err(LayoutError::EntityTooLarge);
        }

        // Descending alignment; the stable sort keeps the entity column first
        // among equal alignments. With sizes always multiples of aligns, this
        // order never needs padding, so the first capacity fits and the retry
        // below is a safety net for any future change of placement rule.
        columns.sort_by_key(|c| std::cmp::Reverse(c.align));

        let mut offsets = vec![NO_COLUMN; ids.len()];
        let mut entity_offset = 0u32;
        loop {
            let mut cursor = 0usize;
            let mut fits = true;
            for column in &columns {
                cursor = cursor.next_multiple_of(column.align);
                if cursor + column.size * capacity > CHUNK_SIZE {
                    fits = false;
                    break;
                }
                if column.index == ENTITY_COLUMN {
                    entity_offset = cursor as u32;
                } else {
                    offsets[column.index] = cursor as u32;
                }
                cursor += column.size * capacity;
            }
            if fits {
                break;
            }
            capacity -= 1; // unreachable under the current placement rule
        }

        Ok(Self {
            capacity: u16::try_from(capacity).expect("capacity bounded by CHUNK_SIZE / stride"),
            components: ids.to_owned().into_boxed_slice(),
            offsets: offsets.into_boxed_slice(),
            entity_offset,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{Component, ComponentKey, StorageClass};
    use crate::entity::Entity;
    use crate::registry::{ComponentId, Registry};
    use crate::storage::alloc::CHUNK_SIZE;
    use proptest::prelude::*;

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("layout::tests::", stringify!($name)));
            }
        };
    }

    // Size/align menagerie. Rust guarantees size is a multiple of align.
    struct B1(#[allow(dead_code)] u8);
    component!(B1);
    struct B2(#[allow(dead_code)] u16);
    component!(B2);
    struct B3(#[allow(dead_code)] [u8; 3]);
    component!(B3);
    struct B4(#[allow(dead_code)] u32);
    component!(B4);
    struct B8(#[allow(dead_code)] u64);
    component!(B8);
    struct B12(#[allow(dead_code)] [u32; 3]);
    component!(B12);
    #[repr(align(16))]
    struct A16(#[allow(dead_code)] [u8; 16]);
    component!(A16);
    #[repr(align(64))]
    struct A64(#[allow(dead_code)] [u8; 64]);
    component!(A64);
    struct Zst;
    component!(Zst);
    struct Tagged;
    impl Component for Tagged {
        const KEY: ComponentKey = ComponentKey::from_path("layout::tests::Tagged");
        const STORAGE: StorageClass = StorageClass::Tag;
    }
    struct Huge(#[allow(dead_code)] [u8; 70_000]);
    component!(Huge);
    struct NearChunk(#[allow(dead_code)] [u8; 65_520]);
    component!(NearChunk);
    struct JustOver(#[allow(dead_code)] [u8; 65_529]);
    component!(JustOver);
    #[repr(align(128))]
    struct A128(#[allow(dead_code)] [u8; 128]);
    component!(A128);

    /// Registers the full menagerie; returns (registry, name→id lookup by KEY).
    fn registry() -> Registry {
        let mut r = Registry::new();
        r.register::<B1>();
        r.register::<B2>();
        r.register::<B3>();
        r.register::<B4>();
        r.register::<B8>();
        r.register::<B12>();
        r.register::<A16>();
        r.register::<A64>();
        r.register::<Zst>();
        r.register::<Tagged>();
        r
    }

    fn ids(r: &mut Registry, keys: &[ComponentKey]) -> Vec<ComponentId> {
        // Look ids up via a fresh sorted list; test helper only.
        let mut out: Vec<ComponentId> = keys
            .iter()
            .map(|k| {
                (0..r.len() as u32)
                    .map(ComponentId)
                    .find(|id| r.info(*id).key == *k)
                    .expect("registered")
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// Checks every structural invariant a layout must satisfy.
    fn assert_valid(layout: &ArchetypeLayout, r: &Registry) {
        assert!(layout.capacity >= 1);
        let cap = layout.capacity as usize;

        // Collect (offset, extent, align) for the entity column + every real column.
        let mut spans: Vec<(usize, usize, usize)> = vec![(
            layout.entity_offset as usize,
            size_of::<Entity>() * cap,
            align_of::<Entity>(),
        )];
        assert_eq!(layout.components.len(), layout.offsets.len(), "parallel arrays");
        for (id, &off) in layout.components.iter().zip(layout.offsets.iter()) {
            let info = r.info(*id);
            if info.size == 0 {
                assert_eq!(off, NO_COLUMN, "zero-sized components get no column");
                continue;
            }
            assert_ne!(off, NO_COLUMN, "sized components get a real column");
            spans.push((off as usize, info.size * cap, info.align));
        }
        for &(off, extent, align) in &spans {
            assert_eq!(off % align, 0, "column at {off} misaligned for align {align}");
            assert!(off + extent <= CHUNK_SIZE, "column at {off} overruns the chunk");
        }
        spans.sort();
        for pair in spans.windows(2) {
            assert!(
                pair[0].0 + pair[0].1 <= pair[1].0,
                "columns overlap: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
        // components stay sorted (they are the signature)
        assert!(layout.components.windows(2).all(|w| w[0] < w[1]));
    }

    // ------------------------------------------------------------ basic shapes

    #[test]
    fn entity_only_archetype() {
        let mut r = registry();
        let layout = ArchetypeLayout::new(&[], &r).unwrap();
        assert_valid(&layout, &r);
        assert_eq!(
            layout.capacity as usize,
            CHUNK_SIZE / size_of::<Entity>(),
            "capacity bounded by the entity column alone"
        );
        let _ = &mut r;
    }

    #[test]
    fn single_u32_component_reaches_5461_rows() {
        // stride = 8 (Entity) + 4 = 12; 65536 / 12 = 5461. Pins maximality:
        // an off-by-one in the capacity solve moves this number.
        let mut r = registry();
        let sig = ids(&mut r, &[B4::KEY]);
        let layout = ArchetypeLayout::new(&sig, &r).unwrap();
        assert_valid(&layout, &r);
        assert_eq!(layout.capacity, 5461);
    }

    #[test]
    fn align16_component_reaches_2730_rows() {
        // stride = 8 + 16 = 24; 65536 / 24 = 2730.
        let mut r = registry();
        let sig = ids(&mut r, &[A16::KEY]);
        let layout = ArchetypeLayout::new(&sig, &r).unwrap();
        assert_valid(&layout, &r);
        assert_eq!(layout.capacity, 2730);
    }

    #[test]
    fn mixed_alignment_archetype_is_valid_and_tight() {
        let mut r = registry();
        let sig = ids(&mut r, &[B1::KEY, B2::KEY, B4::KEY, B8::KEY, B12::KEY, A16::KEY, A64::KEY]);
        let layout = ArchetypeLayout::new(&sig, &r).unwrap();
        assert_valid(&layout, &r);
        // stride = 8+1+2+4+8+12+16+64 = 115; loose lower bound allows for
        // per-column padding of at most (align-1) each.
        let stride: usize = 115;
        let slack: usize = [4usize, 1, 2, 4, 8, 4, 16, 64].iter().map(|a| a - 1).sum();
        assert!(layout.capacity as usize >= (CHUNK_SIZE - slack) / stride);
        assert!(layout.capacity as usize <= CHUNK_SIZE / stride);
    }

    // ---------------------------------------------------------------- ZST / Tag

    #[test]
    fn zsts_occupy_no_space_and_do_not_reduce_capacity() {
        let mut r = registry();
        let with = ArchetypeLayout::new(&ids(&mut r, &[B4::KEY, Zst::KEY, Tagged::KEY]), &r).unwrap();
        let without = ArchetypeLayout::new(&ids(&mut r, &[B4::KEY]), &r).unwrap();
        assert_valid(&with, &r);
        assert_eq!(with.capacity, without.capacity, "ZSTs must not cost capacity");
        assert_eq!(with.components.len(), 3, "ZSTs stay in the signature");
    }

    // ----------------------------------------------------- parallel-array order

    #[test]
    fn offsets_are_indexed_by_sorted_id_not_by_placement() {
        // B1 (align 1) registered before B8 (align 8), so B1 has the lower id
        // and comes first in `components` — but descending-align placement puts
        // B8's column physically first. offsets[] must follow id order.
        let mut fresh = Registry::new();
        let low_align_low_id = fresh.register::<B1>();
        let high_align_high_id = fresh.register::<B8>();
        assert!(low_align_low_id < high_align_high_id);

        let layout = ArchetypeLayout::new(&[low_align_low_id, high_align_high_id], &fresh).unwrap();
        assert_valid(&layout, &fresh);
        assert_eq!(layout.components[0], low_align_low_id);
        assert!(
            layout.offsets[0] > layout.offsets[1],
            "B1's column (id-first) must sit physically after B8's higher-aligned column"
        );
    }

    // -------------------------------------------------------------- edge sizes

    #[test]
    fn near_chunk_sized_component_fits_exactly_one_row() {
        // 8 + 65520 = 65528 ≤ 65536 → capacity 1.
        let mut fresh = Registry::new();
        let id = fresh.register::<NearChunk>();
        let layout = ArchetypeLayout::new(&[id], &fresh).unwrap();
        assert_valid(&layout, &fresh);
        assert_eq!(layout.capacity, 1);
    }

    #[test]
    fn component_that_cannot_fit_one_row_is_an_error() {
        let mut fresh = Registry::new();
        let just_over = fresh.register::<JustOver>(); // 8 + 65529 > 65536
        let huge = fresh.register::<Huge>();
        assert!(matches!(
            ArchetypeLayout::new(&[just_over], &fresh),
            Err(LayoutError::EntityTooLarge)
        ));
        assert!(matches!(
            ArchetypeLayout::new(&[huge], &fresh),
            Err(LayoutError::EntityTooLarge)
        ));
    }

    // ---------------------------------------------------------------- contracts

    #[test]
    #[should_panic(expected = "align")]
    fn alignment_beyond_chunk_align_is_rejected() {
        let mut fresh = Registry::new();
        let id = fresh.register::<A128>();
        let _ = ArchetypeLayout::new(&[id], &fresh);
    }

    #[test]
    #[should_panic(expected = "sorted")]
    #[cfg(debug_assertions)]
    fn unsorted_input_panics_in_debug() {
        let mut fresh = Registry::new();
        let a = fresh.register::<B1>();
        let b = fresh.register::<B8>();
        let _ = ArchetypeLayout::new(&[b, a], &fresh);
    }

    #[test]
    #[should_panic(expected = "sorted")]
    #[cfg(debug_assertions)]
    fn duplicate_ids_panic_in_debug() {
        let mut fresh = Registry::new();
        let a = fresh.register::<B4>();
        let _ = ArchetypeLayout::new(&[a, a], &fresh);
    }

    // ---------------------------------------------------------------- property

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]
        #[test]
        #[cfg_attr(miri, ignore = "proptest is too slow under miri; deterministic tests cover the invariants")]
        fn every_subset_of_the_menagerie_yields_a_valid_layout(mask in 0u16..1024) {
            let mut r = registry();
            let all = [B1::KEY, B2::KEY, B3::KEY, B4::KEY, B8::KEY, B12::KEY, A16::KEY, A64::KEY, Zst::KEY, Tagged::KEY];
            let picked: Vec<_> = all.iter().enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, k)| *k)
                .collect();
            let sig = ids(&mut r, &picked);
            let layout = ArchetypeLayout::new(&sig, &r).unwrap();
            assert_valid(&layout, &r);
        }
    }
}
