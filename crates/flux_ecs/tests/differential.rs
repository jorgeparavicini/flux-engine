//! Differential test: the real implementation against the naive reference.
//!
//! Random op sequences run against both sides; observable state must agree
//! after every op. Ops address entities by index into the log of every entity
//! ever created, so stale handles are exercised constantly.

use flux_ecs::reference::RefWorld;
use flux_ecs::{ChildOf, Component, World};
use proptest::prelude::*;

#[derive(Component, Copy, Clone, PartialEq, Debug)]
struct Da(u32);

#[derive(Component, Copy, Clone, PartialEq, Debug)]
struct Db(i64);

#[derive(Debug, Clone, Copy)]
enum Shape {
    Empty,
    A(u32),
    B(i64),
    Both(u32, i64),
}

#[derive(Debug, Clone)]
enum Op {
    Spawn(Shape),
    Despawn(usize),
    InsertA(usize, u32),
    InsertB(usize, i64),
    RemoveA(usize),
    RemoveB(usize),
    InsertBatchA(Vec<(usize, u32)>),
    Relate(usize, usize),
    Unrelate(usize),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    let shape = prop_oneof![
        Just(Shape::Empty),
        any::<u32>().prop_map(Shape::A),
        any::<i64>().prop_map(Shape::B),
        (any::<u32>(), any::<i64>()).prop_map(|(a, b)| Shape::Both(a, b)),
    ];
    prop_oneof![
        3 => shape.prop_map(Op::Spawn),
        2 => (0usize..256).prop_map(Op::Despawn),
        2 => (0usize..256, any::<u32>()).prop_map(|(i, v)| Op::InsertA(i, v)),
        2 => (0usize..256, any::<i64>()).prop_map(|(i, v)| Op::InsertB(i, v)),
        1 => (0usize..256).prop_map(Op::RemoveA),
        1 => (0usize..256).prop_map(Op::RemoveB),
        2 => prop::collection::vec((0usize..256, any::<u32>()), 1..8).prop_map(Op::InsertBatchA),
        2 => (0usize..256, 0usize..256).prop_map(|(c, p)| Op::Relate(c, p)),
        1 => (0usize..256).prop_map(Op::Unrelate),
    ]
}

/// Applies one op to both worlds and asserts they agree on the result.
struct Driver {
    real: World,
    reference: RefWorld,
    log: Vec<flux_ecs::Entity>,
}

impl Driver {
    fn new() -> Self {
        Self { real: World::new(), reference: RefWorld::new(), log: Vec::new() }
    }

    fn pick(&self, i: usize) -> Option<flux_ecs::Entity> {
        self.log.get(i % self.log.len().max(1)).copied()
    }

    fn apply(&mut self, op: &Op) -> Result<(), TestCaseError> {
        match *op {
            Op::Spawn(shape) => {
                let real = match shape {
                    Shape::Empty => self.real.spawn(()),
                    Shape::A(a) => self.real.spawn(Da(a)),
                    Shape::B(b) => self.real.spawn(Db(b)),
                    Shape::Both(a, b) => self.real.spawn((Da(a), Db(b))),
                };
                let reference = self.reference.spawn();
                match shape {
                    Shape::Empty => {}
                    Shape::A(a) => {
                        self.reference.insert(reference, Da(a));
                    }
                    Shape::B(b) => {
                        self.reference.insert(reference, Db(b));
                    }
                    Shape::Both(a, b) => {
                        self.reference.insert(reference, Da(a));
                        self.reference.insert(reference, Db(b));
                    }
                }
                prop_assert_eq!(real, reference, "both sides must hand out identical ids");
                self.log.push(real);
            }
            Op::Despawn(i) => {
                if let Some(e) = self.pick(i) {
                    prop_assert_eq!(self.real.despawn(e), self.reference.despawn(e), "despawn({:?})", e);
                }
            }
            Op::InsertA(i, v) => {
                if let Some(e) = self.pick(i) {
                    prop_assert_eq!(self.real.insert(e, Da(v)), self.reference.insert(e, Da(v)));
                }
            }
            Op::InsertB(i, v) => {
                if let Some(e) = self.pick(i) {
                    prop_assert_eq!(self.real.insert(e, Db(v)), self.reference.insert(e, Db(v)));
                }
            }
            Op::RemoveA(i) => {
                if let Some(e) = self.pick(i) {
                    prop_assert_eq!(self.real.remove::<Da>(e), self.reference.remove::<Da>(e));
                }
            }
            Op::RemoveB(i) => {
                if let Some(e) = self.pick(i) {
                    prop_assert_eq!(self.real.remove::<Db>(e), self.reference.remove::<Db>(e));
                }
            }
            Op::Relate(ci, pi) => {
                if let (Some(c), Some(p)) = (self.pick(ci), self.pick(pi)) {
                    prop_assert_eq!(
                        self.real.relate::<ChildOf>(c, p),
                        self.reference.relate::<ChildOf>(c, p),
                        "relate({:?}, {:?})",
                        c,
                        p
                    );
                }
            }
            Op::Unrelate(ci) => {
                if let Some(c) = self.pick(ci) {
                    prop_assert_eq!(
                        self.real.unrelate::<ChildOf>(c),
                        self.reference.unrelate::<ChildOf>(c),
                        "unrelate({:?})",
                        c
                    );
                }
            }
            Op::InsertBatchA(ref pairs) => {
                // Resolve indices once so both sides see the same items; the
                // batched path must match per-entity inserts in the same order.
                let items: Vec<(flux_ecs::Entity, Da)> = pairs
                    .iter()
                    .filter_map(|&(i, v)| self.pick(i).map(|e| (e, Da(v))))
                    .collect();
                self.real.insert_batch(items.clone());
                for (e, v) in items {
                    self.reference.insert(e, v);
                }
            }
        }
        Ok(())
    }

    fn check_agreement(&self) -> Result<(), TestCaseError> {
        prop_assert_eq!(self.real.len(), self.reference.len(), "live entity counts diverged");
        for e in &self.log {
            prop_assert_eq!(self.real.is_alive(*e), self.reference.is_alive(*e), "liveness of {:?}", e);
            prop_assert_eq!(
                self.real.get::<Da>(*e),
                self.reference.get::<Da>(*e),
                "Da of {:?}",
                e
            );
            prop_assert_eq!(
                self.real.get::<Db>(*e),
                self.reference.get::<Db>(*e),
                "Db of {:?}",
                e
            );
            prop_assert_eq!(
                self.real.related::<ChildOf>(*e),
                self.reference.related::<ChildOf>(*e),
                "ChildOf of {:?}",
                e
            );
            prop_assert_eq!(
                self.real.depth::<ChildOf>(*e),
                self.reference.depth::<ChildOf>(*e),
                "ChildOf depth of {:?}",
                e
            );
        }
        Ok(())
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    #[cfg_attr(miri, ignore = "proptest is too slow under miri; deterministic test below covers it")]
    fn world_ops_agree_with_reference(ops in prop::collection::vec(op_strategy(), 1..300)) {
        let mut d = Driver::new();
        for op in &ops {
            d.apply(op)?;
            d.check_agreement()?;
        }
    }
}

/// Deterministic long-run variant (10⁵ ops). Fixed xorshift stream so
/// failures reproduce exactly; reduced count under miri.
#[test]
fn one_hundred_thousand_world_ops() {
    const OPS: usize = if cfg!(miri) { 300 } else { 100_000 };
    let mut rng: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };

    let mut d = Driver::new();
    for step in 0..OPS {
        let r = next();
        let i = (r >> 32) as usize;
        let op = match r % 11 {
            0 | 1 => Op::Spawn(Shape::Empty),
            2 | 3 => Op::Spawn(Shape::Both(r as u32, (r >> 16) as i64)),
            4 => Op::Spawn(Shape::A(r as u32)),
            5 => Op::Despawn(i),
            6 | 7 => Op::InsertA(i, r as u32),
            8 => Op::InsertB(i, (r >> 8) as i64),
            9 => Op::RemoveA(i),
            _ => Op::RemoveB(i),
        };
        d.apply(&op).unwrap_or_else(|e| panic!("divergence at step {step}: {e}"));
        if step % 64 == 0 {
            d.check_agreement().unwrap_or_else(|e| panic!("divergence by step {step}: {e}"));
        }
    }
    d.check_agreement().expect("final agreement");
}
