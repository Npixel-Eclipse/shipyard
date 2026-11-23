use super::{Batches, DagEpoch, Label, RunWorkload, SystemFn, World};
use crate::scheduler::order_witness::OrderWitness;
use crate::scheduler::Step;
use alloc::boxed::Box;
use alloc::vec::Vec;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::Mutex;
#[cfg(feature = "tracing")]
use tracing::Span;

impl World {
    pub(super) fn run_order_witness(
        &self,
        systems: &[SystemFn],
        names: &[Box<dyn Label>],
        batches: &Batches,
        witness: &OrderWitness,
        #[cfg(feature = "tracing")] parent: &Span,
    ) -> Result<(), RunWorkload> {
        let tick = witness.begin();
        let events = Mutex::new(Vec::<[usize; 2]>::new());
        let record = |kind, occurrence| {
            events.lock().unwrap_or_else(|p| p.into_inner()).push([kind, occurrence]);
        };
        let run_one = |occurrence, system| {
            record(0, occurrence);
            let result = catch_unwind(AssertUnwindSafe(|| self.run_single_system(
                systems, names,
                #[cfg(feature = "tracing")]
                parent,
                system,
            )));
            match result {
                Ok(Ok(())) => { record(1, occurrence); Ok(()) }
                Ok(Err(error)) => { record(2, occurrence); Err(error) }
                Err(payload) => { record(3, occurrence); resume_unwind(payload) }
            }
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            let plan = batches.dag.as_ref().expect("validated witness lost its DAG");
            for (epoch_index, epoch) in plan.epochs.iter().enumerate() {
                match epoch {
                    DagEpoch::Graph(graph) => {
                        for &local in &witness.epoch_orders[epoch_index] {
                            let node = &graph.nodes[local];
                            run_one(node.occurrence, node.system_index)?;
                        }
                    }
                    DagEpoch::Fence(batch) => {
                        self.run_one_batch_with(batches, *batch, |step, run_if| {
                            let Step::System(system) = step else {
                                unreachable!("a validated order witness cannot contain regroups");
                            };
                            if !self.should_run(batches, names, system, run_if)? {
                                return Ok(());
                            }
                            let occurrence = witness.batch_occurrences[*batch].iter().copied()
                                .find(|&id| witness.occurrence_systems[id] == system)
                                .expect("validated fence occurrence missing");
                            run_one(occurrence, system)
                        })?;
                    }
                }
            }
            Ok(())
        }));
        let events = events.into_inner().unwrap_or_else(|p| p.into_inner());
        match result {
            Ok(result) => {
                witness.record_tick(tick.tick, if result.is_ok() { "ok" } else { "error" }, &events);
                result
            }
            Err(payload) => {
                witness.record_tick(tick.tick, "panic", &events);
                resume_unwind(payload)
            }
        }
    }
}
