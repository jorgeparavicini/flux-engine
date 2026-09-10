use crate::access::AccessList;
use crate::storage::chunks::ChunkId;
use crate::{ComponentId, ComponentKey};

/// Capability to access component data under a declared access set.
///
/// Column accessors take a grant as evidence: reads require a matching term;
/// writes additionally require an unspent claim on that (chunk, component)
/// pair, so one grant can never mint two live mutable slices over the same
/// column.
pub struct AccessGrant {
    allowed: AccessList,
    spent: Vec<(ChunkId, ComponentId)>,
}

impl AccessGrant {
    pub fn new(allowed: AccessList) -> Self {
        Self {
            allowed,
            spent: Vec::new(),
        }
    }

    /// Whether reading `key` is allowed. Write permission implies read.
    pub fn allows_read(&self, key: ComponentKey) -> bool {
        self.allowed.terms().iter().any(|t| t.key == key)
    }

    /// Whether writing `key` is allowed.
    pub fn allows_write(&self, key: ComponentKey) -> bool {
        self.allowed.terms().iter().any(|t| t.key == key && t.write)
    }

    /// Records a mutable claim on (`chunk`, `id`); false if already claimed.
    pub(crate) fn claim_mut(&mut self, chunk: ChunkId, id: ComponentId) -> bool {
        if self.spent.contains(&(chunk, id)) {
            return false;
        }
        self.spent.push((chunk, id));
        true
    }

    /// Releases all claims on `chunk`.
    pub(crate) fn release_chunk(&mut self, chunk: ChunkId) {
        self.spent.retain(|(c, _)| *c != chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::AccessList;
    use crate::component::{Component, ComponentKey};
    use crate::entity::Entities;
    use crate::registry::{ComponentId, Registry};
    use crate::storage::alloc::ChunkAlloc;
    use crate::storage::archetype::{Archetype, ArchetypeId};
    use crate::storage::chunks::{ChunkId, Chunks};
    use crate::storage::ops;

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("grant::tests::", stringify!($name)));
            }
        };
    }

    #[derive(Copy, Clone, PartialEq, Debug)]
    struct A(u64);
    component!(A);
    #[derive(Copy, Clone, PartialEq, Debug)]
    struct B(u16);
    component!(B);

    // -------------------------------------------------------------- permissions

    #[test]
    fn read_grant_allows_read_not_write() {
        let g = AccessGrant::new(AccessList::read(A::KEY));
        assert!(g.allows_read(A::KEY));
        assert!(!g.allows_write(A::KEY));
        assert!(!g.allows_read(B::KEY));
        assert!(!g.allows_write(B::KEY));
    }

    #[test]
    fn write_grant_allows_both() {
        let g = AccessGrant::new(AccessList::write(A::KEY));
        assert!(g.allows_read(A::KEY), "write permission implies read");
        assert!(g.allows_write(A::KEY));
    }

    #[test]
    fn empty_grant_allows_nothing() {
        let g = AccessGrant::new(AccessList::EMPTY);
        assert!(!g.allows_read(A::KEY));
        assert!(!g.allows_write(A::KEY));
    }

    // ------------------------------------------------------------------- claims

    #[test]
    fn claims_are_per_chunk_per_component() {
        let mut g = AccessGrant::new(AccessList::write(A::KEY).concat(AccessList::write(B::KEY)));
        let (a, b) = (ComponentId(0), ComponentId(1));
        assert!(g.claim_mut(ChunkId(0), a));
        assert!(
            !g.claim_mut(ChunkId(0), a),
            "second claim on the same column is refused"
        );
        assert!(
            g.claim_mut(ChunkId(0), b),
            "different component, same chunk"
        );
        assert!(
            g.claim_mut(ChunkId(1), a),
            "same component, different chunk"
        );
    }

    #[test]
    fn release_frees_only_that_chunks_claims() {
        let mut g = AccessGrant::new(AccessList::write(A::KEY));
        let a = ComponentId(0);
        assert!(g.claim_mut(ChunkId(0), a));
        assert!(g.claim_mut(ChunkId(1), a));
        g.release_chunk(ChunkId(0));
        assert!(g.claim_mut(ChunkId(0), a), "released claim can be retaken");
        assert!(
            !g.claim_mut(ChunkId(1), a),
            "other chunk's claim survives the release"
        );
    }

    // ------------------------------------------------------- column accessors

    /// Storage with one archetype {A, B} and `n` rows of (A(i), B(i)).
    struct Bench {
        reg: Registry,
        chunks: Chunks,
        alloc: ChunkAlloc,
        arch: Archetype,
        chunk: ChunkId,
        a_col: usize,
        b_col: usize,
    }

    impl Bench {
        fn new(n: u64) -> Self {
            let mut reg = Registry::new();
            let a = reg.register::<A>();
            let b = reg.register::<B>();
            let mut arch = Archetype::new(&[a, b], &reg).unwrap();
            let mut chunks = Chunks::new();
            let mut alloc = ChunkAlloc::new();
            let mut entities = Entities::new();
            let mut chunk = ChunkId(0);
            let sig = arch.signature().to_vec();
            let a_col = sig.iter().position(|c| *c == a).unwrap();
            let b_col = sig.iter().position(|c| *c == b).unwrap();
            for i in 0..n {
                let e = entities.alloc();
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
            }
            Self {
                reg,
                chunks,
                alloc,
                arch,
                chunk,
                a_col,
                b_col,
            }
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

    #[test]
    fn column_yields_the_occupied_rows() {
        let bench = Bench::new(5);
        let g = AccessGrant::new(AccessList::read(A::KEY));
        let col = unsafe {
            ops::column::<A>(
                &bench.chunks,
                &bench.arch.layout,
                &bench.reg,
                bench.chunk,
                bench.a_col,
                &g,
            )
        }
            .expect("read is granted");
        assert_eq!(col.len(), 5);
        assert_eq!(col[0], A(0));
        assert_eq!(col[4], A(4));
    }

    #[test]
    fn column_requires_a_matching_term() {
        let bench = Bench::new(3);
        let g = AccessGrant::new(AccessList::read(B::KEY));
        let denied = unsafe {
            ops::column::<A>(
                &bench.chunks,
                &bench.arch.layout,
                &bench.reg,
                bench.chunk,
                bench.a_col,
                &g,
            )
        };
        assert!(
            denied.is_none(),
            "undeclared component must not be readable"
        );
    }

    #[test]
    fn column_mut_requires_write_permission() {
        let bench = Bench::new(3);
        let mut read_only = AccessGrant::new(AccessList::read(A::KEY));
        let denied = unsafe {
            ops::column_mut::<A>(
                &bench.chunks,
                &bench.arch.layout,
                &bench.reg,
                bench.chunk,
                bench.a_col,
                &mut read_only,
            )
        };
        assert!(
            denied.is_none(),
            "a read term must not mint a mutable slice"
        );
    }

    #[test]
    fn column_mut_writes_are_visible_and_claims_are_spent() {
        let bench = Bench::new(4);
        let mut g = AccessGrant::new(AccessList::write(A::KEY));

        {
            let col = unsafe {
                ops::column_mut::<A>(
                    &bench.chunks,
                    &bench.arch.layout,
                    &bench.reg,
                    bench.chunk,
                    bench.a_col,
                    &mut g,
                )
            }
                .expect("write is granted");
            col[2] = A(99);
        }
        let denied = unsafe {
            ops::column_mut::<A>(
                &bench.chunks,
                &bench.arch.layout,
                &bench.reg,
                bench.chunk,
                bench.a_col,
                &mut g,
            )
        };
        assert!(
            denied.is_none(),
            "the claim is spent: no second mutable slice"
        );

        g.release_chunk(bench.chunk);
        let col = unsafe {
            ops::column::<A>(
                &bench.chunks,
                &bench.arch.layout,
                &bench.reg,
                bench.chunk,
                bench.a_col,
                &g,
            )
        }
            .expect("read after release");
        assert_eq!(col[2], A(99), "the write landed");
    }

    #[test]
    fn shared_reads_of_one_column_coexist() {
        let bench = Bench::new(2);
        let g = AccessGrant::new(AccessList::read(A::KEY));
        let (c1, c2) = unsafe {
            (
                ops::column::<A>(
                    &bench.chunks,
                    &bench.arch.layout,
                    &bench.reg,
                    bench.chunk,
                    bench.a_col,
                    &g,
                ),
                ops::column::<A>(
                    &bench.chunks,
                    &bench.arch.layout,
                    &bench.reg,
                    bench.chunk,
                    bench.a_col,
                    &g,
                ),
            )
        };
        assert_eq!(
            c1.unwrap()[1],
            c2.unwrap()[1],
            "two shared slices may coexist"
        );
    }
}
