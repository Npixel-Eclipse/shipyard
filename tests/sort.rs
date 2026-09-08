use shipyard::{track, Component, EntityId, Get, IntoIter, View, ViewMut, World};

#[derive(Debug, PartialEq, Eq)]
struct Value(usize);
impl Component for Value {
    type Tracking = track::All;
}

struct Plain(usize);
impl Component for Plain {
    type Tracking = track::Untracked;
}

#[test]
fn sort_preserves_each_entity_tracking() -> Result<(), Box<dyn std::error::Error>> {
    let mut world = World::new();
    let modified = world.add_entity(Value(2));
    world.clear_all_inserted_and_modified();
    world.run(|_: ViewMut<Value>| {});
    let inserted = world.add_entity(Value(1));

    let mut values = world.borrow::<ViewMut<Value>>()?;
    (&mut values).get(modified)?.modify(|value| value.0 += 10);
    assert!(values.is_modified(modified));
    assert!(!values.is_inserted(modified));
    assert!(values.is_inserted(inserted));
    assert!(!values.is_modified(inserted));
    values.sort_unstable_by(|left, right| left.0.cmp(&right.0));

    assert_eq!(values[modified], Value(12));
    assert_eq!(values[inserted], Value(1));
    assert!(values.is_modified(modified));
    assert!(!values.is_inserted(modified));
    assert!(values.is_inserted(inserted));
    assert!(!values.is_modified(inserted));
    assert_eq!(
        values.modified().iter().ids().collect::<Vec<_>>(),
        vec![modified]
    );
    assert_eq!(
        values.inserted().iter().ids().collect::<Vec<_>>(),
        vec![inserted]
    );
    Ok(())
}

fn assert_tracking_membership(
    values: &View<Value>,
    plain: &View<Plain>,
    inserted: &[EntityId],
    modified: &[EntityId],
) {
    macro_rules! assert_ids {
        ($source:expr, $expected:expr) => {{
            let mut expected = $expected.to_vec();
            expected.sort_unstable();
            let mut actual = ($source).iter().ids().collect::<Vec<_>>();
            actual.sort_unstable();
            assert_eq!(actual, expected);
            #[cfg(feature = "parallel")]
            {
                use rayon::prelude::*;
                let mut actual = ($source)
                    .par_iter()
                    .with_id()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>();
                actual.sort_unstable();
                assert_eq!(actual, expected);
            }
        }};
    }

    let mut changed = inserted.to_vec();
    changed.extend_from_slice(modified);
    changed.sort_unstable();
    changed.dedup();
    assert_ids!(values.inserted(), inserted);
    assert_ids!(values.modified(), modified);
    assert_ids!(values.inserted_or_modified(), changed);
    assert_ids!((values.inserted(), plain), inserted);
    assert_ids!((values.modified(), plain), modified);
    assert_ids!((values.inserted_or_modified(), plain), changed);
}

#[test]
fn sort_moves_tracking_across_chunks_without_changing_timestamps(
) -> Result<(), Box<dyn std::error::Error>> {
    for count in [65, 129] {
        let mut world = World::new();
        let ids = (1..count)
            .map(|i| world.add_entity((Value(i), Plain(i))))
            .collect::<Vec<_>>();
        world.clear_all_inserted_and_modified();
        let before_changes = world.get_tracking_timestamp();
        world.run(|_: View<Value>| {});
        let inserted = world.add_entity((Value(0), Plain(0)));
        {
            let mut values = world.borrow::<ViewMut<Value>>()?;
            (&mut values).get(ids[0])?.modify(|value| value.0 = count);
        }
        let after_changes = world.get_tracking_timestamp();
        world.run(|_: View<Value>| {});
        {
            let mut values = world.borrow::<ViewMut<Value>>()?;
            values.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        }
        let (mut values, plain) = world.borrow::<(View<Value>, View<Plain>)>()?;
        assert_eq!(values[ids[0]], Value(count));
        assert_eq!(values[inserted], Value(0));
        assert_eq!(
            values.iter().map(|value| value.0).collect::<Vec<_>>(),
            core::iter::once(0).chain(2..=count).collect::<Vec<_>>()
        );
        assert_tracking_membership(&values, &plain, &[inserted], &ids[..1]);
        values.override_last_insertion(before_changes);
        values.override_last_modification(before_changes);
        assert_tracking_membership(&values, &plain, &[inserted], &ids[..1]);
        values.override_last_insertion(after_changes);
        values.override_last_modification(after_changes);
        assert_tracking_membership(&values, &plain, &[], &[]);
    }
    Ok(())
}

#[test]
fn sort_supports_tracking_modes_and_small_storages() -> Result<(), Box<dyn std::error::Error>> {
    macro_rules! check_mode {
        ($tracking:ty) => {{
            struct Mode(usize);
            impl Component for Mode {
                type Tracking = $tracking;
            }
            for count in [0, 1, 3] {
                let mut world = World::new();
                let ids = (0..count)
                    .map(|i| world.add_entity(Mode(count - i)))
                    .collect::<Vec<_>>();
                let mut values = world.borrow::<ViewMut<Mode>>()?;
                values.sort_unstable_by(|left, right| left.0.cmp(&right.0));
                assert_eq!(
                    values.iter().ids().collect::<Vec<_>>(),
                    ids.iter().rev().copied().collect::<Vec<_>>()
                );
                for (i, &id) in ids.iter().enumerate() {
                    assert_eq!(values[id].0, count - i);
                }
            }
        }};
    }
    check_mode!(track::Insertion);
    check_mode!(track::Modification);
    check_mode!(track::Untracked);
    Ok(())
}

#[test]
fn sort_preserves_runtime_enabled_tracking() -> Result<(), Box<dyn std::error::Error>> {
    let mut world = World::new();
    let modified = world.add_entity(Plain(2));
    world.track_all::<Plain>();
    let inserted = world.add_entity(Plain(1));
    let mut values = world.borrow::<ViewMut<Plain, track::All>>()?;
    (&mut values).get(modified)?.modify(|value| value.0 += 10);
    values.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(values[modified].0, 12);
    assert_eq!(values[inserted].0, 1);
    assert_eq!(
        values.modified().iter().ids().collect::<Vec<_>>(),
        vec![modified]
    );
    assert_eq!(
        values.inserted().iter().ids().collect::<Vec<_>>(),
        vec![inserted]
    );
    Ok(())
}

#[test]
fn sort_comparator_panic_preserves_storage_and_tracking() -> Result<(), Box<dyn std::error::Error>>
{
    let mut world = World::new();
    let modified = world.add_entity(Value(2));
    world.add_entity(Value(1));
    let mut values = world.borrow::<ViewMut<Value>>()?;
    (&mut values).get(modified)?.modify(|value| value.0 += 10);
    let snapshot = |values: &ViewMut<Value>| {
        let components = values
            .iter()
            .with_id()
            .map(|(id, value)| (id, value.0, values.is_inserted(id), values.is_modified(id)))
            .collect::<Vec<_>>();
        (
            components,
            values.inserted().iter().ids().collect::<Vec<_>>(),
            values.modified().iter().ids().collect::<Vec<_>>(),
        )
    };
    let before = snapshot(&values);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        values.sort_unstable_by(|_, _| panic!("comparison failed"));
    }));
    assert!(result.is_err());
    assert_eq!(snapshot(&values), before);
    assert_eq!(values[modified], Value(12));
    Ok(())
}
