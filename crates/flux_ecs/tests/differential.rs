//! Differential test: the real implementation against the naive reference.
//!
//! Random op sequences run against both sides; observable state must agree
//! after every op. Ops address entities by index into the log of every entity
//! ever created, so stale handles are exercised constantly.

use flux_ecs::{Component, World};
use flux_ecs::reference::RefWorld;
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

#[derive(Debug, Clone, Copy)]
enum Op {
    Spawn(Shape),
    Despawn(usize),
    InsertA(usize, u32),
    InsertB(usize, i64),
    RemoveA(usize),
    RemoveB(usize),
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
