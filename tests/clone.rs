use shipyard::{
    sparse_set::SparseSet, track, AllStoragesView, AllStoragesViewMut, Component, EntityId, Get,
    IntoIter, View, ViewMut, World,
};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
struct USIZE(usize);
impl Component for USIZE {
    type Tracking = track::Untracked;
}

#[test]
fn clone_world() {
    let mut world = World::new();

    let eid = world.add_entity(USIZE(1));

    let world2 = world.clone();

    assert!(world2.borrow::<View<USIZE>>().unwrap().is_empty());

    world.register_clone::<SparseSet<USIZE>>();

    let world3 = world.clone();

    world3.run(|usizes: View<USIZE>| {
        assert_eq!(usizes.len(), 1);

        assert_eq!(usizes[eid], USIZE(1));
    });
}

#[test]
fn clone_entity() {
    let mut world = World::new();

    let eid = world.add_entity(USIZE(1));
    world.add_entity(USIZE(2));

    let mut world2 = world.clone();

    assert!(world2.borrow::<View<USIZE>>().unwrap().is_empty());

    world.register_clone::<SparseSet<USIZE>>();

    world.clone_entity_to(&mut world2, eid);

    world2.run(|usizes: View<USIZE>| {
        assert_eq!(usizes.len(), 1);

        assert_eq!(usizes[eid], USIZE(1));
    });
}

#[derive(Clone)]
struct Changed(usize);
impl Component for Changed {
    type Tracking = track::All;
}

#[derive(Clone)]
struct Plain;
impl Component for Plain {
    type Tracking = track::Untracked;
}

const CLONE_COUNTS: [usize; 6] = [128, 63, 64, 65, 127, 129];

fn tracked_world(count: usize) -> (World, Vec<EntityId>) {
    let mut world = World::new();
    let ids = (0..count)
        .map(|i| world.add_entity((Changed(i), Plain)))
        .collect();
    world.register_clone::<(SparseSet<Changed>, SparseSet<Plain>)>();
    (world, ids)
}

fn assert_tracking(
    changed: &View<Changed>,
    plain: &View<Plain>,
    inserted: &[EntityId],
    modified: &[EntityId],
) {
    assert_eq!(
        (changed.modified(), plain)
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        modified,
        "joined modified"
    );
    assert_eq!(
        (changed.inserted(), plain)
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        inserted,
        "joined inserted"
    );
    assert_eq!(
        changed
            .inserted()
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        inserted,
        "single-storage inserted"
    );
    assert_eq!(
        changed
            .modified()
            .iter()
            .with_id()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        modified,
        "single-storage modified"
    );
    #[cfg(feature = "parallel")]
    {
        let mut actual = (changed.inserted(), plain)
            .par_iter()
            .with_id()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        actual.sort_unstable();
        assert_eq!(actual, inserted, "parallel joined inserted");
        let mut actual = (changed.modified(), plain)
            .par_iter()
            .with_id()
            .map(|(id, _)| id)
            .collect::<Vec<_>>();
        actual.sort_unstable();
        assert_eq!(actual, modified, "parallel joined modified");
    }
}

#[test]
fn clone_keeps_inserted_join_after_bulk_add_and_delete() -> Result<(), Box<dyn std::error::Error>> {
    for count in CLONE_COUNTS {
        let (world, ids) = tracked_world(count);
        let mut cloned = world.clone();
        let added = cloned
            .bulk_add_entity([(Changed(count), Plain)])
            .next()
            .ok_or("missing bulk entity")?;
        assert!(cloned.delete_entity(added));
        cloned.run(|changed: View<Changed>, plain: View<Plain>| {
            assert_tracking(&changed, &plain, &ids, &[]);
        });
    }
    Ok(())
}

#[test]
fn clone_keeps_modified_join_after_bulk_add() -> Result<(), Box<dyn std::error::Error>> {
    for count in CLONE_COUNTS {
        let (world, ids) = tracked_world(count);
        let mut cloned = world.clone();
        {
            let mut changed = cloned.borrow::<ViewMut<Changed>>()?;
            (&mut changed).get(ids[0])?.modify(|value| value.0 += 1);
        }
        let added = cloned
            .bulk_add_entity([(Changed(count), Plain)])
            .next()
            .ok_or("missing bulk entity")?;
        let mut inserted = ids.clone();
        inserted.push(added);
        cloned.run(|changed: View<Changed>, plain: View<Plain>| {
            assert_tracking(&changed, &plain, &inserted, &ids[..1]);
        });
    }
    Ok(())
}

#[test]
fn clone_storages_uses_destination_tracking_timestamp() -> Result<(), Box<dyn std::error::Error>> {
    let (world, ids) = tracked_world(128);
    {
        let mut changed = world.borrow::<ViewMut<Changed>>()?;
        (&mut changed).get(ids[0])?.modify(|value| value.0 += 1);
    }

    let destination = World::new();
    for _ in 0..ids.len() + 10 {
        destination.run(|_: View<Plain>| {});
    }
    let before_clone = destination.get_tracking_timestamp();
    assert!(world.get_tracking_timestamp().is_older_than(before_clone));
    world
        .borrow::<AllStoragesView>()?
        .clone_storages_to(&mut *destination.borrow::<AllStoragesViewMut>()?);
    destination.run(|mut changed: View<Changed>, plain: View<Plain>| {
        // Source insertion timestamps precede this destination's observation window.
        changed.override_last_insertion(before_clone);
        assert_tracking(&changed, &plain, &ids, &[]);
        assert_eq!(changed[ids[0]].0, 1);
    });
    Ok(())
}

#[cfg(feature = "thread_local")]
mod thread_local {
    use super::*;
    use shipyard::borrow::{NonSend, NonSendSync, NonSync};
    use shipyard::EntitiesViewMut;

    macro_rules! clone_tracking {
        ($test:ident, $wrapper:ident) => {
            #[test]
            fn $test() -> Result<(), Box<dyn std::error::Error>> {
                let mut world = World::new();
                let mut ids = {
                    let (mut entities, mut changed, mut plain) = world.borrow::<(
                        EntitiesViewMut,
                        $wrapper<ViewMut<Changed>>,
                        ViewMut<Plain>,
                    )>()?;
                    let ids = (0..128)
                        .map(|i| {
                            entities.add_entity((&mut *changed, &mut plain), (Changed(i), Plain))
                        })
                        .collect::<Vec<_>>();
                    (&mut *changed).get(ids[0])?.modify(|value| value.0 += 1);
                    ids
                };
                world.register_clone::<($wrapper<SparseSet<Changed>>, SparseSet<Plain>)>();
                let cloned = world.clone();
                {
                    let (changed, plain) =
                        cloned.borrow::<($wrapper<View<Changed>>, View<Plain>)>()?;
                    assert_tracking(&changed, &plain, &ids, &[]);
                }
                {
                    let (mut entities, mut changed, mut plain) = cloned.borrow::<(
                        EntitiesViewMut,
                        $wrapper<ViewMut<Changed>>,
                        ViewMut<Plain>,
                    )>()?;
                    (&mut *changed).get(ids[0])?.modify(|value| value.0 += 1);
                    ids.push(
                        entities.add_entity((&mut *changed, &mut plain), (Changed(128), Plain)),
                    );
                }
                let (changed, plain) = cloned.borrow::<($wrapper<View<Changed>>, View<Plain>)>()?;
                assert_tracking(&changed, &plain, &ids, &ids[..1]);
                Ok(())
            }
        };
    }

    clone_tracking!(clone_non_send_tracking, NonSend);
    clone_tracking!(clone_non_sync_tracking, NonSync);
    clone_tracking!(clone_non_send_sync_tracking, NonSendSync);
}
