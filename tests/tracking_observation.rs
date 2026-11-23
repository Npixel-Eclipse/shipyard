#![cfg(feature = "scheduler-experiment")]

use rayon::prelude::*;
use shipyard::track::{All, Untracked};
use shipyard::tracking_observation::{TrackingHit, set_observer};
use shipyard::{Component, Get, IntoIter, View, ViewMut, Workload, World};
use std::any::type_name;
use std::collections::BTreeSet;
use std::error::Error;
use std::sync::Mutex;

struct Value(usize);
impl Component for Value {
    type Tracking = All;
}

struct Tag;
impl Component for Tag {
    type Tracking = Untracked;
}

static HITS: Mutex<Vec<TrackingHit>> = Mutex::new(Vec::new());

fn record(hit: TrackingHit) {
    HITS.lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .push(hit);
}

fn observe(values: View<Value>, tags: View<Tag>) {
    let actual: Vec<_> = (values.modified(), &tags)
        .iter()
        .map(|(value, _)| value.0)
        .collect();
    assert_eq!(actual, [2, 4096]);
    // Before the separate parallel/direct queries add hits, the rejected join candidate
    // must already have a positive predicate observation.
    assert_eq!(
        HITS.lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .filter(|hit| hit.kind == "modified")
            .count(),
        3
    );
    // Inner Rayon tasks retain the workload view's timestamp and system identity.
    values
        .modified()
        .par_iter()
        .with_split_threshold(1)
        .for_each(|_| {});
    for (id, _) in values.modified().iter().with_id() {
        assert!(values.is_modified(id));
        assert!(values.is_inserted_or_modified(id));
    }
}

#[test]
fn positive_predicates_keep_candidate_ids_and_inner_rayon_system_identity()
-> Result<(), Box<dyn Error>> {
    set_observer(record)?;
    let mut world = World::new();
    let ids: Vec<_> = (0..4096)
        .map(|i| {
            let id = world.add_entity((Value(i),));
            if i != 1024 {
                world.add_component(id, (Tag,));
            }
            id
        })
        .collect();
    // Make the sparse tracking filter cheaper than the other join source.
    for _ in 0..4096 {
        world.add_entity((Tag,));
    }
    world.clear_all_inserted_and_modified();
    world.run(|_: ViewMut<Value>| {});
    {
        let mut values = world.borrow::<ViewMut<Value>>()?;
        for index in [1, 1024, 4095] {
            (&mut values).get(ids[index])?.modify(|value| value.0 += 1);
        }
        // Non-workload borrows have no system association and must not be attributed.
        assert_eq!(values.modified().iter().count(), 3);
        assert!(values.is_modified(ids[1]));
    }
    assert!(
        HITS.lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .is_empty()
    );
    Workload::new("tracking-observer")
        .with_system(observe)
        .add_to_world(&world)?;
    world.run_workload("tracking-observer")?;

    let hits = HITS.lock().unwrap_or_else(|poison| poison.into_inner());
    let expected: BTreeSet<_> = [1, 1024, 4095]
        .map(|i| ids[i].inner())
        .into_iter()
        .collect();
    let modified: BTreeSet<_> = hits
        .iter()
        .filter(|hit| hit.kind == "modified")
        .map(|hit| hit.entity)
        .collect();
    assert_eq!(modified, expected);
    assert!(
        hits.iter()
            .all(|hit| hit.component == type_name::<Value>() && hit.system.ends_with("::observe"))
    );
    assert_eq!(
        hits.iter()
            .map(|hit| hit.current)
            .collect::<BTreeSet<_>>()
            .len(),
        1
    );
    assert_eq!(
        hits.iter()
            .filter(|hit| hit.kind == "inserted_or_modified")
            .map(|hit| hit.entity)
            .collect::<BTreeSet<_>>(),
        expected
    );
    // A candidate hit rejected by Tag is still represented; it is not a loop-body count.
    assert!(
        hits.iter()
            .any(|hit| hit.kind == "modified" && hit.entity == ids[1024].inner())
    );
    Ok(())
}
