use shipyard::{Component, EntityId, Get, IntoIter, SafeMut, View, ViewMut, World};

#[derive(Component)]
#[track(All)]
struct Position(u64);

#[derive(Component)]
struct Velocity(u64);

#[test]
fn explicit_writes_keep_tracking_contract() {
    let mut world = World::new();
    let id = world.add_entity(Position(1));
    world
        .borrow::<ViewMut<Position>>()
        .unwrap()
        .clear_all_modified();
    {
        let mut value = world.get::<&mut Position>(id).unwrap();
        assert_eq!(value.0, 1);
        value.modify_without_tracking(|p| p.0 += 1);
    }
    assert_eq!(
        world
            .borrow::<View<Position>>()
            .unwrap()
            .modified()
            .iter()
            .count(),
        0
    );
    world.get::<&mut Position>(id).unwrap().modify(|p| p.0 += 1);
    assert_eq!(
        world
            .borrow::<View<Position>>()
            .unwrap()
            .modified()
            .iter()
            .count(),
        1
    );
    world
        .borrow::<ViewMut<Position>>()
        .unwrap()
        .clear_all_modified();
    {
        let mut positions = world.borrow::<ViewMut<Position>>().unwrap();
        let mut value = SafeMut::map((&mut positions).get(id).unwrap(), |p| &mut p.0);
        value.modify(|p| *p += 1);
    }
    assert_eq!(
        world
            .borrow::<View<Position>>()
            .unwrap()
            .modified()
            .iter()
            .count(),
        1
    );
}

#[test]
fn retained_and_mixed_items_keep_tracking() {
    use rayon::prelude::*;
    let mut world = World::new();
    world.bulk_add_entity((0..257).map(|i| (Position(i), Velocity(1))));
    world
        .borrow::<ViewMut<Position>>()
        .unwrap()
        .clear_all_modified();
    {
        let (mut positions, velocities) = world
            .borrow::<(ViewMut<Position>, View<Velocity>)>()
            .unwrap();
        let items: Vec<_> = (&mut positions).iter().take(65).collect();
        items
            .into_par_iter()
            .for_each(|mut p| p.modify(|p| p.0 += 1));
        assert_eq!(positions.modified().iter().count(), 65);
        positions.clear_all_modified();
        let mut positions = world.borrow::<ViewMut<Position>>().unwrap();
        (&velocities, &mut positions)
            .iter()
            .for_each(|(v, mut p)| p.modify(|p| p.0 += v.0));
        assert_eq!(positions.modified().iter().count(), 257);
    }
}

fn delete_batch(world: &mut World, ids: impl IntoIterator<Item = EntityId>) -> usize {
    #[cfg(all(feature = "parallel", not(feature = "thread_local")))]
    {
        world.par_bulk_delete_entity(ids)
    }
    #[cfg(any(not(feature = "parallel"), feature = "thread_local"))]
    {
        world.bulk_delete_entity(ids)
    }
}

#[test]
fn entity_ids_and_bulk_delete_match_battle() {
    let mut world = World::new();
    let id = world.add_entity(Position(1));
    assert_eq!(id.to_string().parse::<EntityId>().unwrap(), id);
    assert_eq!(world.bulk_delete_entity(id), 1);
    assert_eq!(world.bulk_delete_entity(id), 0);
    let ids: Vec<_> = world
        .bulk_add_entity((0..65).map(|i| (Position(i),)))
        .collect();
    assert_eq!(delete_batch(&mut world, ids), 65);
    assert!(world.borrow::<View<Position>>().unwrap().is_empty());
}

#[test]
fn grouped_bulk_delete_preserves_join_and_modifications() {
    let mut world = World::builder()
        .with_group::<(Position, Velocity)>()
        .build();
    let ids: Vec<_> = world
        .bulk_add_entity((0..130).map(|i| (Position(i), Velocity(1))))
        .collect();
    world.regroup();
    world
        .borrow::<ViewMut<Position>>()
        .unwrap()
        .clear_all_modified();
    {
        let (mut positions, velocities) = world
            .borrow::<(ViewMut<Position>, View<Velocity>)>()
            .unwrap();
        (&mut positions, &velocities)
            .iter()
            .for_each(|(mut p, v)| p.modify(|p| p.0 += v.0));
    }
    assert_eq!(delete_batch(&mut world, ids.iter().step_by(2).copied()), 65);
    world.regroup();
    let (positions, velocities) = world.borrow::<(View<Position>, View<Velocity>)>().unwrap();
    assert_eq!((&positions, &velocities).iter().count(), 65);
    assert_eq!(positions.modified().iter().count(), 65);
}
