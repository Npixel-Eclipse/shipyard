use shipyard::{iter::OneOfTwo, track, Component, EntityId, IntoIter, View, World};

struct A(usize);
impl Component for A {
    type Tracking = track::Untracked;
}

struct B(usize);
impl Component for B {
    type Tracking = track::Untracked;
}

fn owned((id, value): (EntityId, OneOfTwo<&A, &B>)) -> (EntityId, OneOfTwo<usize, usize>) {
    (
        id,
        match value {
            OneOfTwo::One(value) => OneOfTwo::One(value.0),
            OneOfTwo::Two(value) => OneOfTwo::Two(value.0),
        },
    )
}

#[test]
fn with_id_fold_visits_both_or_storages() -> Result<(), Box<dyn std::error::Error>> {
    let mut world = World::new();
    let only_a = world.add_entity(A(10));
    let only_b = world.add_entity(B(20));
    let (a, b) = world.borrow::<(View<A>, View<B>)>()?;

    let mut direct = (&a | &b).iter().with_id();
    assert_eq!(direct.next().map(|(id, _)| id), Some(only_a));
    assert_eq!(direct.next().map(|(id, _)| id), Some(only_b));
    assert!(direct.next().is_none());

    let folded = (&a | &b)
        .iter()
        .with_id()
        .fold(Vec::new(), |mut seen, (id, value)| {
            let value = match value {
                OneOfTwo::One(value) => value.0,
                OneOfTwo::Two(value) => value.0,
            };
            seen.push((id, value));
            seen
        });
    assert_eq!(folded, vec![(only_a, 10), (only_b, 20)]);
    Ok(())
}

#[test]
fn fold_consumers_handle_overlaps_empty_sides_and_partial_iteration(
) -> Result<(), Box<dyn std::error::Error>> {
    for (has_a, has_b, overlap) in [
        (true, true, true),
        (true, false, false),
        (false, true, false),
        (false, false, false),
    ] {
        let mut world = World::new();
        let mut expected = Vec::new();
        if has_a {
            expected.push((world.add_entity(A(10)), OneOfTwo::One(10)));
        }
        if overlap {
            expected.push((world.add_entity((A(11), B(111))), OneOfTwo::One(11)));
        }
        if has_b {
            expected.push((world.add_entity(B(20)), OneOfTwo::Two(20)));
        }
        let (a, b) = world.borrow::<(View<A>, View<B>)>()?;

        for prefix in 0..=expected.len() {
            let mut iter = (&a | &b).iter().with_id();
            for item in &expected[..prefix] {
                assert_eq!(iter.next().map(owned).as_ref(), Some(item));
            }
            let folded = iter.fold(Vec::new(), |mut seen, item| {
                seen.push(owned(item));
                seen
            });
            assert_eq!(folded, expected[prefix..]);
        }

        let ids = (&a | &b).iter().ids().fold(Vec::new(), |mut seen, id| {
            seen.push(id);
            seen
        });
        assert_eq!(ids, expected.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        let mut mapped = Vec::new();
        (&a | &b)
            .iter()
            .with_id()
            .map(owned)
            .for_each(|item| mapped.push(item));
        assert_eq!(mapped, expected);
    }
    Ok(())
}
