//! Regression tests for empty required tracking joins.
#![cfg(all(feature = "parallel", feature = "proc", feature = "std"))]

use rayon::prelude::*;
use shipyard::{
    track, Component, EntityId, Get, IntoIter, Unique, UniqueViewMut, View, ViewMut, Workload,
    World,
};

#[derive(Debug, PartialEq, Eq)]
struct Changed(usize);
impl Component for Changed {
    type Tracking = track::All;
}
#[derive(Component, Debug)]
struct Plain(usize);
#[derive(Component, Debug)]
struct Other;

#[derive(Unique, Default)]
struct Observations(Vec<Vec<EntityId>>);

fn observe_changes(
    values: View<Changed>,
    plain: View<Plain>,
    mut seen: UniqueViewMut<Observations>,
) {
    seen.0.push(
        (values.modified(), &plain)
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect(),
    );
}

fn observe_changes_independently(
    values: View<Changed>,
    plain: View<Plain>,
    seen: UniqueViewMut<Observations>,
) {
    observe_changes(values, plain, seen);
}

#[test]
fn empty_guard_preserves_each_systems_last_run() -> Result<(), Box<dyn std::error::Error>> {
    let (world, ids) = setup(65);
    world.add_unique(Observations::default());
    Workload::new("first")
        .with_system(observe_changes)
        .add_to_world(&world)?;
    Workload::new("second")
        .with_system(observe_changes_independently)
        .add_to_world(&world)?;
    world.run_workload("first")?;
    world.run_workload("second")?;
    modify(&world, &[ids[64]]);
    world.run_workload("first")?;
    world.run_workload("first")?;
    world.run_workload("second")?;
    world.run(|seen: shipyard::UniqueView<Observations>| {
        assert_eq!(
            seen.0,
            vec![vec![], vec![], vec![ids[64]], vec![], vec![ids[64]]]
        );
    });
    Ok(())
}

#[test]
fn moved_tracking_after_deletion_remains_visible_to_plain_driver() {
    let (mut world, ids) = setup(129);
    modify(&world, &[ids[128]]);
    assert!(world.delete_entity(ids[3]));
    world.run(|values: View<Changed>, plain: View<Plain>| {
        let actual = (values.modified(), &plain)
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        assert_eq!(actual, vec![ids[128]]);
        let actual = (values.modified(), &plain)
            .par_iter()
            .with_id()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        assert_eq!(actual, vec![ids[128]]);
    });
}

#[test]
fn inserted_and_modified_join_keep_both_kinds_and_driver_order() {
    let (mut world, ids) = setup(65);
    let inserted = world.add_entity((Changed(65), Plain(65)));
    modify(&world, &[ids[3]]);
    world.run(|values: View<Changed>, plain: View<Plain>| {
        assert_eq!(
            (values.inserted(), &plain)
                .iter()
                .with_id()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec![inserted]
        );
        assert_eq!(
            (values.modified(), &plain)
                .iter()
                .with_id()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec![ids[3]]
        );
        let expected = (&plain)
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .filter(|id| *id == ids[3] || *id == inserted)
            .collect::<Vec<_>>();
        let actual = (values.inserted_or_modified(), &plain)
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    });
}

fn setup(len: usize) -> (World, Vec<EntityId>) {
    let mut world = World::new();
    let ids = (0..len)
        .map(|index| world.add_entity((Changed(index),)))
        .collect::<Vec<_>>();
    for (index, &id) in ids.iter().enumerate().rev() {
        world.add_component(id, (Plain(index), Other));
    }
    world.clear_all_inserted_and_modified();
    world.run(|_: ViewMut<Changed>, _: ViewMut<Plain>, _: ViewMut<Other>| {});
    (world, ids)
}

fn modify(world: &World, ids: &[EntityId]) {
    world.run(|mut values: ViewMut<Changed>| {
        for &id in ids {
            let Ok(mut value) = (&mut values).get(id) else {
                panic!("test entity must have Changed");
            };
            value.modify(|value| value.0 += 1000);
        }
    });
}

fn assert_empty<I: DoubleEndedIterator>(mut iter: I, len: usize) {
    assert_eq!(iter.size_hint(), (0, Some(0)), "empty len={len}");
    assert!(iter.next().is_none());
    assert!(iter.next_back().is_none());
}

#[test]
fn empty_tracking_join_has_zero_bound_even_for_partial_chunks() {
    for len in [0, 1, 63, 64, 65, 129] {
        let (world, _) = setup(len);
        world.run(|values: View<Changed>, plain: View<Plain>| {
            assert_empty((values.modified(), &plain).iter(), len);
            assert_empty((values.inserted(), &plain).iter(), len);
            assert_empty((values.inserted_or_modified(), &plain).iter(), len);
            assert_eq!((values.modified(), &plain).iter().with_id().count(), 0);
            assert_eq!((values.modified(), &plain).par_iter().with_id().count(), 0);
        });
        world.run(|mut values: ViewMut<Changed>, plain: View<Plain>| {
            assert_empty(((&mut values).modified(), &plain).iter(), len);
            assert_empty(((&mut values).inserted(), &plain).iter(), len);
            assert_empty(((&mut values).inserted_or_modified(), &plain).iter(), len);
        });
    }
}

#[test]
fn nonempty_join_preserves_raw_driver_order_and_first_consumed_target() {
    for changed_indices in [vec![64], vec![3, 17, 64], (0..65).collect()] {
        let (world, ids) = setup(65);
        let changed_ids = changed_indices
            .iter()
            .map(|&index| ids[index])
            .collect::<Vec<_>>();
        modify(&world, &changed_ids);
        world.run(|values: View<Changed>, plain: View<Plain>| {
            let actual = (values.modified(), &plain)
                .iter()
                .with_id()
                .map(|(id, (value, other))| {
                    assert_eq!(value.0, other.0 + 1000);
                    id
                })
                .collect::<Vec<_>>();
            let expected = changed_ids.iter().copied().rev().collect::<Vec<_>>();
            assert_eq!(actual, expected);
            // A one-use effect must still consume the same first target.
            assert_eq!(actual.first(), expected.first());
            let parallel = (values.modified(), &plain)
                .par_iter()
                .with_id()
                .map(|(id, _)| id)
                .collect::<Vec<_>>();
            assert_eq!(parallel.len(), expected.len());
            assert!(expected
                .iter()
                .all(|id| parallel.iter().filter(|other| *other == id).count() == 1));
        });
    }
}

#[test]
fn missing_join_member_and_mutable_tracking_keep_membership() {
    let (mut world, ids) = setup(65);
    world.remove::<(Plain,)>(ids[64]);
    modify(&world, &[ids[3], ids[64]]);
    world.run(|mut values: ViewMut<Changed>, plain: View<Plain>| {
        let actual = (&mut values)
            .modified()
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        assert_eq!(actual, vec![ids[3], ids[64]]);
        let joined = (values.modified(), &plain)
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        assert_eq!(joined, vec![ids[3]]);
    });
}

#[test]
fn empty_tracking_does_not_remove_or_not_or_optional_results() {
    let (world, ids) = setup(65);
    world.run(
        |values: View<Changed>, plain: View<Plain>, other: View<Other>| {
            let expected = ids.iter().copied().rev().collect::<Vec<_>>();
            let optional = (&plain, values.as_optional())
                .iter()
                .with_id()
                .map(|(id, _)| id)
                .collect::<Vec<_>>();
            assert_eq!(optional, expected);
            let not = (&plain, !values.modified())
                .iter()
                .with_id()
                .map(|(id, _)| id)
                .collect::<Vec<_>>();
            assert_eq!(not, expected);
            let or = ((&plain | values.inserted()), &other)
                .iter()
                .with_id()
                .map(|(id, _)| id)
                .collect::<Vec<_>>();
            assert_eq!(or, expected);
        },
    );
}

#[test]
fn empty_and_with_or_captain_discards_followup_slices() {
    let (world, _) = setup(65);
    world.run(
        |values: View<Changed>, plain: View<Plain>, other: View<Other>| {
            let iter = ((&plain | &other), values.modified()).iter();
            assert_eq!(iter.size_hint(), (0, Some(0)));
            assert_eq!(iter.with_id().count(), 0);
        },
    );
}

#[test]
fn mandatory_entity_slice_keeps_its_order() {
    let (world, ids) = setup(65);
    let order = [ids[3], ids[64], ids[17]];
    modify(&world, &order);
    world.run(|values: View<Changed>, plain: View<Plain>| {
        let actual = (order.as_slice(), values.modified(), &plain)
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        assert_eq!(actual, order);
    });
}

#[test]
fn chunk_budget_boundary_preserves_membership_and_driver_order() {
    // Plain64 allows eight metadata chunks; 513 tracked entities need nine.
    for len in [512, 513] {
        for nonempty in [false, true] {
            let mut world = World::new();
            let ids = world
                .bulk_add_entity((0..len).map(|index| (Changed(index),)))
                .collect::<Vec<_>>();
            let driver = ids[len - 64..].iter().copied().rev().collect::<Vec<_>>();
            for &id in &driver {
                world.add_component(id, (Plain(0),));
            }
            world.clear_all_inserted_and_modified();
            world.run(|_: ViewMut<Changed>| {});
            let changed = if nonempty {
                vec![driver[0], driver[63]]
            } else {
                Vec::new()
            };
            modify(&world, &changed);
            world.run(|values: View<Changed>, plain: View<Plain>| {
                let iter = (values.modified(), &plain).iter();
                if !nonempty {
                    assert_eq!(iter.size_hint().1, Some(if len == 512 { 0 } else { 64 }));
                }
                let actual = iter.with_id().map(|(id, _)| id).collect::<Vec<_>>();
                assert_eq!(actual, changed);
                let parallel = (values.modified(), &plain)
                    .par_iter()
                    .with_id()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>();
                assert_eq!(parallel.len(), changed.len());
                assert!(changed.iter().all(|id| parallel
                    .iter()
                    .filter(|other| *other == id)
                    .count()
                    == 1));
            });
        }
    }
}
