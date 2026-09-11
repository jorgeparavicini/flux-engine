#![allow(dead_code)]
//! Batched vs per-entity structural change: inserting a component into 1M
//! entities. The ratio is the batching speedup.

use criterion::{Criterion, criterion_group, criterion_main};
use flux_ecs::{Component, Entity, World};
use std::hint::black_box;

const N: u64 = 1_000_000;

#[derive(Component, Clone, Copy)]
struct C0([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C1([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C2([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C3([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C4([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C5([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C6([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C7([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C8([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C9([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C10([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C11([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C12([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C13([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C14([f32; 4]);
#[derive(Component, Clone, Copy)]
struct C15([f32; 4]);
#[derive(Component, Clone, Copy)]
struct Added([f32; 4]);

fn seeded() -> (World, Vec<Entity>) {
    let mut world = World::new();
    let z = [0.0f32; 4];
    let ids: Vec<Entity> = (0..N)
        .map(|_| world.spawn((C0(z), C1(z), C2(z), C3(z), C4(z), C5(z), C6(z), C7(z))))
        .collect();
    // widen the source archetype to 16 components (setup, untimed)
    world.insert_batch(ids.iter().map(|&e| (e, C8(z))));
    world.insert_batch(ids.iter().map(|&e| (e, C9(z))));
    world.insert_batch(ids.iter().map(|&e| (e, C10(z))));
    world.insert_batch(ids.iter().map(|&e| (e, C11(z))));
    world.insert_batch(ids.iter().map(|&e| (e, C12(z))));
    world.insert_batch(ids.iter().map(|&e| (e, C13(z))));
    world.insert_batch(ids.iter().map(|&e| (e, C14(z))));
    world.insert_batch(ids.iter().map(|&e| (e, C15(z))));
    (world, ids)
}

fn structural(c: &mut Criterion) {
    c.bench_function("structural/per_entity_insert_1M", |b| {
        b.iter_batched(
            seeded,
            |(mut world, ids)| {
                for e in ids {
                    world.insert(e, Added([1.0; 4]));
                }
                black_box(&world);
            },
            criterion::BatchSize::PerIteration,
        );
    });

    c.bench_function("structural/insert_batch_1M", |b| {
        b.iter_batched(
            seeded,
            |(mut world, ids)| {
                world.insert_batch(ids.into_iter().map(|e| (e, Added([1.0; 4]))));
                black_box(&world);
            },
            criterion::BatchSize::PerIteration,
        );
    });
}

criterion_group!(benches, structural);
criterion_main!(benches);
