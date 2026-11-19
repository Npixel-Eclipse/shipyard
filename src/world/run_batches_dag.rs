//! Opt-in experiment: ready successors may cross incidental batch boundaries.
//! This intentionally changes how much independent work can already have run
//! when an unanticipated error/panic is observed. Explicit fences are retained.

use crate::error::{Run, RunWorkload};
use crate::scheduler::dag::{DagEpoch, DagGraph};
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
            self.world.run_dag_system(
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
        #[cfg(feature = "tracing")]
        let parent_span = info_span!("workload", name = ?workload_name);
        #[cfg(feature = "tracing")]
        let _parent_span = parent_span.enter();
        let run = || -> Result<(), RunWorkload> {
            if let Some(plan) = &batches.dag {
                for epoch in &plan.epochs {
                    match epoch {
                        DagEpoch::Fence(batch) => self.run_dag_fence_batch(
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
                    self.run_dag_fence_batch(
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

    fn run_dag_system(
        &self,
        systems: &[SystemFn],
        names: &[Box<dyn Label>],
        #[cfg(feature = "tracing")] parent: &Span,
        index: usize,
    ) -> Result<(), RunWorkload> {
        #[cfg(feature = "tracing")]
        let span = info_span!(parent: parent.clone(), "system", name = ?names[index]);
        #[cfg(feature = "tracing")]
        let _span = span.enter();
        (systems[index])(self).map_err(|error| RunWorkload::Run((names[index].clone(), error)))
    }

    // A verbatim-semantic baseline batch for condition/owner/fallible fences.
    // Integration may substitute the validated direct-dispatch batch helper;
    // condition evaluation and direct-error priority must remain unchanged.
    fn run_dag_fence_batch(
        &self,
        systems: &[SystemFn],
        names: &[Box<dyn Label>],
        batches: &Batches,
        batch_index: usize,
        #[cfg(feature = "tracing")] parent: &Span,
    ) -> Result<(), RunWorkload> {
        let batch = &batches.parallel[batch_index];
        let conditions = &batches.parallel_run_if[batch_index];
        let run_condition = |index: usize, condition: usize| -> Result<bool, RunWorkload> {
            if condition == usize::MAX {
                Ok(true)
            } else {
                (batches.systems_run_if[condition])(self)
                    .map_err(|error| RunWorkload::Run((names[index].clone(), error)))
            }
        };
        let direct_eligible = match batch.0 {
            Some(index) => run_condition(index, conditions.0)?,
            None => true,
        };
        let parallel_eligible = batch
            .1
            .iter()
            .zip(&conditions.1)
            .map(|(&index, &condition)| run_condition(index, condition))
            .collect::<Result<Vec<_>, _>>()?;
        let mut start = 0;
        let direct = batch.0.filter(|_| direct_eligible).or_else(|| {
            let first = batch.1.first().copied().filter(|_| parallel_eligible[0]);
            if first.is_some() {
                start = 1;
            }
            first
        });
        let mut parallel_result = Ok(());
        in_place_scope(|scope| -> Result<(), RunWorkload> {
            if start < batch.1.len() {
                scope.spawn(|_| {
                    use rayon::prelude::*;
                    parallel_result = batch.1[start..]
                        .par_iter()
                        .zip(&parallel_eligible[start..])
                        .try_for_each(|(&index, &eligible)| {
                            if !eligible {
                                return Ok(());
                            }
                            self.run_dag_system(
                                systems,
                                names,
                                #[cfg(feature = "tracing")]
                                parent,
                                index,
                            )
                        });
                });
            }
            if let Some(index) = direct {
                self.run_dag_system(
                    systems,
                    names,
                    #[cfg(feature = "tracing")]
                    parent,
                    index,
                )?;
            }
            Ok(())
        })?;
        parallel_result
    }
}

#[cfg(test)]
mod tests;
