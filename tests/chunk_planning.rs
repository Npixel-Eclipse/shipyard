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

#[test]
fn or_propagates_empty_proofs_and_skips_each_sources_clean_chunks() {
    use shipyard::iter::{IntoShiperator, ShiperatorCaptain};
    let (world, ids) = setup(4097);
    world.run(|values: View<Value>, markers: View<Marker>| {
        assert_eq!(
            (values.inserted() | values.modified()).iter().size_hint(),
            (0, Some(0))
        );
        assert_eq!(
            (&ids[..], values.inserted() | values.modified(), &markers)
                .iter()
                .size_hint(),
            (0, Some(0))
        );
    });
    modify(&world, &ids, &[4096]);
    world.run(|values: View<Value>| {
        let (mut captain, _, _) =
            (values.inserted() | values.modified()).into_shiperator(&mut Default::default());
        assert!(!captain.has_no_candidates());
        assert!(captain.next_possible(0) >= ids.len());
        captain.next_slice();
        assert_eq!(captain.next_possible(0), 4096);
        assert_eq!(captain.previous_possible(4096), 0);
        assert_eq!(captain.previous_possible(4097), 4097);
    });
}

#[derive(Component)]
#[track(All)]
struct Other(usize);

#[derive(Component)]
#[track(All)]
struct Third;

fn setup_or() -> (World, Vec<EntityId>, Vec<EntityId>) {
    let mut world = World::new();
    let ids: Vec<_> = (0..4097)
        .map(|i| world.add_entity((Value(i), Third)))
        .collect();
    for (i, &id) in ids.iter().enumerate().rev() {
        world.add_component(id, (Other(i), Marker));
    }
    world.clear_all_inserted_and_modified();
    world.run(|_: ViewMut<Value>, _: ViewMut<Other>, _: ViewMut<Third>| {});
    modify(&world, &ids, &[3, 64, 4096]);
    world.run(|mut other: ViewMut<Other>, mut third: ViewMut<Third>| {
        for i in [64, 70, 4000] {
            (&mut other).get(ids[i]).unwrap().modify(|_| {});
        }
        for i in [3, 100, 4096] {
            (&mut third).get(ids[i]).unwrap().modify(|_| {});
        }
    });
    let expected = [3, 64, 4096, 4000, 70, 100].map(|i| ids[i]).to_vec();
    (world, ids, expected)
}

#[test]
fn nested_or_keeps_union_precedence_ids_reverse_and_partial_consumption() {
    let (world, ids, expected) = setup_or();
    world.run(
        |values: View<Value>, other: View<Other>, third: View<Third>, markers: View<Marker>| {
            let query = || values.inserted() | (values.modified(), &markers);
            // Mixed used as an OR branch must check its tracking predicate during
            // entity membership probes as well as when it drives iteration.
            assert_eq!(
                query()
                    .iter()
                    .with_id()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>(),
                [ids[3], ids[64], ids[4096]]
            );
            let query = || values.modified() | (other.modified() | third.modified());
            assert_eq!(
                query()
                    .iter()
                    .with_id()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                query()
                    .iter()
                    .with_id()
                    .rev()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>(),
                expected.iter().rev().copied().collect::<Vec<_>>()
            );
            assert_eq!(
                ((values.modified() | other.modified()) | third.modified())
                    .iter()
                    .with_id()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                query()
                    .iter()
                    .with_id()
                    .fold(Vec::new(), |mut out, (id, _)| {
                        out.push(id);
                        out
                    }),
                expected
            );
            assert_eq!(
                query()
                    .iter()
                    .with_id()
                    .rfold(Vec::new(), |mut out, (id, _)| {
                        out.push(id);
                        out
                    }),
                expected.iter().rev().copied().collect::<Vec<_>>()
            );
            let mut iter = query().iter().with_id();
            let mut remaining = std::collections::VecDeque::from(expected.clone());
            while !remaining.is_empty() {
                assert_eq!(iter.next().map(|(id, _)| id), remaining.pop_front());
                assert_eq!(iter.next_back().map(|(id, _)| id), remaining.pop_back());
            }
            assert!(iter.next().is_none());
            assert!(iter.next_back().is_none());
            let forced: Vec<_> = ids.iter().rev().copied().collect();
            let actual: Vec<_> = (&forced[..], query()).iter().map(|(id, _)| id).collect();
            assert_eq!(
                actual,
                forced
                    .iter()
                    .copied()
                    .filter(|id| expected.contains(id))
                    .collect::<Vec<_>>()
            );
            let mixed_union = values.modified() | (other.inserted() | (other.modified(), &markers));
            assert_eq!(
                mixed_union
                    .iter()
                    .with_id()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>(),
                expected[..5]
            );
        },
    );
}

#[cfg(feature = "parallel")]
#[test]
fn parallel_or_preserves_sources_deduplication_and_mutable_tracking() {
    use rayon::prelude::*;
    use shipyard::iter::OneOfTwo;
    for workers in [1, 4, 16] {
        let (world, ids, mut expected) = setup_or();
        expected.sort_unstable();
        rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .unwrap()
            .install(|| {
                world.run(
                    |values: View<Value>,
                     other: View<Other>,
                     third: View<Third>,
                     markers: View<Marker>| {
                        let query = || values.modified() | (other.modified() | third.modified());
                        let mut actual: Vec<_> = (query(), &markers)
                            .par_iter()
                            .with_id()
                            .map(|(id, _)| id)
                            .collect();
                        actual.sort_unstable();
                        assert_eq!(actual, expected);
                        let mut actual: Vec<_> = ((values.modified() | other.modified())
                            | third.modified())
                        .par_iter()
                        .with_id()
                        .map(|(id, _)| id)
                        .collect();
                        actual.sort_unstable();
                        assert_eq!(actual, expected);
                        assert_eq!((values.inserted() | other.inserted()).par_iter().count(), 0);
                    },
                );
                world.run(|mut values: ViewMut<Value>, mut other: ViewMut<Other>| {
                    use rayon::iter::plumbing::UnindexedProducer;
                    // Keep cross-source probes from racing with mutable timestamps.
                    let (producer, right) = (values.modified_mut() | other.modified_mut())
                        .iter()
                        .split();
                    assert!(right.is_none());
                    drop(producer);
                    (values.modified_mut() | other.modified_mut())
                        .par_iter()
                        .with_id()
                        .for_each(|(id, value)| match value {
                            OneOfTwo::One(mut value) => {
                                assert_eq!(id, ids[value.0]);
                                value.modify(|v| v.0 += 10000);
                            }
                            OneOfTwo::Two(mut value) => {
                                assert_eq!(id, ids[value.0]);
                                value.modify(|v| v.0 += 10000);
                            }
                        });
                    for i in [3, 64, 4096] {
                        assert_eq!(values.get(ids[i]).unwrap().0, i + 10000);
                    }
                    for i in [70, 4000] {
                        assert_eq!(other.get(ids[i]).unwrap().0, i + 10000);
                    }
                    assert_eq!(other.get(ids[64]).unwrap().0, 64);
                });
            });
    }
}

#[test]
fn or_union_matrix_preserves_empty_overlap_optional_not_and_iteration_order() {
    use shipyard::iter::Optional;
    let patterns = [
        vec![],
        vec![0],
        vec![64, 128],
        vec![0, 64, 128],
        (0..129).collect(),
    ];
    for left_changed in &patterns {
        for right_changed in &patterns {
            let mut world = World::new();
            let ids: Vec<_> = (0..129).map(|i| world.add_entity(Value(i))).collect();
            for i in (0..129).rev() {
                world.add_component(ids[i], Other(i));
            }
            world.clear_all_inserted_and_modified();
            world.run(|_: ViewMut<Value>, _: ViewMut<Other>| {});
            modify(&world, &ids, left_changed);
            world.run(|mut other: ViewMut<Other>| {
                for &i in right_changed {
                    (&mut other).get(ids[i]).unwrap().modify(|_| {});
                }
            });
            let expected: Vec<_> = left_changed
                .iter()
                .copied()
                .chain(
                    right_changed
                        .iter()
                        .rev()
                        .copied()
                        .filter(|i| !left_changed.contains(i)),
                )
                .map(|i| ids[i])
                .collect();
            world.run(
                |values: View<Value>, other: View<Other>, third: View<Third>| {
                    let query = || values.inserted_or_modified() | other.modified();
                    assert_eq!(
                        query()
                            .iter()
                            .with_id()
                            .map(|(id, _)| id)
                            .collect::<Vec<_>>(),
                        expected
                    );
                    assert_eq!(
                        query()
                            .iter()
                            .with_id()
                            .rev()
                            .map(|(id, _)| id)
                            .collect::<Vec<_>>(),
                        expected.iter().rev().copied().collect::<Vec<_>>()
                    );
                    assert_eq!((query(), Optional(&third)).iter().count(), expected.len());
                    let mut actual: Vec<_> = (query(), !values.modified())
                        .iter()
                        .with_id()
                        .map(|(id, _)| id)
                        .collect();
                    actual.sort_unstable();
                    let mut right_only: Vec<_> = right_changed
                        .iter()
                        .filter(|i| !left_changed.contains(i))
                        .map(|&i| ids[i])
                        .collect();
                    right_only.sort_unstable();
                    assert_eq!(actual, right_only);
                    let mut iter = query().iter().with_id();
                    let mut remaining = std::collections::VecDeque::from(expected.clone());
                    while !remaining.is_empty() {
                        assert_eq!(iter.next_back().map(|(id, _)| id), remaining.pop_back());
                        assert_eq!(iter.next().map(|(id, _)| id), remaining.pop_front());
                    }
                    assert!(iter.next().is_none());
                    assert!(iter.next_back().is_none());
                    #[cfg(feature = "parallel")]
                    {
                        use rayon::prelude::*;
                        let mut parallel: Vec<_> =
                            query().par_iter().with_id().map(|(id, _)| id).collect();
                        parallel.sort_unstable();
                        let mut sorted = expected.clone();
                        sorted.sort_unstable();
                        assert_eq!(parallel, sorted);
                    }
                },
            );
        }
    }
}

#[test]
fn planning_budget_boundary_keeps_small_joins_bounded_and_large_sparse_joins_planned() {
    for driver_len in [0, 64, 128, 1563, 1564, 3000] {
        let mut world = World::new();
        let ids: Vec<_> = world.bulk_add_entity((0..50_000).map(Value)).collect();
        for &id in ids.iter().rev().take(driver_len) {
            world.add_component(id, Marker);
        }
        world.clear_all_inserted_and_modified();
        world.run(|_: ViewMut<Value>| {});
        let planned = driver_len >= 1564; // 782 chunks fit driver_len / 2.
        world.run(|values: View<Value>, markers: View<Marker>| {
            assert_eq!(
                (values.modified(), &markers).iter().size_hint(),
                (0, Some(if planned { 0 } else { driver_len }))
            );
            assert_eq!((values.modified(), &markers).iter().count(), 0);
        });
        modify(&world, &ids, &[49_996, 49_999]);
        world.run(|values: View<Value>, markers: View<Marker>| {
            let actual: Vec<_> = (values.modified(), &markers)
                .iter()
                .with_id()
                .map(|(id, (value, _))| {
                    assert_eq!(id, ids[value.0]);
                    value.0
                })
                .collect();
            let expected = if driver_len == 0 {
                vec![]
            } else if planned {
                vec![49_996, 49_999]
            } else {
                vec![49_999, 49_996]
            };
            assert_eq!(actual, expected);
            assert_eq!(
                (values.modified(), &markers)
                    .iter()
                    .rev()
                    .map(|(v, _)| v.0)
                    .collect::<Vec<_>>(),
                expected.iter().rev().copied().collect::<Vec<_>>()
            );
            #[cfg(feature = "parallel")]
            {
                use rayon::prelude::*;
                let mut actual: Vec<_> = (values.modified(), &markers)
                    .par_iter()
                    .map(|(v, _)| v.0)
                    .collect();
                let mut expected = expected;
                actual.sort_unstable();
                expected.sort_unstable();
                assert_eq!(actual, expected);
            }
        });
    }
}

#[test]
fn planning_budget_propagates_through_nested_queries_and_respects_forced_sources() {
    use shipyard::iter::Optional;
    let mut world = World::new();
    let ids: Vec<_> = world.bulk_add_entity((0..50_000).map(Value)).collect();
    for (i, &id) in ids.iter().enumerate().rev() {
        world.add_component(id, Other(i));
    }
    for &id in ids.iter().rev().take(64) {
        world.add_component(id, Marker);
    }
    world.clear_all_inserted_and_modified();
    world.run(|_: ViewMut<Value>, _: ViewMut<Other>| {});
    world.run(
        |values: View<Value>, other: View<Other>, markers: View<Marker>, third: View<Third>| {
            // Child tuples and ORs must not replace the parent's 32-chunk budget
            // with their own much larger local budget.
            assert_eq!(
                (&markers, (values.modified(), &other)).iter().size_hint(),
                (0, Some(64))
            );
            assert_eq!(
                (&markers, (values.modified(),)).iter().size_hint(),
                (0, Some(64))
            );
            assert_eq!(
                (
                    &markers,
                    values.inserted() | (other.inserted() | values.modified())
                )
                    .iter()
                    .size_hint(),
                (0, Some(64))
            );
            // A mandatory entity slice overrides the shorter Marker storage. Its
            // 1500-chunk budget allows the large tracking input's empty proof.
            let requested = &ids[47_000..];
            assert_eq!(
                (requested, &markers, values.modified()).iter().size_hint(),
                (0, Some(0))
            );
            // Optional/Not component presence cannot become a zero-length driver.
            assert_eq!(
                (requested, values.modified(), Optional(&third), !&third)
                    .iter()
                    .size_hint(),
                (0, Some(0))
            );
            assert_eq!(
                (requested, values.inserted() | other.modified())
                    .iter()
                    .size_hint(),
                (0, Some(0))
            );
        },
    );
    modify(&world, &ids, &[49_999, 49_998]);
    world.run(
        |mut values: ViewMut<Value>, other: View<Other>, markers: View<Marker>| {
            // Skipping a mutable tracking plan must retain exact per-entity checks.
            let actual: Vec<_> = (values.modified_mut(), &markers)
                .iter()
                .with_id()
                .map(|(id, (mut value, _))| {
                    value.modify(|v| v.0 += 1);
                    id
                })
                .collect();
            assert_eq!(actual, [ids[49_999], ids[49_998]]);
            let actual: Vec<_> = (
                &markers,
                values.inserted() | (other.inserted() | values.modified()),
            )
                .iter()
                .with_id()
                .map(|(id, _)| id)
                .collect();
            assert_eq!(actual, [ids[49_999], ids[49_998]]);
        },
    );
}

#[test]
fn planning_budget_uses_all_or_sources_and_recognizes_not_tracking_drivers() {
    let mut world = World::new();
    let ids: Vec<_> = world.bulk_add_entity((0..50_000).map(Value)).collect();
    for (i, &id) in ids.iter().take(64).enumerate() {
        world.add_component(id, Other(i));
    }
    world.clear_all_inserted_and_modified();
    world.run(|_: ViewMut<Value>, _: ViewMut<Other>| {});
    world.run(|values: View<Value>, other: View<Other>| {
        // Standalone OR must budget for both 64 and 50,000 slots, not only 64.
        assert_eq!(
            (other.inserted() | values.modified()).iter().size_hint(),
            (0, Some(0))
        );
        // Unlike !&view, !view.modified() can drive a query over its storage.
        assert_eq!(
            (values.modified(), !other.modified()).iter().size_hint(),
            (0, Some(64))
        );
        assert_eq!((values.modified(), !other.modified()).iter().count(), 0);
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
fn plain_mutable_or_splits_and_visits_each_entity_once() {
    use rayon::{iter::plumbing::UnindexedProducer, prelude::*};
    use shipyard::iter::OneOfTwo;
    let mut world = World::new();
    let ids: Vec<_> = (0..1025).map(|i| world.add_entity(Value(i))).collect();
    for i in (512..1025).rev() {
        world.add_component(ids[i], Other(i));
    }
    let extra = world.add_entity(Other(1025));
    world.run(|mut values: ViewMut<Value>, mut other: ViewMut<Other>| {
        let (left, right) = (&mut values | &mut other).iter().split();
        assert!(right.is_some());
        drop((left, right));
        let mut found: Vec<_> = (&mut values | &mut other)
            .par_iter()
            .with_id()
            .map(|(id, item)| {
                match item {
                    OneOfTwo::One(mut value) => value.modify(|v| v.0 += 2000),
                    OneOfTwo::Two(mut value) => value.modify(|v| v.0 += 2000),
                }
                id
            })
            .collect();
        found.sort_unstable();
        let mut expected = ids.clone();
        expected.push(extra);
        expected.sort_unstable();
        assert_eq!(found, expected);
        assert_eq!(values.get(ids[512]).unwrap().0, 2512);
        assert_eq!(other.get(ids[512]).unwrap().0, 512);
        assert_eq!(other.get(extra).unwrap().0, 3025);
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
