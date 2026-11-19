# Local scheduler DAG experiment

Base: Shipyard 0.11.9, commit `246579659572b816d7f252f005c578d47667490c`.
`scheduler-experiment` enables `std` and `parallel`; it is off by default.
This prototype supplies `World::run_batches_dag` internally. The experiment
integration selects it with `BATTLE_SCHEDULER_MODE=dag`; the unset mode
remains baseline. Fences and validation fallback reuse `run_one_batch_direct`
from stage 1, so the second comparison isolates DAG scheduling. No production rollout is implied.

## Registration-time graph

The input is captured inside `create_workload` after hard ordering and barriers
have been propagated and before the normal sorter consumes those requirements.
Each node is one workload occurrence, even when multiple occurrences share the
same registered function. The original sorter still produces the baseline
batches and occurrence ordering used to orient conflicting accesses.

Every hard predecessor and every pair of conflicting declared storage borrows
contributes an edge. `soft_after` controls the baseline sequential placement,
but expressly permits parallel execution and is not an execution dependency.
Transitive edges are removed once at registration using forward topological
IDs and reverse-order reachability bitsets. The reduced graph has exactly the
same reachability. Each invocation initializes only atomic predecessor counts;
it does not reconstruct the graph.

Any original batch containing a run condition, required caller slot
(`AllStorages` or non-thread-safe views), or a registered fallible function is
kept as a complete fence. Its conditions still run on the original coordinator,
after the preceding epoch finishes and before any of its batch bodies starts.
Try-system provenance is recorded at first registration, alongside the function
registry, so reusing that function through another workload does not lose it.

Other consecutive batches form a DAG epoch. An epoch uses one Rayon scope;
ready roots are submitted and successful completion releases ready successors
with an AcqRel predecessor countdown. Nested Rayon work finishes as part of the
system call before its successors can become ready. All registered tracing
system/workload labels are retained.

## Failure contract

The existing baseline/direct modes retain their original batch failure contract.
The DAG experiment intentionally cannot promise that an independent later-batch
effect remains unstarted if an unanticipated error or panic occurs: removing
that batch barrier permits precisely such early execution. Known try-system
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
affinity, registered try-function reuse, dependency cancellation, deterministic
error selection, original panic payloads, deliberately allowed early independent
effects, and repeated invocations with nested Rayon work.
