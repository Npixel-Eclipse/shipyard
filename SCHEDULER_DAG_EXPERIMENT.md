# Local scheduler DAG experiment

Release: Shipyard 0.11.9. The scheduler/iterator/proc-macro base is
`origin/custom-features` commit `2fb4fddbbb947d2ce0ea6f1bab59edb960841739`,
merged with the local experiments through `4b8be985aee669947b8a38124e33017db3876f51`.
`scheduler-experiment` enables `std` and `parallel`; it is off by default.
This prototype supplies `World::run_batches_dag` internally. The experiment
integration selects it with `BATTLE_SCHEDULER_MODE=dag`; the unset mode
remains baseline. Fences retain upstream step/condition execution. Workloads
containing regroup steps and validation failures use the upstream baseline
executor. No production rollout is implied.

## Registration-time graph

The input is captured from final upstream `WorkloadInfo` after scheduling,
regroup preparation and tracking registration have succeeded. Occurrence IDs,
predecessors and complete system/condition borrow footprints come from that
metadata; the experimental compiler does not replace upstream scheduling.
Each node is one workload occurrence, even when multiple occurrences share the
same registered function. The original sorter still produces the baseline
batches and occurrence ordering used to orient conflicting accesses.

Every hard predecessor and every pair of conflicting declared storage borrows
contributes an edge. `order_after` controls the baseline sequential placement,
but expressly permits parallel execution and is not an execution dependency.
Occurrences sharing a registered callable also retain completion ordering
between distinct baseline batches because they share the tracking `last_run`.
Repeated occurrences already within one batch remain independent.
Transitive edges are removed once at registration using forward topological
IDs and reverse-order reachability bitsets. The reduced graph has exactly the
same reachability. Each invocation initializes only atomic predecessor counts;
it does not reconstruct the graph.

Any original batch containing a run condition, required caller slot
(`AllStorages` or non-thread-safe views), or a registered fallible function is
kept as a complete fence. Each condition runs immediately before its own body,
at the upstream-selected execution location, after the preceding epoch ends.
The caller-bound slot stays on the calling thread even with a local Rayon pool.
Native `Result<(), SystemError>` provenance is recorded at first registration,
alongside the function registry. Reuse retains it and failed registration rolls
back this metadata together with the registered callable. The std-only legacy
`IntoWorkloadTrySystem` / `with_try_system` compatibility API marks its callable
the same way. No-std retains upstream's native `SystemError` API; the old
arbitrary-`Any` no-std error API is not restored.

Other consecutive batches form a DAG epoch. An epoch uses one Rayon scope;
ready roots are submitted and successful completion releases ready successors
with an AcqRel predecessor countdown. Nested Rayon work finishes as part of the
system call before its successors can become ready. All registered tracing
system/workload labels are retained.

## Failure contract

The existing baseline/direct modes retain their original batch failure contract.
The DAG experiment intentionally cannot promise that an independent later-batch
effect remains unstarted if an unanticipated error or panic occurs: removing
that batch barrier permits precisely such early execution. Known fallible-system
batches are fenced, but dynamic borrow failures and arbitrary panics remain
possible in ordinary systems.

An observed failure atomically cancels future admissions. A node admitted before
cancellation is already in flight and can have run. A failed node never releases
successors. The Rayon scope joins all admitted work; if more than one admitted
system returns an error, the lowest baseline node position among those errors
is returned. A panic sets cancellation and resumes its original payload through
Rayon's normal scope unwinding. Error return does not override a panic.

This contract difference is tested explicitly and must accompany benchmark
results. It is not a production-ready replacement for the baseline scheduler.

## Measurement evidence

Setting an absolute `BATTLE_SCHEDULER_PLAN_OUTPUT_DIR` writes one JSON file per
workload registration: `scheduler-plan-<pid>-<ordinal>.json`. There is no per-tick
I/O. An explicitly requested output failure aborts registration rather than
silently producing a measurement without evidence.

Schema 1 includes `plan_valid`, `fallback_reason`, `workload_name`, node/fence/
epoch counts, raw/reduced edge counts, baseline batch membership, complete named
input occurrences and hard predecessors, and each reduced epoch's adjacency.
A failed graph validation retains original batch execution and is visible as
`plan_valid: false`. A valid plan can still have only fences; benchmark validation
must inspect actual graph node and epoch sizes, not merely the selected mode.

Focused unit tests cover pairwise conflicts, repeated occurrence identity,
explicit ordering/barriers, reachability reduction, condition timing, caller
affinity, registered fallible-function reuse, dependency cancellation, deterministic
error selection, original panic payloads, deliberately allowed early independent
effects, and repeated invocations with nested Rayon work.

The registration timing schema is retained. `create_workload_ns` now covers
upstream registration/order construction; DAG input capture and construction
happen afterwards, within `add_to_world_ns`. Regroup preparation and tracking
enablement are included in the latter total. These nested phase timings must
not be compared to the older compiler phase boundaries without adjustment.
