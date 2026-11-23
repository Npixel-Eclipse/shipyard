# Tracking

Shipyard comes with built-in tracking for *insertion*, *modification*, *deletion* and *removal*.

## Declaration

Tracking is set with the `Component` trait. You can select individual operations or use `All` to track everything.

```rs
{{#include ../../../../tests/book/tracking.rs:component}}
{{#include ../../../../tests/book/tracking.rs:component_proc}}
```

## Usage

When inside a workload you will get information about operations that happened since the last time the current system ran.\
Outside workloads you'll get information since the last call to `clear_*`.

#### *Inserted* or *Modified*

You can iterate *inserted* and *modified* components by calling `inserted`, `modified` or `inserted_or_modified` on a view before making the iterator. (`*_mut` versions also exist).

```rs
{{#include ../../../../tests/book/tracking.rs:run}}
```

#### *Removed* or *Deleted*

The difference between `remove` and `delete` can be understood from their signature:

```rs
fn delete(&mut self, entity: EntityId) -> bool {}
fn remove(&mut self, entity: EntityId) -> Option<T> {}
```

*deletion* stores the component whereas *removal* gives it back immediately.\
Components can be deleted or removed but whole entities can only be deleted (at least for now).

*Removed* and *deleted* components cannot be accessed with `iter` but with `removed`, `deleted` or `removed_or_deleted` methods.

## Reset

Inside workloads tracking information is automatically reset.\
You will always get the operations that happened since the last run of the system.

You can reset tracking information outside of workload with:
- `clear_all_inserted`
- `clear_all_modified`
- `clear_all_inserted_and_modified`
- `clear_all_removed`
- `clear_all_deleted`
- `clear_all_removed_and_deleted`

You can also reset removed and deleted information older than some timestamp.

Use `World::get_tracking_timestamp` or `AllStorages::get_tracking_timestamp` to get a timestamp.\
Then call `clear_all_deleted_older_than_timestamp`, `clear_all_removed_older_than_timestamp` or `clear_all_removed_and_deleted_older_than_timestamp`.
