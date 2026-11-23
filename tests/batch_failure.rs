//! Preserve the failure boundary between actual batches, including external effects.
#![cfg(all(feature = "parallel", feature = "std"))]

use shipyard::{track, Component, ViewMut, Workload, World};
use shipyard::error::SystemError;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct X;
struct Y;
impl Component for X {
    type Tracking = track::Untracked;
}
impl Component for Y {
    type Tracking = track::Untracked;
}

fn assert_failure_boundary(panic_first: bool) -> Result<(), Box<dyn std::error::Error>> {
    let independent_effects = Arc::new(AtomicUsize::new(0));
    let later_effects = Arc::new(AtomicUsize::new(0));
    let independent = Arc::clone(&independent_effects);
    let later_x = Arc::clone(&later_effects);
    let later_y = Arc::clone(&later_effects);
    // Borrow conflicts form A -> B and C -> D. C represents a producer/other external write.
    let (workload, _) = Workload::new("failure-boundary")
        .with_system(move |_: ViewMut<X>| -> Result<(), SystemError> {
            assert!(!panic_first, "expected first-system panic");
            Err(std::io::Error::other("expected first-system error").into())
        })
        // Independent roots may share the first batch in the current scheduler.
        // Give Y its own predecessor so the observable effect is in a later batch.
        .with_system(|_: ViewMut<Y>| {})
        .with_system(move |_: ViewMut<X>| {
            later_x.fetch_add(1, Ordering::SeqCst);
        })
        .with_system(move |_: ViewMut<Y>| {
            independent.fetch_add(1, Ordering::SeqCst);
        })
        .with_system(move |_: ViewMut<Y>| {
            later_y.fetch_add(1, Ordering::SeqCst);
        })
        .build()?;
    let world = World::new();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        workload.run_with_world(&world)
    }));
    if panic_first {
        assert!(outcome.is_err());
    } else {
        assert!(matches!(outcome, Ok(Err(_))));
    }
    assert_eq!(later_effects.load(Ordering::SeqCst), 0);
    assert_eq!(
        independent_effects.load(Ordering::SeqCst),
        0,
        "a system from a later batch ran before the earlier failure was observed"
    );
    Ok(())
}

#[test]
fn error_preserves_independent_later_batch_effects() -> Result<(), Box<dyn std::error::Error>> {
    assert_failure_boundary(false)
}

#[test]
fn panic_preserves_independent_later_batch_effects() -> Result<(), Box<dyn std::error::Error>> {
    assert_failure_boundary(true)
}
