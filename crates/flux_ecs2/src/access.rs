use crate::ComponentKey;

/// One declared access: a component and whether it is written.
#[derive(Copy, Clone, Debug)]
pub struct AccessTerm {
    pub key: ComponentKey,
    pub write: bool,
}

impl AccessTerm {
    /// Placeholder for unused list slots; never read.
    const FILLER: AccessTerm = AccessTerm {
        key: ComponentKey::from_path(""),
        write: false,
    };
}

/// Maximum number of terms in one [`AccessList`].
pub const MAX_ACCESS: usize = 32;

/// A fixed-capacity list of access terms, buildable and checkable in const
/// context.
///
/// Two terms conflict when they name the same component and at least one of
/// them writes; duplicate reads are allowed.
#[derive(Copy, Clone, Debug)]
pub struct AccessList {
    terms: [AccessTerm; MAX_ACCESS],
    len: usize,
}

impl AccessList {
    /// The list with no terms.
    pub const EMPTY: Self = Self {
        terms: [AccessTerm::FILLER; MAX_ACCESS],
        len: 0,
    };

    /// A list with one read of `key`.
    pub const fn read(key: ComponentKey) -> Self {
        Self::single(key, false)
    }

    /// A list with one write of `key`.
    pub const fn write(key: ComponentKey) -> Self {
        Self::single(key, true)
    }

    const fn single(key: ComponentKey, write: bool) -> Self {
        let mut list = Self::EMPTY;
        list.terms[0] = AccessTerm { key, write };
        list.len = 1;
        list
    }

    /// Both lists' terms, `self`'s first, in order.
    ///
    /// # Panics
    ///
    /// If the combined length exceeds [`MAX_ACCESS`]. In const context this
    /// is a compile error.
    pub const fn concat(self, other: Self) -> Self {
        assert!(
            self.len + other.len <= MAX_ACCESS,
            "access list capacity exceeded"
        );
        let mut out = self;
        let mut i = 0;
        while i < other.len {
            out.terms[out.len] = other.terms[i];
            out.len += 1;
            i += 1;
        }
        out
    }

    /// Whether any two of this list's own terms conflict.
    pub const fn self_conflicting(&self) -> bool {
        let mut i = 0;
        while i < self.len {
            let mut j = i + 1;
            while j < self.len {
                if Self::conflicting(&self.terms[i], &self.terms[j]) {
                    return true;
                }
                j += 1;
            }
            i += 1;
        }
        false
    }

    /// Whether any term of `self` conflicts with any term of `other`.
    pub const fn conflicts_with(&self, other: &Self) -> bool {
        let mut i = 0;
        while i < self.len {
            let mut j = 0;
            while j < other.len {
                if Self::conflicting(&self.terms[i], &other.terms[j]) {
                    return true;
                }
                j += 1;
            }
            i += 1;
        }
        false
    }

    const fn conflicting(a: &AccessTerm, b: &AccessTerm) -> bool {
        a.key == b.key && (a.write || b.write)
    }

    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// The live terms.
    pub const fn terms(&self) -> &[AccessTerm] {
        self.terms.split_at(self.len).0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::ComponentKey;

    const A: ComponentKey = ComponentKey::from_path("access::tests::A");
    const B: ComponentKey = ComponentKey::from_path("access::tests::B");
    const C: ComponentKey = ComponentKey::from_path("access::tests::C");

    // ------------------------------------------------------------ construction

    #[test]
    fn empty_list() {
        assert_eq!(AccessList::EMPTY.len(), 0);
        assert!(AccessList::EMPTY.is_empty());
        assert!(!AccessList::EMPTY.self_conflicting());
        assert!(AccessList::EMPTY.terms().is_empty());
    }

    #[test]
    fn single_term_lists() {
        let r = AccessList::read(A);
        let w = AccessList::write(A);
        assert_eq!(r.len(), 1);
        assert!(!r.terms()[0].write);
        assert!(w.terms()[0].write);
        assert!(r.terms()[0].key == A);
        assert!(!r.self_conflicting());
        assert!(
            !w.self_conflicting(),
            "a single write conflicts with nothing"
        );
    }

    #[test]
    fn concat_preserves_order_and_length() {
        let list = AccessList::read(A)
            .concat(AccessList::write(B))
            .concat(AccessList::read(C));
        assert_eq!(list.len(), 3);
        let keys: Vec<_> = list.terms().iter().map(|t| t.key).collect();
        assert!(keys[0] == A && keys[1] == B && keys[2] == C);
        assert!(!list.terms()[0].write && list.terms()[1].write && !list.terms()[2].write);
    }

    #[test]
    fn concat_with_empty_is_identity() {
        let list = AccessList::write(A).concat(AccessList::EMPTY);
        assert_eq!(list.len(), 1);
        let list = AccessList::EMPTY.concat(AccessList::write(A));
        assert_eq!(list.len(), 1);
        assert!(list.terms()[0].write);
    }

    #[test]
    #[should_panic(expected = "capacity")]
    fn concat_past_capacity_panics() {
        let mut list = AccessList::EMPTY;
        for _ in 0..=MAX_ACCESS {
            list = list.concat(AccessList::read(A));
        }
    }

    // ---------------------------------------------------------- self conflicts

    #[test]
    fn duplicate_reads_do_not_conflict() {
        let list = AccessList::read(A).concat(AccessList::read(A));
        assert!(!list.self_conflicting(), "shared reads alias freely");
    }

    #[test]
    fn write_write_on_same_key_conflicts() {
        let list = AccessList::write(A).concat(AccessList::write(A));
        assert!(list.self_conflicting());
    }

    #[test]
    fn read_write_on_same_key_conflicts_in_both_orders() {
        assert!(
            AccessList::read(A)
                .concat(AccessList::write(A))
                .self_conflicting()
        );
        assert!(
            AccessList::write(A)
                .concat(AccessList::read(A))
                .self_conflicting()
        );
    }

    #[test]
    fn distinct_keys_never_self_conflict() {
        let list = AccessList::write(A)
            .concat(AccessList::write(B))
            .concat(AccessList::write(C));
        assert!(!list.self_conflicting());
    }

    #[test]
    fn conflict_hiding_at_the_tail_is_found() {
        // Off-by-one guard: the conflicting pair is the last two terms.
        let list = AccessList::read(A)
            .concat(AccessList::read(B))
            .concat(AccessList::read(C))
            .concat(AccessList::write(C));
        assert!(list.self_conflicting());
    }

    // --------------------------------------------------------- cross conflicts

    #[test]
    fn cross_conflicts_require_a_shared_key_and_a_write() {
        let reads = AccessList::read(A).concat(AccessList::read(B));
        let writes_c = AccessList::write(C);
        let writes_a = AccessList::write(A);
        assert!(!reads.conflicts_with(&writes_c), "disjoint keys");
        assert!(reads.conflicts_with(&writes_a), "read vs write on A");
        assert!(writes_a.conflicts_with(&reads), "symmetric");
        assert!(!reads.conflicts_with(&reads), "read sets never conflict");
        assert!(writes_a.conflicts_with(&writes_a), "two writers of A");
    }

    #[test]
    fn empty_conflicts_with_nothing() {
        let w = AccessList::write(A);
        assert!(!AccessList::EMPTY.conflicts_with(&w));
        assert!(!w.conflicts_with(&AccessList::EMPTY));
    }

    // ------------------------------------------------------------ const context

    #[test]
    fn everything_works_in_const_context() {
        const LIST: AccessList = AccessList::read(A).concat(AccessList::write(B));
        const SELF_OK: bool = LIST.self_conflicting();
        const CROSS: bool = LIST.conflicts_with(&AccessList::read(B));
        const _: () = assert!(!SELF_OK);
        const _: () = assert!(CROSS);
        const _: () = assert!(AccessList::EMPTY.is_empty());
        // the shape the query machinery relies on:
        const { assert!(!LIST.self_conflicting()) }
    }
}
