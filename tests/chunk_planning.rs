use shipyard::{track, Component, EntityId, Get, IntoIter, View, ViewMut, World};

#[derive(Debug)]
struct Value(usize);
impl Component for Value {
    type Tracking = track::All;
}

struct Marker;
impl Component for Marker {
    type Tracking = track::Untracked;
}

fn setup(len: usize) -> (World, Vec<EntityId>) {
    let mut world = World::new();
    let ids: Vec<_> = (0..len).map(|i| world.add_entity(Value(i))).collect();
    // Deliberately different dense orders. A tracking captain must carry the
    // correct entity pointer and index, not the previous untracked captain's.
    for &id in ids.iter().rev() {
        world.add_component(id, Marker);
    }
    world.clear_all_inserted_and_modified();
    world.run(|_values: ViewMut<Value>| {});
    (world, ids)
}

fn modify(world: &World, ids: &[EntityId], indices: &[usize]) {
    world.run(|mut values: ViewMut<Value>| {
        for &index in indices {
            (&mut values).get(ids[index]).unwrap().modify(|_| {});
        }
    });
}

#[test]
fn sparse_metadata_selects_tracking_captain_and_keeps_ids_with_values() {
    let (world, ids) = setup(4096);
    modify(&world, &ids, &[0, 65, 4095]);
    world.run(|markers: View<Marker>, values: View<Value>| {
        let actual: Vec<_> = (&markers, values.modified())
            .iter()
            .with_id()
            .map(|(id, (_, v))| (id, v.0))
            .collect();
        assert_eq!(actual, vec![(ids[0], 0), (ids[65], 65), (ids[4095], 4095)]);
        // A full tracking input is more expensive than a plain input of equal length.
        let all: Vec<_> = (&markers, &values)
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(all, ids.iter().rev().copied().collect::<Vec<_>>());
    });
}

#[test]
fn forced_entity_slice_captain_preserves_order_and_tracking_filter() {
    let (world, ids) = setup(4096);
    modify(&world, &ids, &[3, 70, 2000]);
    let requested = [ids[2000], ids[4], ids[70], ids[3]];
    world.run(|markers: View<Marker>, values: View<Value>| {
        let actual: Vec<_> = (&requested[..], &markers, values.modified())
            .iter()
            .map(|(id, _, value)| (id, value.0))
            .collect();
        assert_eq!(actual, vec![(ids[2000], 2000), (ids[70], 70), (ids[3], 3)]);
    });
}

#[test]
fn reverse_and_partial_iteration_check_tracking_captain_and_ids() {
    let (world, ids) = setup(1025);
    modify(&world, &ids, &[0, 65, 1024]);
    world.run(|markers: View<Marker>, values: View<Value>| {
        let mut iter = (&markers, values.modified()).iter().with_id();
        assert_eq!(iter.next().unwrap().0, ids[0]);
        assert_eq!(iter.next_back().unwrap().0, ids[1024]);
        assert_eq!(iter.next().unwrap().0, ids[65]);
        assert!(iter.next_back().is_none());
        let reverse: Vec<_> = (&markers, values.modified())
            .iter()
            .with_id()
            .rev()
            .map(|(id, (_, value))| (id, value.0))
            .collect();
        assert_eq!(reverse, vec![(ids[1024], 1024), (ids[65], 65), (ids[0], 0)]);
        assert_eq!(
            (&markers, values.modified())
                .iter()
                .rev()
                .map(|(_, v)| v.0)
                .collect::<Vec<_>>(),
            vec![1024, 65, 0]
        );
    });
}

#[test]
fn tracking_window_changes_rebuild_the_plan() {
    let (world, ids) = setup(512);
    modify(&world, &ids, &[5]);
    let boundary = world.get_tracking_timestamp();
    world.run(|_values: ViewMut<Value>| {});
    modify(&world, &ids, &[400]);
    world.run(|mut values: View<Value>| {
        assert_eq!(
            values.modified().iter().map(|v| v.0).collect::<Vec<_>>(),
            vec![5, 400]
        );
        values.override_last_modification(boundary);
        assert_eq!(
            values.modified().iter().map(|v| v.0).collect::<Vec<_>>(),
            vec![400]
        );
    });
}

#[test]
fn insertion_or_modification_is_a_union_and_mutable_iteration_updates_values() {
    let (mut world, ids) = setup(4096);
    modify(&world, &ids, &[7]);
    let inserted = world.add_entity(Value(4096));
    world.run(|mut values: ViewMut<Value>| {
        let actual: Vec<_> = values
            .inserted_or_modified()
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(actual, vec![ids[7], inserted]);
        for mut value in values.inserted_or_modified_mut().iter() {
            value.modify(|v| v.0 += 1);
        }
        assert_eq!(values.get(ids[7]).unwrap().0, 8);
        assert_eq!(values.get(inserted).unwrap().0, 4097);
    });
}

#[test]
fn empty_tracking_does_not_prune_negation_optional_or_union() {
    use shipyard::iter::Optional;
    let (world, ids) = setup(512);
    world.run(|markers: View<Marker>, values: View<Value>| {
        assert_eq!((&markers, !values.modified()).iter().count(), ids.len());
        assert_eq!((&markers, Optional(&values)).iter().count(), ids.len());
        assert_eq!((&markers | values.modified()).iter().count(), ids.len());
        assert_eq!(
            (values.inserted() | &markers).iter().with_id().fold(
                Vec::new(),
                |mut found, (id, _)| {
                    found.push(id);
                    found
                }
            ),
            ids.iter().rev().copied().collect::<Vec<_>>()
        );
        assert_eq!(
            (&markers, (values.inserted() | values.modified()))
                .iter()
                .count(),
            0
        );
    });
}

#[cfg(feature = "parallel")]
#[test]
fn required_empty_input_prevents_splitting_even_with_a_forced_captain() {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (world, ids) = setup(4096);
    rayon::ThreadPoolBuilder::new()
        .num_threads(16)
        .build()
        .unwrap()
        .install(|| {
            world.run(|markers: View<Marker>, values: View<Value>| {
                let initialized = AtomicUsize::new(0);
                let count: usize = (&ids[..], &markers, values.modified())
                    .par_iter()
                    .fold(
                        || {
                            initialized.fetch_add(1, Ordering::Relaxed);
                            0
                        },
                        |n, _| n + 1,
                    )
                    .sum();
                assert_eq!(count, 0);
                assert_eq!(initialized.load(Ordering::Relaxed), 1);
                let count: usize = (&markers, values.modified())
                    .par_iter()
                    .with_id()
                    .fold(
                        || {
                            initialized.fetch_add(1, Ordering::Relaxed);
                            0
                        },
                        |n, _| n + 1,
                    )
                    .sum();
                assert_eq!(count, 0);
                assert_eq!(initialized.load(Ordering::Relaxed), 2);
            });
        });
}

#[cfg(feature = "parallel")]
#[test]
fn sparse_tail_splits_by_candidate_work_instead_of_dense_midpoint() {
    use rayon::iter::plumbing::UnindexedProducer;
    let (world, ids) = setup(65536);
    let changed: Vec<_> = (65280..65536).collect();
    modify(&world, &ids, &changed);
    world.run(|values: View<Value>| {
        let (left, right) = values.modified().iter().split();
        assert_eq!(
            left.map(|v| v.0).collect::<Vec<_>>(),
            (65280..65408).collect::<Vec<_>>()
        );
        assert_eq!(
            right.unwrap().map(|v| v.0).collect::<Vec<_>>(),
            (65408..65536).collect::<Vec<_>>()
        );
    });
}

#[cfg(feature = "parallel")]
#[test]
fn parallel_plans_cover_each_matching_entity_once_for_dense_sparse_and_mutable_inputs() {
    use rayon::prelude::*;
    for step in [1, 3, 64, 128, 257] {
        let (world, ids) = setup(4097);
        let indices: Vec<_> = (0..ids.len()).step_by(step).collect();
        modify(&world, &ids, &indices);
        for threads in [1, 4, 16] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    world.run(|markers: View<Marker>, mut values: ViewMut<Value>| {
                        let mut actual: Vec<_> = (&markers, values.modified())
                            .par_iter()
                            .with_id()
                            .map(|(id, (_, value))| {
                                assert_eq!(id, ids[value.0]);
                                value.0
                            })
                            .collect();
                        actual.sort_unstable();
                        assert_eq!(actual, indices);
                        let count = std::sync::atomic::AtomicUsize::new(0);
                        values
                            .modified_mut()
                            .par_iter()
                            .with_id()
                            .for_each(|(id, mut value)| {
                                assert_eq!(id, ids[value.0]);
                                value.modify(|_| {});
                                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            });
                        assert_eq!(
                            count.load(std::sync::atomic::Ordering::Relaxed),
                            indices.len()
                        );
                    });
                });
        }
    }
}
