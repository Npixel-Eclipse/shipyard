use shipyard::{track, Component, EntityId, Get, IntoIter, ViewMut, World};

#[derive(PartialEq, Eq, Debug)]
struct Counter(usize);
impl Component for Counter {
    type Tracking = track::All;
}

const COUNT: usize = 300;

fn setup() -> (World, Vec<EntityId>) {
    let mut world = World::new();

    let ids = (0..COUNT).map(|i| world.add_entity(Counter(i))).collect();

    world.clear_all_inserted_and_modified();
    world.run(|_counters: ViewMut<Counter, track::All>| {});

    (world, ids)
}

#[test]
fn modified_across_chunk_boundaries() {
    let (world, ids) = setup();

    world.run(|mut counters: ViewMut<Counter, track::All>| {
        for &index in &[5usize, 70, 200, 299] {
            let mut counter = (&mut counters)
                .get(ids[index])
                .expect("entity should have a Counter");
            counter.modify(|counter| counter.0 += 1);
        }
    });

    world.run(|counters: ViewMut<Counter, track::All>| {
        let modified: Vec<EntityId> = counters
            .modified()
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect();

        assert_eq!(modified, vec![ids[5], ids[70], ids[200], ids[299]]);
    });
}

#[test]
fn modified_none_when_untouched() {
    let (world, _ids) = setup();

    world.run(|counters: ViewMut<Counter, track::All>| {
        assert_eq!(counters.modified().iter().count(), 0);
        assert_eq!(counters.inserted().iter().count(), 0);
    });
}

#[test]
fn swap_remove_keeps_moved_modification_visible() {
    let (mut world, ids) = setup();

    world.run(|mut counters: ViewMut<Counter, track::All>| {
        let mut counter = (&mut counters)
            .get(ids[COUNT - 1])
            .expect("entity should have a Counter");
        counter.modify(|counter| counter.0 += 1);
    });

    world.delete_entity(ids[3]);

    world.run(|counters: ViewMut<Counter, track::All>| {
        let modified: Vec<EntityId> = counters
            .modified()
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect();

        assert_eq!(modified, vec![ids[COUNT - 1]]);
    });
}

#[test]
fn inserted_across_chunk_boundaries() {
    let (mut world, _ids) = setup();

    let new_id = world.add_entity(Counter(1000));

    world.run(|counters: ViewMut<Counter, track::All>| {
        let inserted: Vec<EntityId> = counters
            .inserted()
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect();

        assert_eq!(inserted, vec![new_id]);
    });
}

#[test]
fn bulk_insert_updates_existing_insertion_chunk() {
    let (mut world, _ids) = setup();

    let new_ids = world
        .bulk_add_entity((0..1).map(|_| Counter(1000)))
        .collect::<Vec<_>>();

    world.run(|counters: ViewMut<Counter, track::All>| {
        let inserted: Vec<EntityId> = counters
            .inserted()
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect();

        assert_eq!(inserted, new_ids);
    });
}

#[cfg(feature = "parallel")]
#[test]
fn par_iter_covers_all_entities() {
    use rayon::prelude::*;

    let (world, _ids) = setup();

    world.run(|counters: ViewMut<Counter, track::All>| {
        let total: usize = (&counters).par_iter().map(|_| 1).sum();

        assert_eq!(total, COUNT);

        let modified_total: usize = counters.modified().par_iter().map(|_| 1).sum();

        assert_eq!(modified_total, 0);
    });
}
