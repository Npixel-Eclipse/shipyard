use shipyard::{
    iter::OneOfTwo, track, Component, EntityId, Get, IntoIter, SafeMut, View, ViewMut, World,
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

struct A;
impl Component for A {
    type Tracking = track::All;
}

struct B(usize);
impl Component for B {
    type Tracking = track::All;
}

struct C(usize);
impl Component for C {
    type Tracking = track::Untracked;
}

struct Marker;
impl Component for Marker {
    type Tracking = track::Untracked;
}

fn record(
    (id, value): (EntityId, OneOfTwo<OneOfTwo<&A, SafeMut<'_, B>>, &C>),
) -> (EntityId, char, usize) {
    match value {
        OneOfTwo::One(OneOfTwo::Two(mut b)) => {
            let original = b.0;
            b.modify(|value| value.0 += 1);
            (id, 'B', original)
        }
        OneOfTwo::Two(c) => (id, 'C', c.0),
        _ => panic!("A is empty"),
    }
}

fn fixture() -> TestResult<(World, [EntityId; 4])> {
    let mut world = World::new();
    let ids = [
        world.add_entity((B(10), C(100))),
        world.add_entity((B(20), C(200))),
        world.add_entity((C(300),)),
        world.add_entity((B(40), C(400))),
    ];
    // An ID that fails the left predicate from the outset must still yield C.
    (&mut world.borrow::<ViewMut<B>>()?)
        .get(ids[3])?
        .modify(|value| value.0 += 1);
    Ok((world, ids))
}

#[test]
fn nested_or_yields_each_entity_once_after_mutation() -> TestResult {
    let mut world = World::new();
    let shared = world.add_entity((B(10), C(100)));
    let (a, mut b, c) = world.borrow::<(View<A>, ViewMut<B>, View<C>)>()?;
    let mut iter = ((a.inserted() | !b.modified_mut()) | &c).iter().with_id();
    assert_eq!(
        record(iter.next().ok_or("missing B component")?),
        (shared, 'B', 10)
    );
    let remaining = iter.fold(Vec::new(), |mut items, item| {
        items.push(record(item));
        items
    });
    assert_eq!(remaining, Vec::new());
    assert_eq!(b.get(shared)?.0, 11);
    Ok(())
}

#[test]
fn nested_or_preserves_variants_and_values_across_consumers() -> TestResult {
    for mode in ["fold", "for_each", "reverse", "mixed"] {
        let (world, ids) = fixture()?;
        let (a, mut b, c) = world.borrow::<(View<A>, ViewMut<B>, View<C>)>()?;
        let mut iter = ((a.inserted() | !b.modified_mut()) | &c).iter().with_id();
        let mut actual = Vec::new();
        match mode {
            "fold" => {
                actual = iter.fold(actual, |mut items, item| {
                    items.push(record(item));
                    items
                });
            }
            "for_each" => iter.map(record).for_each(|item| actual.push(item)),
            "reverse" => {
                while let Some(item) = iter.next_back() {
                    actual.push(record(item));
                }
            }
            "mixed" => {
                actual.push(record(iter.next().ok_or("missing first B")?));
                assert_eq!(actual[0], (ids[0], 'B', 10));
                // The back visits C before returning the remaining B values.
                while let Some(item) = iter.next_back() {
                    actual.push(record(item));
                }
                assert!(iter.next().is_none());
            }
            _ => unreachable!(),
        }
        actual.sort_unstable();
        assert_eq!(
            actual,
            vec![
                (ids[0], 'B', 10),
                (ids[1], 'B', 20),
                (ids[2], 'C', 300),
                (ids[3], 'C', 400),
            ],
            "{mode}"
        );
        assert_eq!(b.get(ids[0])?.0, 11);
        assert_eq!(b.get(ids[1])?.0, 21);
        assert_eq!(b.get(ids[3])?.0, 41);
    }
    Ok(())
}

#[test]
fn nested_or_does_not_record_ids_rejected_by_the_left_join() -> TestResult {
    let mut world = World::new();
    let accepted = world.add_entity((B(10), C(100), Marker));
    let rejected = world.add_entity((B(20), C(200)));
    for _ in 0..3 {
        world.add_entity((Marker,));
    }
    let (a, mut b, c, marker) = world.borrow::<(View<A>, ViewMut<B>, View<C>, View<Marker>)>()?;
    // B is smaller than Marker, so its rejected candidate is actually probed.
    let actual = ((a.inserted() | (!b.modified_mut(), &marker)) | &c)
        .iter()
        .with_id()
        .fold(Vec::new(), |mut items, (id, value)| {
            let item = match value {
                OneOfTwo::One(OneOfTwo::Two((b, _))) => {
                    record((id, OneOfTwo::One(OneOfTwo::Two(b))))
                }
                OneOfTwo::Two(c) => record((id, OneOfTwo::Two(c))),
                _ => panic!("A is empty"),
            };
            items.push(item);
            items
        });
    assert_eq!(actual, vec![(accepted, 'B', 10), (rejected, 'C', 200)]);
    assert_eq!(b.get(rejected)?.0, 20);
    Ok(())
}

#[cfg(feature = "parallel")]
#[test]
fn nested_mutable_or_uses_one_producer_and_yields_each_id_once() -> TestResult {
    use rayon::{iter::plumbing::UnindexedProducer, prelude::*};

    let (world, ids) = fixture()?;
    let (a, mut b, c) = world.borrow::<(View<A>, ViewMut<B>, View<C>)>()?;
    let (producer, right) = ((a.inserted() | !b.modified_mut()) | &c).iter().split();
    assert!(right.is_none());
    drop(producer);
    let mut actual: Vec<_> = ((a.inserted() | !b.modified_mut()) | &c)
        .par_iter()
        .with_id()
        .map(record)
        .collect();
    actual.sort_unstable();
    assert_eq!(
        actual,
        vec![
            (ids[0], 'B', 10),
            (ids[1], 'B', 20),
            (ids[2], 'C', 300),
            (ids[3], 'C', 400),
        ]
    );
    assert_eq!(b.get(ids[0])?.0, 11);
    assert_eq!(b.get(ids[1])?.0, 21);
    Ok(())
}
