# Direct batch experiment

This opt-in experiment is ported to the `custom-features`
`2fb4fddbbb947d2ce0ea6f1bab59edb960841739` execution contract.

Enable `scheduler-experiment` (which enables `std` and `parallel`). Select the
mode before starting the process:

- Unset `BATTLE_SCHEDULER_MODE`, or `baseline`: the original scheduler body.
- `BATTLE_SCHEDULER_MODE=direct`: ordinary parallel batches are driven by the
  parallel iterator, without the outer `scope.spawn` wrapper and its batch
  completion scope. Singleton batches call the step directly. A configured
  local pool executes the ordinary parallel iterator.
- `BATTLE_SCHEDULER_MODE=dag`: stage-1 direct fences plus the opt-in DAG
  executor described in `SCHEDULER_DAG_EXPERIMENT.md`.
- Any other explicit value fails rather than silently measuring the baseline.

The selection is cached once per process and checked once per workload. When
`BATTLE_SCHEDULER_MODE_OUTPUT` is present, initialization writes one JSON line
such as `{"mode":"direct"}` to that path. An invalid path or failed write aborts
the experiment. The harness must create the parent directory and verify this
file independently of its requested mode.

The required caller-thread slot (`batch.0`, used for AllStorages or non-thread-safe
borrows) retains the upstream scope implementation and runs on the original
calling thread, including its condition. A false condition skips that slot;
it does not promote a parallel sibling to the calling thread. The whole batch
loop is never installed inside the optional local pool.

Each `run_if` is evaluated immediately before its own body, on that step's
execution thread, as in upstream. Conditions are not evaluated as a serial
prepass. A failing condition skips its body and stops later batches, while
already admitted siblings may complete. Normal batches retain exactly-once
eligible system invocation, the full barrier between batches, nested Rayon work,
and existing `run_single_system` tracing names, parent, and lifetime. Error handling
must preserve the original first caller body's priority even when a sibling fails
first. Therefore the direct iterator uses a fold/reduce result with a separate
caller error and cooperative cancellation for sibling errors. A caller panic is
retained until siblings join, so putting them in the same iterator leaf does not
accidentally cancel otherwise successful siblings. Sibling error identity and
execution order were already nondeterministic in the parallel baseline.

Workloads with regroup steps use the upstream baseline executor in both direct
and DAG modes. An unavailable DAG plan also uses that executor. This preserves
regroup borrows, errors, and caller-thread requirements without inventing graph
dependencies for grouping. The selected-mode JSON alone does not prove a DAG
graph was executed; inspect the registered plan's validity and epochs too.

`run_one_batch_direct` is crate-visible for the separate DAG experiment's
semantic fences. It accepts the full `Batches`, a zero-based batch index, and
the caller's workload span when tracing is enabled. It does not open another
workload span or select a mode.

The performance comparison must include all costs of this implementation,
including fold/reduce, cancellation checks, and panic handling. No speedup is
assumed from removing the scope.

## Validation prepared

Ten focused unit tests cover mode validation, execution counts, nested Rayon,
batch barriers, per-step/filtered/failing `run_if`, caller error precedence,
caller panic completion, local-pool caller/condition affinity, regroup fallback
and borrow errors, and tracing parent/name/close.
They run as part of the library with `scheduler-experiment,tracing` enabled.

This port has been checked by source review and `git diff --check` only.
Compilation and test execution are deferred to the parent task's exclusive
build slot. The parent will record the actual combined direct/DAG results.
