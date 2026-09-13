//! Intra-system scaling: the same 1M-entity per-row workload run serially
//! (`for_each`) and split across cores (`par_for_each`). The ratio is the
//! intra-system speedup.

use criterion::{Criterion, criterion_group, criterion_main};
use flux_ecs::{Component, QueryState, World};
use std::hint::black_box;

const N: u64 = 1_000_000;

#[derive(Component)]
struct Position([f32; 4]);

#[derive(Component)]
struct Velocity([f32; 4]);

/// Non-trivial per-row work so the split has something to amortize over.
#[inline]
fn integrate(p: &mut Position, v: &Velocity) {
    for k in 0..4 {
        let mut x = p.0[k] + v.0[k];
        // heavier compute per row to expose compute-bound scaling
        for _ in 0..8 {
            x = x.sin().cos().mul_add(0.5, x);
        }
        p.0[k] = x;
    }
}

fn scaling(c: &mut Criterion) {
    let mut world = World::new();
    for i in 0..N {
        let f = i as f32;
        world.spawn((Position([f; 4]), Velocity([(i % 7) as f32; 4])));
    }
    let mut serial_state = QueryState::<(&mut Position, &Velocity)>::new();
    let mut par_state = QueryState::<(&mut Position, &Velocity)>::new();

    c.bench_function("parallel/serial_for_each_1M", |b| {
        b.iter(|| {
            world
                .query(&mut serial_state)
                .for_each(|(p, v)| integrate(p, v));
            black_box(&world);
        });
    });

    c.bench_function("parallel/par_for_each_1M", |b| {
        b.iter(|| {
            world
                .query(&mut par_state)
                .par_for_each(|(p, v)| integrate(p, v));
            black_box(&world);
        });
    });
}

criterion_group!(benches, scaling);
criterion_main!(benches);
