use super::*;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use rayon::ThreadPoolBuilder;
use std::io::Error;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;
use std::thread::current;

type System = Box<dyn Fn(&World) -> Result<(), Run> + Send + Sync>;

fn names(count: usize) -> Vec<Box<dyn Label>> {
    (0..count)
        .map(|id| Box::new(format!("{id}")) as Box<dyn Label>)
        .collect()
}

fn batches(slots: Vec<(Option<usize>, Vec<usize>)>) -> Batches {
    Batches {
        parallel_run_if: slots
            .iter()
            .map(|(_, ids)| (usize::MAX, vec![usize::MAX; ids.len()]))
            .collect(),
        parallel: slots,
        ..Batches::default()
    }
}

fn run(systems: &[System], batches: &Batches, workers: usize) -> Result<(), RunWorkload> {
    let pool = ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .expect("test thread pool");
    pool.install(|| {
        World::new().run_batches_direct(systems, &names(systems.len()), batches, &"experiment")
    })
}

fn error(message: &str) -> Run {
    Run::from_custom(Error::other(String::from(message)))
}

#[test]
fn mode_defaults_to_baseline_and_only_direct_opts_in() {
    assert_eq!(Mode::parse(None), Mode::Baseline);
    assert_eq!(Mode::parse(Some("baseline")), Mode::Baseline);
    assert_eq!(Mode::parse(Some("direct")), Mode::Direct);
}

#[test]
#[should_panic(expected = "Unsupported BATTLE_SCHEDULER_MODE")]
fn unknown_mode_is_rejected() {
    Mode::parse(Some("unknown"));
}

#[test]
fn exactly_once_nested_parallel_and_next_batch_barrier() {
    for workers in [1, 4] {
        let hits = Arc::new((0..9).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
        let nested = Arc::new(AtomicUsize::new(0));
        let mut systems: Vec<System> = (0..8)
            .map(|id| {
                let hits = hits.clone();
                let nested = nested.clone();
                Box::new(move |_: &World| {
                    (0..64).into_par_iter().for_each(|_| {
                        nested.fetch_add(1, Ordering::SeqCst);
                    });
                    hits[id].fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }) as System
            })
            .collect();
        let last_hits = hits.clone();
        let last_nested = nested.clone();
        systems.push(Box::new(move |_| {
            assert!(last_hits[..8].iter().all(|x| x.load(Ordering::SeqCst) == 1));
            assert_eq!(last_nested.load(Ordering::SeqCst), 8 * 64);
            last_hits[8].fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));
        run(
            &systems,
            &batches(vec![(None, (0..8).collect()), (None, vec![8])]),
            workers,
        )
        .expect("complete workload");
        assert!(hits.iter().all(|x| x.load(Ordering::SeqCst) == 1));
    }
}

#[test]
fn run_if_is_ordered_and_runs_once_before_bodies() {
    let evaluated = Arc::new(Mutex::new(Vec::new()));
    let hits = Arc::new((0..4).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
    let systems: Vec<System> = (0..4)
        .map(|id| {
            let hits = hits.clone();
            let evaluated = evaluated.clone();
            Box::new(move |_: &World| {
                assert_eq!(*evaluated.lock().expect("evaluated lock"), vec![0, 1, 2, 3]);
                hits[id].fetch_add(1, Ordering::SeqCst);
                Ok(())
            }) as System
        })
        .collect();
    let mut batch = batches(vec![(None, (0..4).collect())]);
    batch.parallel_run_if[0].1 = (0..4).collect();
    batch.systems_run_if = (0..4)
        .map(|id| {
            let evaluated = evaluated.clone();
            Box::new(move |_: &World| {
                evaluated.lock().expect("evaluated lock").push(id);
                Ok(id % 2 == 1)
            }) as Box<dyn Fn(&World) -> Result<bool, Run> + Send + Sync>
        })
        .collect();
    run(&systems, &batch, 4).expect("filtered workload");
    assert_eq!(
        hits.iter()
            .map(|x| x.load(Ordering::SeqCst))
            .collect::<Vec<_>>(),
        vec![0, 1, 0, 1]
    );
}

#[test]
fn run_if_failure_dispatches_nothing_and_reports_its_system() {
    let hits = Arc::new(AtomicUsize::new(0));
    let systems: Vec<System> = (0..3)
        .map(|_| {
            let hits = hits.clone();
            Box::new(move |_: &World| {
                hits.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }) as System
        })
        .collect();
    let mut batch = batches(vec![(None, vec![0, 1]), (None, vec![2])]);
    batch.parallel_run_if[0].1[1] = 0;
    batch
        .systems_run_if
        .push(Box::new(|_| Err(error("run-if"))));
    let result = run(&systems, &batch, 4).expect_err("run_if failure");
    match result {
        RunWorkload::Run((name, err)) => {
            assert_eq!(format!("{name:?}"), "1");
            assert!(format!("{err}").contains("run-if"));
        }
        _ => panic!("wrong run_if error"),
    }
    assert_eq!(hits.load(Ordering::SeqCst), 0);
}

#[test]
fn caller_error_wins_and_does_not_cancel_successful_siblings() {
    for workers in [1, 4] {
        let siblings = Arc::new(AtomicUsize::new(0));
        let later = Arc::new(AtomicUsize::new(0));
        let mut systems: Vec<System> = vec![Box::new(|_| Err(error("caller")))];
        for _ in 1..8 {
            let siblings = siblings.clone();
            systems.push(Box::new(move |_| {
                siblings.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }));
        }
        let later_copy = later.clone();
        systems.push(Box::new(move |_| {
            later_copy.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));
        let result = run(
            &systems,
            &batches(vec![(None, (0..8).collect()), (None, vec![8])]),
            workers,
        )
        .expect_err("caller failure");
        assert!(format!("{result}").contains("caller"));
        assert_eq!(siblings.load(Ordering::SeqCst), 7);
        assert_eq!(later.load(Ordering::SeqCst), 0);
        systems[1] = Box::new(|_| Err(error("sibling")));
        let result = run(&systems, &batches(vec![(None, (0..8).collect())]), workers)
            .expect_err("both fail");
        assert!(format!("{result}").contains("caller"));
    }
}

#[test]
fn caller_panic_joins_siblings_without_running_next_batch() {
    for workers in [1, 4] {
        let siblings = Arc::new(AtomicUsize::new(0));
        let later = Arc::new(AtomicUsize::new(0));
        let mut systems: Vec<System> = vec![Box::new(|_| panic!("caller panic"))];
        for _ in 1..8 {
            let siblings = siblings.clone();
            systems.push(Box::new(move |_| {
                siblings.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }));
        }
        let later_copy = later.clone();
        systems.push(Box::new(move |_| {
            later_copy.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));
        let result = catch_unwind(AssertUnwindSafe(|| {
            run(
                &systems,
                &batches(vec![(None, (0..8).collect()), (None, vec![8])]),
                workers,
            )
        }));
        assert!(result.is_err());
        assert_eq!(siblings.load(Ordering::SeqCst), 7);
        assert_eq!(later.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn required_slot_preserves_caller_thread_and_filtered_fallback() {
    let caller = current().id();
    let hits = Arc::new(AtomicUsize::new(0));
    let required_hits = hits.clone();
    let systems: Vec<System> = vec![
        Box::new(move |_| {
            assert_eq!(current().id(), caller);
            required_hits.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
        Box::new(|_| Ok(())),
    ];
    let mut batch = batches(vec![(Some(0), vec![1])]);
    let world = World::new();
    world
        .run_batches_direct(&systems, &names(2), &batch, &"required")
        .expect("required slot");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    batch.parallel_run_if[0].0 = 0;
    batch.systems_run_if.push(Box::new(|_| Ok(false)));
    world
        .run_batches_direct(&systems, &names(2), &batch, &"required")
        .expect("fallback slot");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "tracing")]
mod tracing_tests;
