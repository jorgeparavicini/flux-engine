//! Query iteration against the hand-written speed-of-light baseline.
//! The ratio between the two is the number that matters.

use criterion::{Criterion, criterion_group, criterion_main};
use flux_ecs::{Component, QueryState, World};
use std::hint::black_box;

const N: u64 = 1_000_000;

#[derive(Component)]
struct Position(f32);

#[derive(Component)]
struct Velocity(f32);

fn simple_iter(c: &mut Criterion) {
    let mut world = World::new();
    for i in 0..N {
        world.spawn((Position(i as f32), Velocity((i % 7) as f32)));
    }
    let mut state = QueryState::<(&mut Position, &Velocity)>::new();
    let dt = 1.0 / 60.0_f32;

    c.bench_function("query/simple_iter_1M", |b| {
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

criterion_group!(benches, simple_iter);
criterion_main!(benches);
