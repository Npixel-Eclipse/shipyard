use super::*;
use crate::scheduler::dag::DagNode;
use crate::scheduler::{Step, SystemModificator};
use crate::error::SystemError;
use crate::track::Untracked;
use crate::{AllStoragesViewMut, Component, ViewMut, Workload};
use alloc::sync::Arc;
use alloc::vec;
use rayon::prelude::*;
use rayon::ThreadPoolBuilder;
use std::io::Error;
use std::sync::{Barrier, Condvar};
use std::thread::current;
use std::time::Duration;

struct X;
struct Y;
impl Component for X {
    type Tracking = Untracked;
}
impl Component for Y {
    type Tracking = Untracked;
}

fn run(world: &World) -> Result<(), RunWorkload> {
    world.run_workload("test")
}

fn two_workers() -> World {
    let Ok(pool) = ThreadPoolBuilder::new().num_threads(2).build() else {
        panic!("test pool");
    };
    World::builder().with_local_thread_pool(pool).build()
}

fn assert_first_batch_is_fenced(world: &World) {
    let scheduler = world.scheduler.borrow().expect("scheduler borrow");
    let batches = scheduler.workload(&"test").expect("registered workload");
    let plan = &batches.dag;
    assert!(matches!(plan.epochs.first(), Some(DagEpoch::Fence(0))), "fallible batch must be fenced");
}

#[derive(Default)]
struct Signal(Mutex<bool>, Condvar);
impl Signal {
    fn set(&self) {
        let Ok(mut done) = self.0.lock() else {
            panic!("signal lock");
        };
        *done = true;
        self.1.notify_all();
    }
    fn wait(&self) -> bool {
        let Ok(done) = self.0.lock() else {
            panic!("signal lock");
        };
        let Ok((done, _)) = self
            .1
            .wait_timeout_while(done, Duration::from_secs(3), |done| !*done)
        else {
            panic!("signal wait");
        };
        *done
    }
}

#[test]
fn ready_successor_runs_before_unrelated_previous_batch_body_finishes() {
    let world = two_workers();
    let child = Arc::new(Signal::default());
    let observed = Arc::new(AtomicBool::new(false));
    let slow_signal = Arc::clone(&child);
    let slow_observed = Arc::clone(&observed);
    let workload = Workload::new("test")
        .with_system(move |_: ViewMut<'_, Y>| {
            slow_observed.store(slow_signal.wait(), Ordering::SeqCst);
        })
        .with_system(|_: ViewMut<'_, X>| {})
        .with_system(move |_: ViewMut<'_, X>| child.set());
    assert!(workload.add_to_world(&world).is_ok());
    #[cfg(feature = "workload-order")]
    {
        let Ok(order) = world.workload_execution_order("test") else {
            panic!("registered execution order");
        };
        assert!(!order.happens_before(0, 2));
        assert!(order.happens_before(1, 2));
        assert!(!order.happens_before(2, 1));
        assert!(!order.happens_before(2, 2));
        assert!(!order.happens_before(1, 3));
    }
    assert!(run(&world).is_ok());
    assert!(
        observed.load(Ordering::SeqCst),
        "incidental batch barrier was retained"
    );
}

#[test]
fn condition_is_evaluated_after_prior_batch_and_before_its_own_body() {
    let world = two_workers();
    let state = Arc::new(AtomicUsize::new(0));
    let a = Arc::clone(&state);
    let b = Arc::clone(&state);
    let predicate = Arc::clone(&state);
    let called = Arc::new(AtomicUsize::new(0));
    let result = Arc::clone(&called);
    let workload = Workload::new("test")
        .with_system(move |_: ViewMut<'_, X>| {
            a.store(1, Ordering::SeqCst);
        })
        .with_system(
            (move |_: ViewMut<'_, X>| {
                assert_eq!(b.load(Ordering::SeqCst), 1);
                b.store(2, Ordering::SeqCst);
                result.fetch_add(1, Ordering::SeqCst);
            })
            .run_if(move || predicate.load(Ordering::SeqCst) == 1),
        );
    assert!(workload.add_to_world(&world).is_ok());
    assert!(run(&world).is_ok());
    assert_eq!(state.load(Ordering::SeqCst), 2);
    assert_eq!(called.load(Ordering::SeqCst), 1);
}

#[test]
fn all_storages_stays_on_owner_and_explicit_barrier_is_preserved() {
    let world = two_workers();
    let owner = current().id();
    let state = Arc::new(AtomicUsize::new(0));
    let before = Arc::clone(&state);
    let during = Arc::clone(&state);
    let after = Arc::clone(&state);
    let workload = Workload::new("test")
        .with_system(move || {
            before.store(1, Ordering::SeqCst);
        })
        .with_barrier()
        .with_system(move |_: AllStoragesViewMut<'_>| {
            assert_eq!(current().id(), owner);
            assert_eq!(during.load(Ordering::SeqCst), 1);
            during.store(2, Ordering::SeqCst);
        })
        .with_barrier()
        .with_system(move || {
            assert_eq!(after.load(Ordering::SeqCst), 2);
        });
    assert!(workload.add_to_world(&world).is_ok());
    #[cfg(feature = "workload-order")]
    {
        let Ok(order) = world.workload_execution_order("test") else {
            panic!("registered execution order");
        };
        assert!(order.happens_before(0, 1));
        assert!(order.happens_before(1, 2));
        assert!(order.happens_before(0, 2));
    }
    assert!(run(&world).is_ok());
}

#[test]
fn known_fallible_batch_keeps_later_independent_external_effects_unstarted() {
    for should_panic in [false, true] {
        let world = two_workers();
        let later = Arc::new(AtomicUsize::new(0));
        let dependent = Arc::clone(&later);
        let independent = Arc::clone(&later);
        let workload = Workload::new("test")
            .with_system(move |_: ViewMut<'_, X>| -> Result<(), SystemError> {
                assert!(!should_panic, "expected marked panic");
                Err(Error::other("expected marked error").into())
            })
            // Upstream packs every ready independent system into the first
            // batch. This root puts the later Y effect in the next batch.
            .with_system(|_: ViewMut<'_, Y>| {})
            .with_system(move |_: ViewMut<'_, X>| {
                dependent.fetch_add(1, Ordering::SeqCst);
            })
            .with_system(move |_: ViewMut<'_, Y>| {
                independent.fetch_add(1, Ordering::SeqCst);
            });
        assert!(workload.add_to_world(&world).is_ok());
        assert_first_batch_is_fenced(&world);
        let outcome = catch_unwind(AssertUnwindSafe(|| run(&world)));
        if should_panic {
            assert!(outcome.is_err());
        } else {
            assert!(matches!(outcome, Ok(Err(_))));
        }
        assert_eq!(later.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn reused_registered_fallible_function_retains_fallible_provenance() {
    fn fail(_: ViewMut<'_, X>) -> Result<(), SystemError> {
        Err(Error::other("registered as fallible system first").into())
    }
    let world = two_workers();
    assert!(Workload::new("registered-first")
        .with_system(fail)
        .add_to_world(&world)
        .is_ok());
    let later = Arc::new(AtomicUsize::new(0));
    let dependent = Arc::clone(&later);
    let independent = Arc::clone(&later);
    // The registry reuses the first TypeId's fallible callable across workloads.
    assert!(Workload::new("test")
        .with_system(fail)
        .with_system(|_: ViewMut<'_, Y>| {})
        .with_system(move |_: ViewMut<'_, X>| {
            dependent.fetch_add(1, Ordering::SeqCst);
        })
        .with_system(move |_: ViewMut<'_, Y>| {
            independent.fetch_add(1, Ordering::SeqCst);
        })
        .add_to_world(&world)
        .is_ok());
    assert_first_batch_is_fenced(&world);
    assert!(run(&world).is_err());
    assert_eq!(later.load(Ordering::SeqCst), 0);
}

#[test]
fn unexpected_panic_propagates_original_payload_but_early_independent_effect_is_possible() {
    let world = two_workers();
    let independent_done = Arc::new(Signal::default());
    let wait = Arc::clone(&independent_done);
    let dependent_count = Arc::new(AtomicUsize::new(0));
    let dependent = Arc::clone(&dependent_count);
    let independent_count = Arc::new(AtomicUsize::new(0));
    let independent = Arc::clone(&independent_count);
    let workload = Workload::new("test")
        .with_system(move |_: ViewMut<'_, X>| {
            assert!(wait.wait(), "independent root must have been admitted");
            panic!("original DAG panic payload");
        })
        .with_system(move |_: ViewMut<'_, X>| {
            dependent.fetch_add(1, Ordering::SeqCst);
        })
        .with_system(move |_: ViewMut<'_, Y>| {
            independent.fetch_add(1, Ordering::SeqCst);
            independent_done.set();
        });
    assert!(workload.add_to_world(&world).is_ok());
    let outcome = catch_unwind(AssertUnwindSafe(|| run(&world)));
    let Err(payload) = outcome else {
        panic!("expected original panic");
    };
    assert_eq!(
        payload.downcast_ref::<&str>().copied(),
        Some("original DAG panic payload")
    );
    assert_eq!(dependent_count.load(Ordering::SeqCst), 0);
    assert_eq!(independent_count.load(Ordering::SeqCst), 1);
}

#[test]
fn concurrent_errors_choose_lowest_node_and_do_not_release_successors() {
    let world = two_workers();
    let gate = Arc::new(Barrier::new(2));
    let first = Arc::clone(&gate);
    let successor_count = Arc::new(AtomicUsize::new(0));
    let successor = Arc::clone(&successor_count);
    let systems: Vec<SystemFn> = vec![
        Box::new(move |_| {
            first.wait();
            Err(Run::from_custom(Error::other("first")))
        }),
        Box::new(move |_| {
            gate.wait();
            Err(Run::from_custom(Error::other("second")))
        }),
        Box::new(move |_| {
            successor.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
    ];
    let names: Vec<Box<dyn Label>> = vec![Box::new("first"), Box::new("second"), Box::new("never")];
    let graph = DagGraph {
        nodes: vec![
            DagNode {
                system_index: 0,
                predecessors: 0,
                successors: vec![2],
            },
            DagNode {
                system_index: 1,
                predecessors: 0,
                successors: vec![2],
            },
            DagNode {
                system_index: 2,
                predecessors: 2,
                successors: vec![],
            },
        ],
        roots: vec![0, 1],
        #[cfg(feature = "workload-order")]
        occurrences: vec![0, 1, 2],
    };
    #[cfg(feature = "tracing")]
    let span = Span::none();
    let execute = || {
        world.run_dag_graph(
            &systems,
            &names,
            &graph,
            #[cfg(feature = "tracing")]
            &span,
        )
    };
    let result = match &world.thread_pool {
        Some(pool) => pool.install(execute),
        None => execute(),
    };
    let Err(RunWorkload::Run((label, _))) = result else {
        panic!("expected system error");
    };
    assert!(label.dyn_eq(&"first"));
    assert_eq!(successor_count.load(Ordering::SeqCst), 0);
}

#[test]
fn repeated_occurrences_reinitialize_each_tick_and_join_nested_rayon() {
    let world = two_workers();
    let count = Arc::new(AtomicUsize::new(0));
    let actual = Arc::clone(&count);
    let repeated = move || {
        (0..64).into_par_iter().for_each(|_| {
            actual.fetch_add(1, Ordering::SeqCst);
        });
    };
    let checks = Arc::new(AtomicUsize::new(0));
    let checker = Arc::clone(&checks);
    let total = Arc::clone(&count);
    assert!(Workload::new("test")
        .with_system(repeated.clone())
        .with_system(repeated)
        .with_barrier()
        .with_system(move || {
            let tick = checker.fetch_add(1, Ordering::SeqCst) + 1;
            assert_eq!(total.load(Ordering::SeqCst), 128 * tick);
        })
        .add_to_world(&world)
        .is_ok());
    for _ in 0..3 {
        assert!(run(&world).is_ok());
    }
    assert_eq!(count.load(Ordering::SeqCst), 384);
    assert_eq!(checks.load(Ordering::SeqCst), 3);
}

#[test]
fn reused_read_only_tracking_system_keeps_its_original_batch_boundaries() {
    use crate::track::All;
    use crate::{IntoIter, View};

    struct Tracked;
    impl Component for Tracked {
        type Tracking = All;
    }
    fn unrelated(_: ViewMut<'_, X>) {}

    let mut world = two_workers();
    let counts = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&counts);
    let read_tracked = move |values: View<'_, Tracked>| {
        let count = values.inserted_or_modified().iter().count();
        let Ok(mut observations) = observed.lock() else {
            panic!("tracking observations");
        };
        observations.push(count);
    };
    assert!(Workload::new("test")
        .with_system(read_tracked.clone())
        .with_system(unrelated)
        .with_system(read_tracked.clone().after_all(unrelated))
        .with_system(read_tracked.after_all(unrelated))
        .add_to_world(&world)
        .is_ok());

    {
        let Ok(scheduler) = world.scheduler.borrow() else {
            panic!("scheduler borrow");
        };
        let Ok(batches) = scheduler.workload(&"test") else {
            panic!("registered workload");
        };
        assert_eq!(batches.parallel, vec![(None, vec![Step::System(0), Step::System(1)]), (None, vec![Step::System(0), Step::System(0)])]);
        let plan = &batches.dag;
        let [DagEpoch::Graph(graph)] = plan.epochs.as_slice() else {
            panic!("read-only systems must remain in one graph epoch");
        };
        // All three occurrences reuse the same registered tracking closure.
        // These assertions detect the old bug deterministically, without
        // depending on which ready root a Rayon worker happens to steal.
        assert_eq!(graph.nodes[0].system_index, graph.nodes[2].system_index);
        assert_eq!(graph.nodes[0].system_index, graph.nodes[3].system_index);
        assert_eq!(graph.nodes[0].successors, vec![2, 3]);
        assert!(graph.nodes[2].successors.is_empty());
        assert!(graph.nodes[3].successors.is_empty());
    }

    for _ in 0..3 {
        world.add_entity((Tracked,));
        assert!(run(&world).is_ok());
    }
    // Each tick's first occurrence consumes the insertion window; both later
    // occurrences retain their original empty window and remain unordered.
    let Ok(observations) = counts.lock() else {
        panic!("tracking observations");
    };
    assert_eq!(*observations, vec![1, 0, 0, 1, 0, 0, 1, 0, 0]);
}

#[cfg(feature = "thread_local")]
#[test]
fn non_send_sync_borrow_remains_on_owner_thread() {
    use crate::borrow::NonSendSync;
    use crate::View;
    use std::rc::Rc;
    struct Local {
        _not_send_sync: Rc<()>,
    }
    impl Component for Local {
        type Tracking = Untracked;
    }
    let world = two_workers();
    let owner = current().id();
    assert!(Workload::new("test")
        .with_system(move |_: NonSendSync<View<'_, Local>>| {
            assert_eq!(current().id(), owner);
        })
        .add_to_world(&world)
        .is_ok());
    assert!(run(&world).is_ok());
}
