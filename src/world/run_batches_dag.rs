//! Opt-in experiment: ready successors may cross incidental batch boundaries.
//! This intentionally changes how much independent work can already have run
//! when an unanticipated error/panic is observed. Explicit fences are retained.

use crate::error::{Run, RunWorkload};
use crate::scheduler::dag::{DagEpoch, DagGraph};
use crate::scheduler::order_witness::ensure_bound as ensure_order_witness_bound;
use crate::scheduler::{Batches, Label};
use crate::world::World;
use alloc::boxed::Box;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use rayon::{in_place_scope, Scope};
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::Mutex;
#[cfg(feature = "tracing")]
use tracing::{info_span, Span};

type SystemFn = Box<dyn Fn(&World) -> Result<(), Run> + Send + Sync + 'static>;

struct Execution<'a> {
    world: &'a World,
    systems: &'a [SystemFn],
    names: &'a [Box<dyn Label>],
    graph: &'a DagGraph,
    remaining: Vec<AtomicUsize>,
    cancelled: AtomicBool,
    failure: Mutex<Option<(usize, RunWorkload)>>,
    #[cfg(feature = "tracing")]
    parent: &'a Span,
}

impl Execution<'_> {
    // Admission linearizes against cancellation. An admission that won the CAS
    // before cancellation is already in flight, even if Rayon starts it later.
    fn admit(&self) -> bool {
        self.cancelled
            .compare_exchange(false, false, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn execute<'scope>(&'scope self, id: usize, scope: &Scope<'scope>) {
        if self.cancelled.load(Ordering::Acquire) {
            return;
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.world.run_single_system(
                self.systems,
                self.names,
                #[cfg(feature = "tracing")]
                self.parent,
                self.graph.nodes[id].system_index,
            )
        }));
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.cancelled.store(true, Ordering::Release);
                let mut failure = match self.failure.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                // Stable selection among errors from already admitted work.
                if failure.as_ref().is_none_or(|(earlier, _)| id < *earlier) {
                    *failure = Some((id, error));
                }
                return;
            }
            Err(payload) => {
                self.cancelled.store(true, Ordering::Release);
                // Rayon joins all admitted tasks before propagating the original
                // payload. A failed node never releases its successors.
                resume_unwind(payload);
            }
        }
        for &next in &self.graph.nodes[id].successors {
            if self.remaining[next].fetch_sub(1, Ordering::AcqRel) == 1 && self.admit() {
                scope.spawn(move |scope| self.execute(next, scope));
            }
        }
    }
}

impl World {
    pub(crate) fn run_batches_dag(
        &self,
        systems: &[SystemFn],
        system_names: &[Box<dyn Label>],
        batches: &Batches,
        #[cfg_attr(not(feature = "tracing"), allow(unused))] workload_name: &dyn Label,
    ) -> Result<(), RunWorkload> {
        ensure_order_witness_bound();
        #[cfg(feature = "tracing")]
        let parent_span = info_span!("workload", name = ?workload_name);
        #[cfg(feature = "tracing")]
        let _parent_span = parent_span.enter();
        let run = || -> Result<(), RunWorkload> {
            if let Some(witness) = batches.dag.as_ref().and_then(|plan| plan.order_witness.as_ref()) {
                return self.run_order_witness(
                    systems,
                    system_names,
                    batches,
                    witness,
                    #[cfg(feature = "tracing")]
                    &parent_span,
                );
            }
            if let Some(plan) = &batches.dag {
                for epoch in &plan.epochs {
                    match epoch {
                        DagEpoch::Fence(batch) => self.run_one_batch_direct(
                            systems,
                            system_names,
                            batches,
                            *batch,
                            #[cfg(feature = "tracing")]
                            &parent_span,
                        )?,
                        DagEpoch::Graph(graph) => self.run_dag_graph(
                            systems,
                            system_names,
                            graph,
                            #[cfg(feature = "tracing")]
                            &parent_span,
                        )?,
                    }
                }
            } else {
                // Fail closed without reentering a mode router.
                for batch in 0..batches.parallel.len() {
                    self.run_one_batch_direct(
                        systems,
                        system_names,
                        batches,
                        batch,
                        #[cfg(feature = "tracing")]
                        &parent_span,
                    )?;
                }
            }
            Ok(())
        };
        if let Some(pool) = &self.thread_pool {
            pool.scope(|_| run())
        } else {
            run()
        }
    }

    fn run_dag_graph(
        &self,
        systems: &[SystemFn],
        names: &[Box<dyn Label>],
        graph: &DagGraph,
        #[cfg(feature = "tracing")] parent: &Span,
    ) -> Result<(), RunWorkload> {
        let execution = Execution {
            world: self,
            systems,
            names,
            graph,
            remaining: graph
                .nodes
                .iter()
                .map(|node| AtomicUsize::new(node.predecessors))
                .collect(),
            cancelled: AtomicBool::new(false),
            failure: Mutex::new(None),
            #[cfg(feature = "tracing")]
            parent,
        };
        in_place_scope(|scope| {
            if let Some((&first, others)) = graph.roots.split_first() {
                for &root in others {
                    if execution.admit() {
                        let execution = &execution;
                        scope.spawn(move |scope| execution.execute(root, scope));
                    }
                }
                if execution.admit() {
                    execution.execute(first, scope);
                }
            }
        });
        let failure = match execution.failure.into_inner() {
            Ok(failure) => failure,
            Err(poisoned) => poisoned.into_inner(),
        };
        match failure {
            Some((_, error)) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests;

mod order_witness;
