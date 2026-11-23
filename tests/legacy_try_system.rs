#![cfg(feature = "std")]

use shipyard::track::Untracked;
use shipyard::{Component, IntoWorkloadTrySystem, SystemModificator, View, Workload, World};
use std::io::Error;

struct Marker;
impl Component for Marker {
    type Tracking = Untracked;
}

fn plain_failure() -> Result<(), Error> {
    Err(Error::other("legacy plain failure"))
}

fn borrowed_failure(_: View<Marker>) -> Result<(), Error> {
    Err(Error::other("legacy borrowed failure"))
}

#[test]
fn legacy_try_wrappers_keep_errors_and_reused_callable_provenance() {
    let world = World::new();
    assert!(
        Workload::new("plain")
            .with_try_system(plain_failure)
            .add_to_world(&world)
            .is_ok()
    );
    assert!(
        Workload::new("borrowed")
            .with_try_system(borrowed_failure)
            .add_to_world(&world)
            .is_ok()
    );
    assert!(
        Workload::new("reused")
            .with_system(borrowed_failure)
            .add_to_world(&world)
            .is_ok()
    );
    for label in ["plain", "borrowed", "reused"] {
        let Err(error) = world.run_workload(label) else {
            panic!("legacy error must propagate");
        };
        assert!(error.custom_error().is_some());
    }
}

#[test]
fn converted_legacy_try_system_keeps_its_condition() {
    let world = World::new();
    let Ok(system) = borrowed_failure.into_workload_try_system() else {
        panic!("valid system");
    };
    assert!(
        Workload::new("skipped")
            .with_try_system(system.run_if(|| false))
            .add_to_world(&world)
            .is_ok()
    );
    assert!(world.run_workload("skipped").is_ok());
}
