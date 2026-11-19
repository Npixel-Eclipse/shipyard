# Direct batch experiment

This disposable branch starts from Shipyard 0.11.9 `246579659572b816d7f252f005c578d47667490c`.

Enable `scheduler-experiment` (which enables `std` and `parallel`). Select the
mode before starting the process:

- Unset `BATTLE_SCHEDULER_MODE`, or `baseline`: the original scheduler body.
- `BATTLE_SCHEDULER_MODE=direct`: ordinary parallel batches are driven by the
  caller's parallel iterator, without the outer `scope.spawn` wrapper and its
  batch completion scope. Singleton batches call the system directly.
- Any other explicit value fails rather than silently measuring the baseline.

The selection is cached once per process and checked once per workload. When
`BATTLE_SCHEDULER_MODE_OUTPUT` is present, initialization writes one JSON line
such as `{"mode":"direct"}` to that path. An invalid path or failed write aborts
the experiment. The harness must create the parent directory and verify this
file independently of its requested mode.

The required caller-thread slot (`batch.0`, used for AllStorages or non-thread-safe
borrows) retains the original scope implementation, including its filtered
fallback to the first ordinary system. The optional world-owned pool is retained.

Normal batches retain serial `run_if` evaluation before dispatch, exactly-once
eligible system invocation, the full barrier between batches, nested Rayon work,
and existing `run_single_system` tracing names, parent, and lifetime. Error handling
must preserve the original first caller body's priority even when a sibling fails
first. Therefore the direct iterator uses a fold/reduce result with a separate
caller error and cooperative cancellation for sibling errors. A caller panic is
retained until siblings join, so putting them in the same iterator leaf does not
accidentally cancel otherwise successful siblings. Sibling error identity and
execution order were already nondeterministic in the parallel baseline.

`run_one_batch_direct` is crate-visible for the separate DAG experiment's
semantic fences. It accepts the full `Batches`, a zero-based batch index, and
the caller's workload span when tracing is enabled. It does not open another
workload span or select a mode.

The performance comparison must include all costs of this implementation,
including fold/reduce, cancellation checks, and panic handling. No speedup is
assumed from removing the scope.

## Validation prepared

Nine focused unit tests cover mode validation, execution counts, nested Rayon,
batch barriers, ordered/filtered/failing `run_if`, caller error precedence,
caller panic completion, caller-thread affinity, and tracing parent/name/close.
They run as part of the library with `scheduler-experiment,tracing` enabled.

At authoring time, only rustfmt and `git diff --check` were run. Compilation and
test execution are deliberately deferred to the parent task's exclusive build
slot because another user session is building. The parent will validate the
combined direct/DAG clone and record the actual results.
