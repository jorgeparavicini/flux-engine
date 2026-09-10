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

criterion_group!(benches, baseline);
criterion_main!(benches);
