//! Disposable scheduling experiment. The ordinary scheduler remains the default.

use crate::error::{Run, RunWorkload};
use crate::scheduler::{Batches, Label, Step};
#[cfg(feature = "scheduler-registration-benchmark")]
use crate::scheduler::registration_benchmark::validate_execution_mode;
use crate::World;
use alloc::boxed::Box;
use alloc::format;
use core::any::Any;
use rayon::prelude::*;
use std::env::{var, VarError};
use std::fs::write;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
#[cfg(feature = "tracing")]
use tracing::{info_span, Span};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Baseline,
    Direct,
    Dag,
}

impl Mode {
    fn parse(value: Option<&str>) -> Self {
        match value {
            None | Some("baseline") => Self::Baseline,
            Some("direct") => Self::Direct,
            Some("dag") => Self::Dag,
            Some(value) => panic!("Unsupported BATTLE_SCHEDULER_MODE: {value}"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Direct => "direct",
            Self::Dag => "dag",
        }
    }
}

/// Read once per process; inspect only once per workload, never per system.
pub(super) fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| {
        let value = match var("BATTLE_SCHEDULER_MODE") {
            Ok(value) => Some(value),
            Err(VarError::NotPresent) => None,
            Err(error) => panic!("Invalid BATTLE_SCHEDULER_MODE: {error}"),
        };
        let selected = Mode::parse(value.as_deref());
        #[cfg(feature = "scheduler-registration-benchmark")]
        validate_execution_mode(selected == Mode::Baseline);
        let output = match var("BATTLE_SCHEDULER_MODE_OUTPUT") {
            Ok(path) => Some(path),
            Err(VarError::NotPresent) => None,
            Err(error) => panic!("Invalid BATTLE_SCHEDULER_MODE_OUTPUT: {error}"),
        };
        if let Some(path) = output {
            if let Err(error) = write(&path, format!("{{\"mode\":\"{}\"}}\n", selected.name())) {
                panic!("Cannot record BATTLE_SCHEDULER_MODE_OUTPUT {path:?}: {error}");
            }
        }
        selected
    })
}

#[derive(Default)]
struct BatchOutcome {
    direct_error: Option<RunWorkload>,
    sibling_error: Option<RunWorkload>,
    direct_panic: Option<Box<dyn Any + Send>>,
}

impl BatchOutcome {
    fn merge(self, other: Self) -> Self {
        Self {
            direct_error: self.direct_error.or(other.direct_error),
            sibling_error: self.sibling_error.or(other.sibling_error),
            direct_panic: self.direct_panic.or(other.direct_panic),
        }
    }

    fn finish(self) -> Result<(), RunWorkload> {
        if let Some(panic) = self.direct_panic {
            resume_unwind(panic);
        }
        match self.direct_error.or(self.sibling_error) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl World {
    #[allow(clippy::type_complexity)]
    pub(super) fn run_batches_direct(
        &self,
        systems: &[Box<dyn Fn(&World) -> Result<(), Run> + Send + Sync + 'static>],
        system_names: &[Box<dyn Label>],
        batches: &Batches,
        #[cfg_attr(not(feature = "tracing"), allow(unused))] workload_name: &dyn Label,
    ) -> Result<(), RunWorkload> {
        if !batches.regroups.is_empty() {
            return self.run_batches_parallel_baseline(systems, system_names, batches, workload_name);
        }
        #[cfg(feature = "tracing")]
        let parent_span = info_span!("workload", name = ?workload_name);
        #[cfg(feature = "tracing")]
        let _parent_span = parent_span.enter();

        // Do not install the entire loop in the local pool: caller-bound steps
        // must stay on the World's owner even when a local pool is configured.
        for batch_index in 0..batches.parallel.len() {
            self.run_one_batch_direct(
                systems, system_names, batches, batch_index,
                #[cfg(feature = "tracing")]
                &parent_span,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    pub(crate) fn run_one_batch_direct(
        &self,
        systems: &[Box<dyn Fn(&World) -> Result<(), Run> + Send + Sync + 'static>],
        system_names: &[Box<dyn Label>],
        batches: &Batches,
        batch_index: usize,
        #[cfg(feature = "tracing")] parent_span: &Span,
    ) -> Result<(), RunWorkload> {
        self.run_one_batch_with(batches, batch_index, |step, run_if| {
            self.run_step(
                systems, system_names, batches,
                #[cfg(feature = "tracing")]
                parent_span,
                step, run_if,
            )
        })
    }

    /// Fence logic with an optional caller-supplied step runner.
    /// The ordinary path supplies the upstream condition/body call, without a
    /// per-system observer or environment lookup.
    pub(crate) fn run_one_batch_with<F>(
        &self,
        batches: &Batches,
        batch_index: usize,
        run_step: F,
    ) -> Result<(), RunWorkload>
    where
        F: Fn(Step, usize) -> Result<(), RunWorkload> + Sync,
    {
        let batch = &batches.parallel[batch_index];
        let batch_run_if = &batches.parallel_run_if[batch_index];
        if batch.0.is_some() || batch.1.iter().any(|step| matches!(step, Step::Regroup(_))) {
            return self.run_one_batch_parallel(batch, batch_run_if, &run_step);
        }

        if batch.1.len() == 1 {
            return run_step(batch.1[0], batch_run_if.1[0]);
        }

        // Drive the iterator on the caller: no scope.spawn wrapper and
        // no per-batch scope latch. Index zero cannot be skipped due to
        // a sibling error, and its error retains baseline precedence.
        let sibling_failed = AtomicBool::new(false);
        let run_parallel = || batch
            .1
            .par_iter()
            .zip(&batch_run_if.1)
            .enumerate()
            .fold(
                BatchOutcome::default,
                |mut outcome, (position, (&step, &run_if))| {
                    if position == 0 {
                        // A caller panic must not discard sibling bodies
                        // merely because Rayon placed them in the same leaf.
                        match catch_unwind(AssertUnwindSafe(|| run_step(step, run_if))) {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => outcome.direct_error = Some(error),
                            Err(panic) => outcome.direct_panic = Some(panic),
                        }
                    } else if !sibling_failed.load(Ordering::Relaxed) {
                        if let Err(error) = run_step(step, run_if) {
                            sibling_failed.store(true, Ordering::Relaxed);
                            outcome.sibling_error = Some(error);
                        }
                    }
                    outcome
                },
            )
            .reduce(BatchOutcome::default, BatchOutcome::merge)
            .finish();
        if let Some(pool) = &self.thread_pool {
            pool.install(run_parallel)
        } else {
            run_parallel()
        }
    }
}

#[cfg(test)]
mod tests;
