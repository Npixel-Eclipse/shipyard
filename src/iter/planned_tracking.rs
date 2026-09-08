use super::{ShiperatorCaptain, ShiperatorOutput, ShiperatorSailor};
use crate::{entity_id::EntityId, sparse_set::TrackingPlan};

/// Internal tracking wrapper carrying the current query's chunk plan.
#[doc(hidden)]
#[derive(Clone)]
pub struct PlannedTracking<S> {
    inner: S,
    plan: TrackingPlan,
}

impl<S> PlannedTracking<S> {
    pub(crate) fn new(inner: S, plan: TrackingPlan) -> Self {
        Self { inner, plan }
    }
}

impl<S: ShiperatorOutput> ShiperatorOutput for PlannedTracking<S> {
    type Out = S::Out;
}

impl<S: ShiperatorCaptain> ShiperatorCaptain for PlannedTracking<S> {
    #[inline]
    fn has_stable_membership(&self) -> bool {
        self.plan.is_empty() || self.inner.has_stable_membership()
    }
    #[inline]
    unsafe fn get_captain_data(&self, index: usize) -> Self::Out {
        self.inner.get_captain_data(index)
    }
    #[inline]
    fn next_slice(&mut self) {
        self.inner.next_slice();
    }
    #[inline]
    fn sail_time(&self) -> usize {
        self.plan.scan_cost()
    }
    #[inline]
    fn is_exact_sized(&self) -> bool {
        false
    }
    #[inline]
    fn unpick(&mut self) {
        self.inner.unpick();
        // Sailors only need per-entity probes and the empty-input proof. Release
        // sparse ranges so producer clones don't retain unused plans/Arc refs.
        if !self.has_no_candidates() {
            self.plan = TrackingPlan::Dense {
                len: self.inner.sail_time(),
            };
        }
    }
    #[inline]
    fn has_no_candidates(&self) -> bool {
        self.plan.is_empty()
    }
    #[inline]
    fn next_possible(&self, index: usize) -> usize {
        if matches!(self.plan, TrackingPlan::Dense { .. }) {
            return self.inner.next_possible(index);
        }
        self.plan.next(index)
    }
    #[inline]
    fn previous_possible(&self, end: usize) -> usize {
        self.plan.previous(end)
    }
    #[cfg(feature = "parallel")]
    #[inline]
    fn candidate_count(&self, start: usize, end: usize) -> usize {
        self.plan.count(start, end)
    }
    #[cfg(feature = "parallel")]
    #[inline]
    fn candidate_midpoint(&self, start: usize, end: usize) -> usize {
        self.plan.midpoint(start, end)
    }
}

impl<S: ShiperatorSailor> ShiperatorSailor for PlannedTracking<S> {
    type Index = S::Index;
    #[inline]
    unsafe fn get_sailor_data(&self, index: Self::Index) -> Self::Out {
        self.inner.get_sailor_data(index)
    }
    #[inline]
    fn indices_of(&self, eid: EntityId, index: usize) -> Option<Self::Index> {
        if self.plan.is_empty() {
            return None;
        }
        self.inner.indices_of(eid, index)
    }
    #[inline]
    fn captain_indices_of(&self, eid: EntityId, index: usize) -> Option<Self::Index> {
        self.inner.captain_indices_of(eid, index)
    }
    #[inline]
    fn index_from_usize(index: usize) -> Self::Index {
        S::index_from_usize(index)
    }
}
