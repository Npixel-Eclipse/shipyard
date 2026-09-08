//! Isolated iterator throughput probe, not a Battle FPS benchmark.
//! `cargo run --release -p shipyard --example chunk_planning -- 16 1000`
use rayon::prelude::*;
use shipyard::{track, Component, Get, IntoIter, View, ViewMut, World};
use std::{hint::black_box, time::Instant};

struct Value(usize);
impl Component for Value {
    type Tracking = track::All;
}
struct Marker;
impl Component for Marker {
    type Tracking = track::Untracked;
}

fn main() {
    let threads = std::env::args().nth(1).map_or(16, |v| v.parse().unwrap());
    let iterations = std::env::args()
        .nth(2)
        .map_or(1000, |v| v.parse::<usize>().unwrap());
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap();
    for (name, len, kind) in [
        ("empty", 65536, 0),
        ("sparse_tail", 65536, 1),
        ("sparse_tail_work", 65536, 5),
        ("sparse_scattered", 65536, 2),
        ("sparse_fragmented", 65536, 6),
        ("dense", 65536, 3),
        ("small", 64, 3),
        ("untracked_join", 65536, 4),
    ] {
        let mut world = World::new();
        let ids: Vec<_> = (0..len)
            .map(|i| world.add_entity((Value(i), Marker)))
            .collect();
        world.clear_all_inserted_and_modified();
        world.run(|_values: ViewMut<Value>| {});
        let indices: Vec<_> = (0..len)
            .filter(|&i| match kind {
                0 => false,
                1 | 5 => i >= len - 1024,
                2 => i % 64 == 0,
                6 => i % 128 == 0,
                _ => true,
            })
            .collect();
        world.run(|mut values: ViewMut<Value>| {
            for &i in &indices {
                (&mut values).get(ids[i]).unwrap().modify(|_| {});
            }
        });
        let expected = if kind == 4 {
            (0..len).sum::<usize>()
        } else {
            indices.iter().sum::<usize>()
        };
        pool.install(|| world.run(|markers: View<Marker>, values: View<Value>| {
            let query = || {
                let sum: usize = if kind == 4 {
                    (&markers, &values).par_iter().map(|(_, v)| consume(v.0, kind == 5)).sum()
                } else {
                    (&markers, values.modified()).par_iter().map(|(_, v)| consume(v.0, kind == 5)).sum()
                };
                assert_eq!(sum, expected);
                black_box(sum);
            };
            for _ in 0..20 { query(); }
            let mut samples = Vec::new();
            for _ in 0..5 {
                let started = Instant::now();
                for _ in 0..iterations { query(); }
                samples.push(started.elapsed().as_nanos() as f64 / iterations as f64);
            }
            samples.sort_by(f64::total_cmp);
            println!("{name},threads={threads},slots={len},matches={},median_ns={:.0},min_ns={:.0},max_ns={:.0}",
                indices.len(), samples[2], samples[0], samples[4]);
        }));
    }
}

#[inline]
fn consume(value: usize, work: bool) -> usize {
    if work {
        let mut hash = value;
        for _ in 0..64 {
            hash ^= hash >> 7;
            hash = hash.wrapping_mul(0x9e3779b97f4a7c15);
        }
        black_box(hash);
    }
    black_box(value)
}
