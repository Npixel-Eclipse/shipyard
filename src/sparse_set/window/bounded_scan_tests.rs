use crate::iter::{IntoShiperator, Shiperator, ShiperatorCaptain};
use crate::{track, Component, EntityId, Get, View, ViewMut, World};
use alloc::{vec, vec::Vec};
use core::{cell::RefCell, ops::Range};

std::thread_local! {
    static CHUNK_VISITS: RefCell<Option<Vec<usize>>> = const { RefCell::new(None) };
}

pub(super) fn record_chunk_visit(chunk: usize) {
    CHUNK_VISITS.with(|visits| {
        if let Some(visits) = visits.borrow_mut().as_mut() {
            visits.push(chunk);
        }
    });
}

fn scan_chunks<R>(scan: impl FnOnce() -> R) -> (R, Vec<usize>) {
    CHUNK_VISITS.with(|visits| *visits.borrow_mut() = Some(Vec::new()));
    let result = scan();
    let visits = CHUNK_VISITS.with(|visits| {
        visits
            .borrow_mut()
            .take()
            .expect("scan observation was enabled")
    });
    (result, visits)
}

struct Value(usize);
impl Component for Value {
    type Tracking = track::All;
}

struct Other;
impl Component for Other {
    type Tracking = track::Untracked;
}

fn setup() -> (World, Vec<EntityId>) {
    let mut world = World::new();
    let ids = (0..4097)
        .map(|i| world.add_entity(Value(i)))
        .collect::<Vec<_>>();
    world.clear_all_inserted_and_modified();
    world.run(|_: ViewMut<'_, Value>| {});
    world.run(|mut values: ViewMut<'_, Value>| {
        for index in [70, 4096] {
            (&mut values)
                .get(ids[index])
                .expect("test entity exists")
                .modify(|_| {});
        }
    });
    (world, ids)
}

// The zero planning budget exercises the Dense fallback even for sparse data.
// These are the same dense-index subranges carried by split producers.
fn producer<I: IntoShiperator>(input: I, range: Range<usize>) -> Shiperator<I::Shiperator>
where
    I::Shiperator: ShiperatorCaptain,
{
    let (shiperator, len, mut entities) =
        input.into_shiperator_with_budget(&mut Default::default(), 0);
    assert!(range.start <= range.end && range.end <= len);
    entities.follow_up_ptrs.clear();
    Shiperator {
        is_exact_sized: shiperator.is_exact_sized(),
        shiperator,
        entities,
        start: range.start,
        end: range.end,
        min_split_len: 1,
    }
}

#[test]
fn bounded_dense_next_does_not_scan_past_clean_gap_or_tail() {
    let (world, _) = setup();
    world.run(|values: View<'_, Value>| {
        for range in [0..64, 128..256, 128..130, 4097..4097] {
            let mut iter = producer(values.modified(), range.clone());
            let (item, visits) = scan_chunks(|| iter.next());
            assert!(item.is_none());
            let expected = if range.is_empty() {
                Vec::new()
            } else {
                (range.start / 64..range.end.div_ceil(64)).collect::<Vec<_>>()
            };
            assert_eq!(visits, expected, "range {range:?}");
        }
    });
}

#[test]
fn bounded_dense_fold_and_with_id_fold_stop_at_the_producer_end() {
    let (world, _) = setup();
    world.run(|values: View<'_, Value>| {
        let (count, visits) =
            scan_chunks(|| producer(values.modified(), 128..256).fold(0, |count, _| count + 1));
        assert_eq!((count, visits), (0, vec![2, 3]));
        let (count, visits) = scan_chunks(|| {
            producer(values.modified(), 128..256)
                .with_id()
                .fold(0, |count, _| count + 1)
        });
        assert_eq!((count, visits), (0, vec![2, 3]));
    });
}

#[test]
fn bounded_dense_partial_chunk_preserves_exact_tracking_and_ids() {
    let (world, ids) = setup();
    world.run(|values: View<'_, Value>| {
        for range in [64..70, 64..71, 70..71, 71..128, 4096..4097, 70..70] {
            let (actual, visits) = scan_chunks(|| {
                producer(values.modified(), range.clone()).with_id().fold(
                    Vec::new(),
                    |mut found, (id, value)| {
                        found.push((id, value.0));
                        found
                    },
                )
            });
            let expected = [70, 4096]
                .into_iter()
                .filter(|i| range.contains(i))
                .map(|i| (ids[i], i))
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "range {range:?}");
            assert!(visits
                .iter()
                .all(|&chunk| { chunk >= range.start / 64 && chunk < range.end.div_ceil(64) }));
            if range.is_empty() {
                assert!(visits.is_empty());
            }
        }
    });
}

#[test]
fn bounded_dense_tracking_wrappers_and_mutable_outputs_keep_the_range() {
    let (mut world, ids) = setup();
    world.add_entity(Value(4097));
    world.run(|mut values: ViewMut<'_, Value>| {
        for (count, visits) in [
            scan_chunks(|| producer(values.inserted(), 128..256).count()),
            scan_chunks(|| producer(values.inserted_or_modified(), 128..256).count()),
            scan_chunks(|| producer(values.inserted_mut(), 128..256).count()),
            scan_chunks(|| producer(values.modified_mut(), 128..256).count()),
            scan_chunks(|| producer(values.inserted_or_modified_mut(), 128..256).count()),
        ] {
            assert_eq!((count, visits), (0, vec![2, 3]));
        }
        let changed = producer(values.modified_mut(), 64..71).with_id().fold(
            Vec::new(),
            |mut changed, (id, mut value)| {
                value.modify(|value| value.0 += 1);
                changed.push(id);
                changed
            },
        );
        assert_eq!(changed, vec![ids[70]]);
        assert_eq!(values.get(ids[70]).expect("test entity exists").0, 71);
        assert_eq!(values.get(ids[4096]).expect("test entity exists").0, 4096);
    });
}

#[test]
fn bounded_dense_mixed_and_both_or_sources_forward_the_end() {
    let (mut world, _) = setup();
    for _ in 0..8194 {
        world.add_entity(Other);
    }
    world.run(|values: View<'_, Value>, others: View<'_, Other>| {
        let iter = producer((values.modified(), &others), 128..256);
        assert_eq!(
            iter.shiperator.mask, 1,
            "tracking input must drive the join"
        );
        let (count, visits) = scan_chunks(|| iter.with_id().count());
        assert_eq!((count, visits), (0, vec![2, 3]));
        let (count, visits) = scan_chunks(|| {
            producer(values.modified() | values.inserted(), 128..256)
                .with_id()
                .count()
        });
        assert_eq!((count, visits), (0, vec![2, 3]));

        let (mut captain, _, _) = (values.modified() | values.inserted())
            .into_shiperator_with_budget(&mut Default::default(), 0);
        captain.next_slice();
        let (next, visits) = scan_chunks(|| captain.next_possible_in(128, 256));
        assert_eq!((next, visits), (256, vec![2, 3]));

        let (mut captain, _, _) = (values.inserted()
            | (values.modified() | values.inserted_or_modified()))
        .into_shiperator_with_budget(&mut Default::default(), 0);
        for source in 0..3 {
            captain.set_slice(source);
            let (next, visits) = scan_chunks(|| captain.next_possible_in(128, 256));
            assert_eq!((next, visits), (256, vec![2, 3]));
        }
    });
}

#[test]
fn bounded_dense_forward_scan_respects_the_consumed_back_cursor() {
    let (world, ids) = setup();
    world.run(|values: View<'_, Value>| {
        let mut iter = producer(values.modified(), 0..4097).with_id();
        assert_eq!(
            iter.next_back().map(|(id, v)| (id, v.0)),
            Some((ids[4096], 4096))
        );
        assert_eq!(iter.next().map(|(id, v)| (id, v.0)), Some((ids[70], 70)));
        let (item, visits) = scan_chunks(|| iter.next());
        assert!(item.is_none());
        assert!(visits.iter().all(|&chunk| chunk < 64));
        assert_eq!(
            producer(values.modified(), 64..71).with_id().rfold(
                Vec::new(),
                |mut found, (id, v)| {
                    found.push((id, v.0));
                    found
                }
            ),
            vec![(ids[70], 70)]
        );
    });
}

#[cfg(feature = "parallel")]
#[test]
fn bounded_dense_split_producers_do_not_rescan_the_clean_tail() {
    use rayon::iter::plumbing::UnindexedProducer;

    let (world, _) = setup();
    world.run(|values: View<'_, Value>| {
        let (left, right) = producer(values.modified(), 128..4096).split();
        let right = right.expect("a nonempty producer can split");
        let (count, visits) = scan_chunks(|| left.count());
        assert_eq!(count, 0);
        assert_eq!(visits, (2..33).collect::<Vec<_>>());
        let (count, visits) = scan_chunks(|| right.with_id().count());
        assert_eq!(count, 0);
        assert_eq!(visits, (33..64).collect::<Vec<_>>());
    });
}
