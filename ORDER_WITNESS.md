# Local order evidence

This measurement-only extension requires `scheduler-experiment`. It does not
select itself for any ordinary run. Builds and executions are owned by the
parent measurement runner and must remain serialized with other sessions.

Set `BATTLE_SCHEDULER_MODE=dag` and
`BATTLE_ORDER_WITNESS_FILE=<absolute manifest path>` before starting Battle.
The manifest is:

```json
{
  "schema": 1,
  "source_plan": "E:/.../plans/scheduler-plan-13552-0000.json",
  "witness_id": "normal-reverse-1",
  "occurrences": [0, 1, 4, 2],
  "output": "E:/.../witness.jsonl"
}
```

The occurrence array above is abbreviated: supply the complete array from
`order-witnesses.json` (`plans.normal.reference_order` or one
`plans.normal.reverse_witnesses[i].occurrences`, and the analogous profiled
source). Create the output parent directory beforehand. The output file must
not exist. There is no resume, overwrite, randomization or topology fallback.

Only the Battle main workload, identified by its first registered system,
can be selected. Registration validates every current occurrence name,
borrow storage name/mutability/thread affinity, hard ordering constraint,
baseline batch, required caller occurrence, condition/fallibility boundary,
graph node, indegree, root and reduced edge against the source plan. Rust
TypeId strings and global registry indices are not used as identities. The
requested order must contain every occurrence once, preserve fence positions,
and respect every graph edge. Other workloads retain their normal DAG path.
A requested witness that has not bound to a main workload fails before DAG
execution. Registration controls such as `no_graph` are incompatible.

Each graph epoch executes serially in the requested topological order. Fence
batches use the same condition evaluation, caller affinity, parallel body
execution, joining and error selection as the direct executor. Unexpected
errors/panics stop the serial epoch; this witness is not a performance mode.
No spin-wait or blocking dependency gates are introduced.

The JSONL starts with registration metadata, the manifest and current names.
Each actually executed main workload emits a numbered tick with `status` and
`events`. Each event is `[code, occurrence]`: 0 body start, 1 successful body
return, 2 error return, 3 panic. The sequence records actual invocation
boundaries, including concurrent fence bodies. A false system condition has
no body events. Evidence is flushed after each completed/error/panic tick.
Fatal process termination can leave the last tick absent; it is never evidence
of successful execution. The witness tick is an invocation index, not an
assumed game tick or elapsed-time measurement.

## Tracking callback

`shipyard::tracking_observation::set_observer(fn(TrackingHit))` registers one
process-wide callback before workloads run. It is independent of witness
selection, so native A/A and ordinary DAG runs can use the same observation.
`TrackingHit` has `system`, `component`, `kind` (`&'static str`), `entity`
(`EntityId::inner()` as `u64`) and `current` (`u64` tracking timestamp).
Battle owns tick association, semantic entity mapping, buffering and output.
The callback must not panic, block, or reenter Shipyard.

Covered paths:

- View/ViewMut direct positive `is_inserted`, `is_modified`, `is_deleted`,
  `is_removed`, and their union queries.
- Positive Inserted/Modified/InsertedOrModified iterator timestamp predicates,
  including read/write views and nested Rayon workers. Such a candidate can
  still fail a later tuple join/filter. These are not final loop-body counts.
- View/ViewMut deleted/removed/removed-or-deleted iterator return points.

Union iterator filters retain `inserted_or_modified` or `removed_or_deleted`
as their kind; no extra query is run to split them. System-start timestamps
map worker observations back to their workload function. Registration uses
the function type name, which can differ from a custom display label.
Non-workload borrows, independently borrowed nested World/AllStorages queries
with another timestamp, negated filters, Unique tracking, and direct advanced
tracking calls are outside coverage. Only one observed World may execute at a
time; overlapping per-World timestamp identities fail explicitly.

When no callback is registered, no map allocation, locking, or output occurs.
When no witness manifest is configured, no witness output or serial schedule
is created. The existing experiment feature contains all observation hooks.
