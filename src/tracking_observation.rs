//! Opt-in query evidence for a single observed World.
//!
//! The callback observes successful tracking predicates and values actually
//! tested by positive tracking iterators. An iterator predicate hit can still
//! be rejected by a later tuple join/filter; it is not a final loop-body count.
//! Deleted/removed iterator hits are recorded at their actual return points.
//! The observer does not run an extra ECS query.
//! Raw timestamps associate nested Rayon queries with their workload system.
//! Non-workload borrows, negated tracking filters, Unique tracking and direct
//! advanced::Tracking calls are not observed. This is not a mutation audit.
//! The callback must not panic, block or reenter Shipyard.

use crate::entity_id::EntityId;
use crate::tracking::TrackingTimestamp;
use core::any::type_name;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// One positive query observation. Entity is EntityId::inner(), not a game ID.
#[derive(Clone, Copy, Debug)]
pub struct TrackingHit {
    pub system: &'static str,
    pub component: &'static str,
    /// inserted, modified, inserted_or_modified, deleted, removed, or
    /// removed_or_deleted. Union filters retain their union kind.
    pub kind: &'static str,
    pub entity: u64,
    /// System-start tracking timestamp, not elapsed time or a tick number.
    pub current: u64,
}

type Observer = fn(TrackingHit);
static OBSERVER: OnceLock<Observer> = OnceLock::new();
static SYSTEMS: OnceLock<RwLock<HashMap<u64, &'static str>>> = OnceLock::new();

/// Register once before running workloads. There is no default observer or IO.
/// Only one World may execute observed workloads concurrently because tracking
/// timestamps are local to a World. A collision fails instead of misattributing.
pub fn set_observer(observer: fn(TrackingHit)) -> Result<(), &'static str> {
    OBSERVER
        .set(observer)
        .map_err(|_| "tracking observer already registered")
}

#[inline]
pub(crate) fn enabled() -> bool {
    OBSERVER.get().is_some()
}

pub(crate) struct SystemGuard(u64);

impl Drop for SystemGuard {
    fn drop(&mut self) {
        if let Some(systems) = SYSTEMS.get() {
            systems
                .write()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&self.0);
        }
    }
}

pub(crate) fn system_started(
    current: TrackingTimestamp,
    name: &'static str,
) -> Option<SystemGuard> {
    OBSERVER.get()?;
    let mut systems = SYSTEMS
        .get_or_init(|| RwLock::new(HashMap::new()))
        .write()
        .unwrap_or_else(|p| p.into_inner());
    assert!(
        systems.insert(current.get(), name).is_none(),
        "tracking observer encountered overlapping World timestamps"
    );
    Some(SystemGuard(current.get()))
}

#[inline]
pub(crate) fn hit<T>(kind: &'static str, entity: EntityId, current: TrackingTimestamp) {
    let Some(observer) = OBSERVER.get() else {
        return;
    };
    let Some(systems) = SYSTEMS.get() else {
        return;
    };
    let system = systems
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .get(&current.get())
        .copied();
    if let Some(system) = system {
        observer(TrackingHit {
            system,
            component: type_name::<T>(),
            kind,
            entity: entity.inner(),
            current: current.get(),
        });
    }
}

#[inline]
pub(crate) fn matched<T>(
    value: bool,
    kind: &'static str,
    entity: EntityId,
    current: TrackingTimestamp,
) -> bool {
    if value {
        hit::<T>(kind, entity, current);
    }
    value
}
