//! Speed-of-light baseline: a hand-written SoA loop over plain `Vec`s.
//! Every iteration benchmark in this crate reports its ratio against this
//! number; the ratio is the figure that matters, not the absolute time.

use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;

const N: usize = 1_000_000;

fn baseline(c: &mut Criterion) {
    let mut pos: Vec<f32> = (0..N).map(|i| i as f32).collect();
    let vel: Vec<f32> = (0..N).map(|i| (i % 7) as f32).collect();
    let dt = 1.0 / 60.0_f32;

    c.bench_function("baseline/simple_iter_1M", |b| {
        b.iter(|| {
            for (p, v) in pos.iter_mut().zip(vel.iter()) {
                *p += v * dt;
            }
            black_box(&mut pos);
        });
    });
}

fn chunk_layout_baseline(c: &mut Criterion) {
    // The same loop over memory laid out the way chunked storage lays it out:
    // repeating CHUNK-sized blocks of [entity ids | positions | velocities].
    // This is the speed of light *given the storage model*; the flat baseline
    // above is the speed of light given none.
    const CHUNK: usize = 64 * 1024;
    const CAP: usize = CHUNK / 16; // stride: 8 (entity) + 4 + 4
    let chunks = N.div_ceil(CAP);
    let mut memory = vec![1u8; chunks * CHUNK];
    let dt = 1.0 / 60.0_f32;
    c.bench_function("baseline/chunk_layout_1M", |b| {
        b.iter(|| {
            for i in 0..chunks {
                let base = i * CHUNK;
                unsafe {
                    let pos = std::slice::from_raw_parts_mut(
                        memory.as_mut_ptr().add(base + 8 * CAP).cast::<f32>(),
                        CAP,
                    );
                    let vel = std::slice::from_raw_parts(
                        memory.as_ptr().add(base + 12 * CAP).cast::<f32>(),
                        CAP,
                    );
                    for (p, v) in pos.iter_mut().zip(vel.iter()) {
                        *p += v * dt;
                    }
                }
            }
            black_box(&mut memory);
        });
    });
}

criterion_group!(benches, baseline, chunk_layout_baseline);
criterion_main!(benches);
