//! Transform propagation down a deep hierarchy: a serial recursive walk
//! against the per-depth parallel pass. The ratio is the propagation speedup.
//!
//! The tree is built wide rather than bushy — a bounded number of parents per
//! level, each with a large fan-out — because every distinct parent is its own
//! archetype (and so its own chunk). A balanced fine-grained tree of this size
//! would allocate one 64 KiB chunk per interior node.

use criterion::{Criterion, criterion_group, criterion_main};
use flux_ecs::{ChildOf, Component, Entity, World};
use std::collections::HashMap;
use std::hint::black_box;

type Mat4 = [f32; 16];

const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0, //
];

#[inline]
fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [0.0f32; 16];
    for row in 0..4 {
        for col in 0..4 {
            let mut sum = 0.0;
            for k in 0..4 {
                sum += a[row * 4 + k] * b[k * 4 + col];
            }
            out[row * 4 + col] = sum;
        }
    }
    out
}

#[derive(Component, Copy, Clone)]
struct Local(Mat4);

#[derive(Component, Copy, Clone)]
struct Global(Mat4);

const LEVELS: usize = 10;
const PARENTS_PER_LEVEL: usize = 64;
const FANOUT: usize = 1800;

/// A `LEVELS`-deep tree: `PARENTS_PER_LEVEL` interior nodes at each level, each
/// with `FANOUT` children; the first `PARENTS_PER_LEVEL` children carry the
/// tree deeper. Returns the world, the roots, and a parent→children map for
/// the recursive baseline.
fn build() -> (World, Vec<Entity>, HashMap<Entity, Vec<Entity>>) {
    let mut world = World::new();
    let mut children: HashMap<Entity, Vec<Entity>> = HashMap::new();

    let local = |i: usize| {
        let t = (i % 97) as f32 * 0.01;
        let mut m = IDENTITY;
        m[3] = t;
        m[7] = t * 0.5;
        Local(m)
    };

    let mut counter = 0usize;
    let mut spawn = |world: &mut World| {
        counter += 1;
        world.spawn((local(counter), Global(IDENTITY)))
    };

    let roots: Vec<Entity> = (0..PARENTS_PER_LEVEL).map(|_| spawn(&mut world)).collect();
    let mut parents = roots.clone();
    for _ in 1..LEVELS {
        let mut produced: Vec<Entity> = Vec::with_capacity(PARENTS_PER_LEVEL * FANOUT);
        for &parent in &parents {
            let mut kids = Vec::with_capacity(FANOUT);
            for _ in 0..FANOUT {
                let child = spawn(&mut world);
                world.relate::<ChildOf>(child, parent);
                kids.push(child);
                produced.push(child);
            }
            children.insert(parent, kids);
        }
        parents = produced[..PARENTS_PER_LEVEL].to_vec();
    }

    (world, roots, children)
}

/// Serial depth-first propagation via the children map.
fn walk(world: &mut World, children: &HashMap<Entity, Vec<Entity>>, node: Entity, parent: Mat4) {
    let local = world.get::<Local>(node).expect("node has Local").0;
    let global = mul(&parent, &local);
    world.get_mut::<Global>(node).expect("node has Global").0 = global;
    if let Some(kids) = children.get(&node) {
        for &child in kids {
            walk(world, children, child, global);
        }
    }
}

fn propagation(c: &mut Criterion) {
    let (mut world, roots, children) = build();

    let mut group = c.benchmark_group("hierarchy");
    group.sample_size(20);

    group.bench_function("recursive_walk_1M", |b| {
        b.iter(|| {
            for &root in &roots {
                walk(&mut world, &children, root, IDENTITY);
            }
            black_box(world.get::<Global>(roots[0]));
        });
    });

    group.bench_function("parallel_depth_1M", |b| {
        b.iter(|| {
            world.propagate::<Local, Global>(
                |l| Global(mul(&IDENTITY, &l.0)),
                |pw, l| Global(mul(&pw.0, &l.0)),
            );
            black_box(world.get::<Global>(roots[0]));
        });
    });

    group.finish();
}

criterion_group!(benches, propagation);
criterion_main!(benches);
