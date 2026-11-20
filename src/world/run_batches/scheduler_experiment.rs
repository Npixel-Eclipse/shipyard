//! Disposable scheduling experiment. The ordinary scheduler remains the default.

use crate::error::{Run, RunWorkload};
use crate::scheduler::{Batches, Label};
#[cfg(feature = "scheduler-registration-benchmark")]
use crate::scheduler::registration_benchmark::validate_execution_mode;
use crate::World;
use alloc::boxed::Box;
use alloc::format;
use alloc::vec::Vec;
use core::any::Any;
use rayon::in_place_scope;
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
        #[cfg(feature = "tracing")]
        let parent_span = info_span!("workload", name = ?workload_name);
        #[cfg(feature = "tracing")]
        let _parent_span = parent_span.enter();

        let run_batch = || -> Result<(), RunWorkload> {
            for batch_index in 0..batches.parallel.len() {
                self.run_one_batch_direct(
                    systems,
                    system_names,
                    batches,
                    batch_index,
                    #[cfg(feature = "tracing")]
                    &parent_span,
                )?;
            }
            Ok(())
        };

        if let Some(thread_pool) = &self.thread_pool {
            thread_pool.scope(|_| run_batch())
        } else {
            run_batch()
        }
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
        self.run_one_batch_with(system_names, batches, batch_index, |index| {
            self.run_single_system(
                systems,
                system_names,
                #[cfg(feature = "tracing")]
                parent_span,
                index,
            )
        })
    }

    /// Identical fence logic with an optional caller-supplied body observer.
    /// The ordinary path supplies its original call directly, without a
    /// per-system observer or environment lookup.
    pub(crate) fn run_one_batch_with<F>(
        &self,
        system_names: &[Box<dyn Label>],
        batches: &Batches,
        batch_index: usize,
        run_system: F,
    ) -> Result<(), RunWorkload>
    where
        F: Fn(usize) -> Result<(), RunWorkload> + Sync,
    {
        let batch = &batches.parallel[batch_index];
        let batch_run_if = &batches.parallel_run_if[batch_index];
        // Preserve serial run_if evaluation, including failures before any
        // body in this batch can be dispatched.
        let run_if = (
            if batch_run_if.0 == usize::MAX {
                true
            } else {
                (batches.systems_run_if[batch_run_if.0])(self).map_err(|err| {
                    let index = match batch.0 {
                        Some(index) => index,
                        None => unreachable!("run_if requires a caller-thread system"),
                    };
                    RunWorkload::Run((system_names[index].clone(), err))
                })?
            },
            batch
                .1
                .iter()
                .zip(&batch_run_if.1)
                .map(|(&index, &run_if_index)| {
                    if run_if_index == usize::MAX {
                        Ok(true)
                    } else {
                        (batches.systems_run_if[run_if_index])(self)
                            .map_err(|err| RunWorkload::Run((system_names[index].clone(), err)))
                    }
                })
                .collect::<Result<Vec<_>, RunWorkload>>()?,
        );

        if batch.0.is_some() {
            // Required caller-thread slots retain the original scope,
            // wrapper, fallback, and error precedence.
            let mut result = Ok(());
            let mut start = 0;
            let single_system = batch.0.filter(|_| run_if.0).or_else(|| {
                let system = batch.1.first().copied().filter(|_| run_if.1[0]);
                if system.is_some() {
                    start = 1;
                }
                system
            });
            let scope_result: Result<(), RunWorkload> = in_place_scope(|scope| {
                if start < batch.1.len() {
                    scope.spawn(|_| {
                        result = batch.1[start..]
                            .par_iter()
                            .zip(&run_if.1[start..])
                            .try_for_each(|(&index, should_run)| {
                                if *should_run {
                                    run_system(index)
                                } else {
                                    Ok(())
                                }
                            });
                    });
                }
                if let Some(index) = single_system {
                    run_system(index)?;
                }
                Ok(())
            });
            scope_result?;
            result?;
            return Ok(());
        }

        if batch.1.len() == 1 {
            if run_if.1[0] {
                run_system(batch.1[0])?;
            }
            return Ok(());
        }

        // Drive the iterator on the caller: no scope.spawn wrapper and
        // no per-batch scope latch. Index zero cannot be skipped due to
        // a sibling error, and its error retains baseline precedence.
        let sibling_failed = AtomicBool::new(false);
        batch
            .1
            .par_iter()
            .zip(&run_if.1)
            .enumerate()
            .fold(
                BatchOutcome::default,
                |mut outcome, (position, (&index, eligible))| {
                    if !eligible {
                        return outcome;
                    }
                    if position == 0 {
                        // A caller panic must not discard sibling bodies
                        // merely because Rayon placed them in the same leaf.
                        match catch_unwind(AssertUnwindSafe(|| run_system(index))) {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => outcome.direct_error = Some(error),
                            Err(panic) => outcome.direct_panic = Some(panic),
                        }
                    } else if !sibling_failed.load(Ordering::Relaxed) {
                        if let Err(error) = run_system(index) {
                            sibling_failed.store(true, Ordering::Relaxed);
                            outcome.sibling_error = Some(error);
                        }
                    }
                    outcome
                },
            )
            .reduce(BatchOutcome::default, BatchOutcome::merge)
            .finish()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
