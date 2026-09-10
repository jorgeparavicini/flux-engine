//! Iteration cost under archetype fragmentation: the same entity total
//! spread across 1, 8, 64, and 512 archetypes.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use flux_ecs2::{Component, Entity, QueryState, World};
use std::hint::black_box;

const N: u64 = 100_000;

#[derive(Component)]
struct Position(f32);

#[derive(Component)]
struct Velocity(f32);

macro_rules! markers {
    ($($m:ident),+) => {
        $(
            #[derive(Component)]
            struct $m;
        )+
        /// Adds a distinct marker combination for `bits`, fragmenting the
        /// archetype space.
        fn add_markers(world: &mut World, entity: Entity, bits: u32) {
            let mut bit = 0;
            $(
                if bits & (1 << bit) != 0 {
                    world.insert(entity, $m);
                }
                bit += 1;
            )+
            let _ = bit;
        }
    };
}
markers!(M0, M1, M2, M3, M4, M5, M6, M7, M8);

fn fragmentation(c: &mut Criterion) {
    let mut group = c.benchmark_group("query/fragmented_iter_100k");
    for archetypes in [1u32, 8, 64, 512] {
        let mut world = World::new();
        for i in 0..N {
            let e = world.spawn((Position(i as f32), Velocity((i % 7) as f32)));
            add_markers(&mut world, e, (i as u32) % archetypes);
        }
        let mut state = QueryState::<(&mut Position, &Velocity)>::new();
        let dt = 1.0 / 60.0_f32;
        group.bench_with_input(BenchmarkId::from_parameter(archetypes), &archetypes, |b, _| {
            b.iter(|| {
                for (pos, vel) in world.query(&mut state).chunks() {
                    for (p, v) in pos.iter_mut().zip(vel.iter()) {
                        p.0 += v.0 * dt;
                    }
                }
                black_box(&world);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, fragmentation);
criterion_main!(benches);
