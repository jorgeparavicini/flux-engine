//! Differential test: the real implementation against the naive reference.
//!
//! Random op sequences run against both sides; observable state must agree
//! after every op. Ops address entities by index into the log of every entity
//! ever created, so stale handles are exercised constantly.

use flux_ecs2::Entities;
use flux_ecs2::reference::RefWorld;
use proptest::prelude::*;

#[derive(Debug, Clone)]
enum Op {
    Spawn,
    Despawn(usize),
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => Just(Op::Spawn),
        2 => (0usize..256).prop_map(Op::Despawn),
    ]
}

/// Applies one op to both worlds and asserts they agree on the result.
struct Driver {
    real: Entities,
    reference: RefWorld,
    log: Vec<flux_ecs2::Entity>,
}

impl Driver {
    fn new() -> Self {
        Self { real: Entities::new(), reference: RefWorld::new(), log: Vec::new() }
    }

    fn apply(&mut self, op: &Op) -> Result<(), TestCaseError> {
        match op {
            Op::Spawn => {
                let a = self.real.alloc();
                let b = self.reference.spawn();
                prop_assert_eq!(a, b, "both sides must hand out identical entity ids");
                self.log.push(a);
            }
            Op::Despawn(i) => {
                if self.log.is_empty() {
                    return Ok(());
                }
                let e = self.log[i % self.log.len()];
                prop_assert_eq!(self.real.dealloc(e), self.reference.despawn(e), "despawn({:?})", e);
            }
        }
        Ok(())
    }

    fn check_agreement(&self) -> Result<(), TestCaseError> {
        for e in &self.log {
            prop_assert_eq!(
                self.real.is_alive(*e),
                self.reference.is_alive(*e),
                "liveness of {:?} diverged",
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
    fn entity_ops_agree_with_reference(ops in prop::collection::vec(op_strategy(), 1..400)) {
        let mut d = Driver::new();
        for op in &ops {
            d.apply(op)?;
            d.check_agreement()?;
        }
    }
}

/// Deterministic long-run variant (10⁵ ops).
/// Uses a fixed xorshift stream so failures reproduce exactly; runs a reduced
/// count under miri, where full agreement checks would take hours.
#[test]
fn one_hundred_thousand_entity_ops() {
    const OPS: usize = if cfg!(miri) { 500 } else { 100_000 };
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
        let op = if r % 5 < 3 {
            Op::Spawn
        } else {
            Op::Despawn((r >> 32) as usize)
        };
        d.apply(&op).unwrap_or_else(|e| panic!("divergence at step {step}: {e}"));
        // Full agreement sweep every 64 ops keeps the run O(n·log-size/64);
        // per-op sweeps would be O(n²) at this scale.
        if step % 64 == 0 {
            d.check_agreement().unwrap_or_else(|e| panic!("divergence by step {step}: {e}"));
        }
    }
    d.check_agreement().expect("final agreement");
    assert!(d.log.len() > OPS / 2, "op mix should skew toward spawns");
}
