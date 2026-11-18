#![cfg(feature = "parallel")]

use rayon::{prelude::*, ThreadPoolBuilder};
use shipyard::{
    iter::OneOfTwo,
    track::{All, Untracked},
    Component, EntityId, Get, IntoIter, View, ViewMut, World,
};
use std::error::Error;

type TestResult = Result<(), Box<dyn Error>>;

struct Value(usize);
impl Component for Value {
    type Tracking = All;
}

struct Marker;
impl Component for Marker {
    type Tracking = Untracked;
}

fn fixture(len: usize) -> (World, Vec<EntityId>) {
    let mut world = World::new();
    let ids: Vec<_> = (0..len).map(|i| world.add_entity(Value(i))).collect();
    for &id in ids.iter().rev() {
        world.add_component(id, Marker);
    }
    world.clear_all_inserted_and_modified();
    world.run(|_values: ViewMut<Value>| {});
    (world, ids)
}

fn partitions<I: ParallelIterator>(iter: I) -> Vec<Vec<I::Item>> {
    iter.fold(Vec::new, |mut items, item| {
        items.push(item);
        items
    })
    .collect()
}

#[test]
fn threshold_stops_at_the_boundary_and_preserves_ordinary_results() -> TestResult {
    let pool = ThreadPoolBuilder::new().num_threads(4).build()?;
    for len in [0, 1, 8, 9] {
        let (world, _) = fixture(len);
        let values = world.borrow::<View<Value>>()?;
        pool.install(|| {
            let default = partitions(values.par_iter().map(|v| v.0));
            assert_eq!(default.len(), 1);
            assert_eq!(default[0], (0..len).collect::<Vec<_>>());

            let iter = values.par_iter().with_split_threshold(8);
            assert_eq!(iter.opt_len(), Some(len));
            let explicit = partitions(iter.map(|v| v.0));
            assert_eq!(explicit.len() == 1, len <= 8);
            let mut actual: Vec<_> = explicit.into_iter().flatten().collect();
            actual.sort_unstable();
            assert_eq!(actual, (0..len).collect::<Vec<_>>());
            assert_eq!(values.par_iter().with_split_threshold(8).count(), len);
        });
    }
    Ok(())
}

#[test]
fn last_threshold_wins_across_with_id_without_changing_other_iterators() -> TestResult {
    let pool = ThreadPoolBuilder::new().num_threads(4).build()?;
    let (world, ids) = fixture(33);
    let values = world.borrow::<View<Value>>()?;
    pool.install(|| {
        let small = partitions(
            values
                .par_iter()
                .with_split_threshold(usize::MAX)
                .with_id()
                .with_split_threshold(8)
                .map(|(id, value)| (id, value.0)),
        );
        assert!(small.len() > 1);
        let mut actual: Vec<_> = small.into_iter().flatten().collect();
        actual.sort_unstable();
        assert_eq!(
            actual,
            ids.iter().copied().zip(0..ids.len()).collect::<Vec<_>>()
        );

        let large = partitions(
            values
                .par_iter()
                .with_split_threshold(8)
                .with_split_threshold(usize::MAX)
                .with_id()
                .map(|(id, value)| (id, value.0)),
        );
        assert_eq!(large.len(), 1);
        assert_eq!(large[0], actual);
        assert!(partitions(values.par_iter()).len() > 1);
    });
    Ok(())
}

#[test]
#[should_panic(expected = "split threshold must be greater than zero")]
fn zero_threshold_panics_before_consumption() {
    let (world, _) = fixture(0);
    world.run(|values: View<Value>| {
        let _ = values.par_iter().with_split_threshold(0);
    });
}

#[test]
#[should_panic(expected = "split threshold must be greater than zero")]
fn zero_threshold_with_id_panics_before_consumption() {
    let (world, _) = fixture(0);
    world.run(|values: View<Value>| {
        let _ = values.par_iter().with_id().with_split_threshold(0);
    });
}

#[test]
fn threshold_counts_sparse_tracking_candidates_instead_of_dense_span() -> TestResult {
    let pool = ThreadPoolBuilder::new().num_threads(4).build()?;
    let (world, ids) = fixture(4097);
    {
        let mut values = world.borrow::<ViewMut<Value>>()?;
        for index in [70, 4096] {
            (&mut values).get(ids[index])?.modify(|_| {});
        }
    }
    let values = world.borrow::<View<Value>>()?;
    pool.install(|| {
        // One full candidate chunk plus the final one-slot chunk.
        let bounded = partitions(
            values
                .modified()
                .par_iter()
                .with_split_threshold(65)
                .map(|v| v.0),
        );
        assert_eq!(bounded, vec![vec![70, 4096]]);
        let split = partitions(
            values
                .modified()
                .par_iter()
                .with_split_threshold(8)
                .with_id()
                .map(|(id, v)| (id, v.0)),
        );
        assert!(split.len() > 1);
        let mut actual: Vec<_> = split.into_iter().flatten().collect();
        actual.sort_unstable();
        assert_eq!(actual, vec![(ids[70], 70), (ids[4096], 4096)]);
        assert!(partitions(values.modified().par_iter()).len() > 1);
    });
    Ok(())
}

#[test]
fn mutable_joins_preserve_ids_tracking_and_exactly_once_updates() -> TestResult {
    let pool = ThreadPoolBuilder::new().num_threads(4).build()?;
    for threshold in [1, 8, usize::MAX] {
        let (world, ids) = fixture(257);
        let (mut values, markers) = world.borrow::<(ViewMut<Value>, View<Marker>)>()?;
        pool.install(|| {
            (&markers, &mut values)
                .par_iter()
                .with_split_threshold(threshold)
                .with_id()
                .for_each(|(id, (_, mut value))| {
                    assert_eq!(id, ids[value.0]);
                    value.modify(|v| v.0 += ids.len());
                });
            let mut modified: Vec<_> = (&ids[..], values.modified())
                .par_iter()
                .with_split_threshold(threshold)
                .map(|(id, value)| (id, value.0))
                .collect();
            modified.sort_unstable();
            assert_eq!(
                modified,
                ids.iter()
                    .copied()
                    .zip(ids.len()..ids.len() * 2)
                    .collect::<Vec<_>>()
            );
        });
    }
    Ok(())
}

#[test]
fn explicit_threshold_keeps_mutable_or_membership_on_one_producer() -> TestResult {
    let pool = ThreadPoolBuilder::new().num_threads(4).build()?;
    let (world, ids) = fixture(257);
    let (mut values, markers) = world.borrow::<(ViewMut<Value>, View<Marker>)>()?;
    for mut value in (&mut values).iter() {
        value.modify(|_| {});
    }
    pool.install(|| {
        let output = partitions(
            (values.modified_mut() | &markers)
                .par_iter()
                .with_split_threshold(1)
                .with_id()
                .map(|(id, item)| {
                    let OneOfTwo::One(mut value) = item else {
                        panic!("every entity must be yielded by the left source once");
                    };
                    assert_eq!(id, ids[value.0]);
                    value.modify(|v| v.0 += ids.len());
                    id
                }),
        );
        assert_eq!(output, vec![ids.clone()]);
    });
    assert_eq!(values.modified().iter().count(), ids.len());
    for (&id, expected) in ids.iter().zip(ids.len()..ids.len() * 2) {
        assert_eq!(values.get(id)?.0, expected);
    }
    Ok(())
}
