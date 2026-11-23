//! Builds the parallel workload DAG at registration.

use crate::all_storages::AllStorages;
use crate::borrow::Mutability;
use crate::scheduler::info::WorkloadInfo;
use crate::scheduler::{Batches, Step, TypeInfo};
use alloc::vec;
use alloc::vec::Vec;
use core::any::TypeId;
use core::iter::once;

/// Private TypeId tag; never exposed as a user-facing scheduling label.
pub(crate) struct Fallible;

pub(crate) struct DagInput {
    pub(crate) system_index: usize,
    pub(crate) hard_after: Vec<usize>,
    pub(crate) borrow_constraints: Vec<TypeInfo>,
    pub(crate) fallible: bool,
}

pub(crate) struct DagNode {
    pub(crate) system_index: usize,
    pub(crate) predecessors: usize,
    pub(crate) successors: Vec<usize>,
}

pub(crate) struct DagGraph {
    pub(crate) nodes: Vec<DagNode>,
    pub(crate) roots: Vec<usize>,
}

pub(crate) enum DagEpoch {
    /// Retains regroup, condition, caller affinity, and failure boundaries.
    Fence(usize),
    Graph(DagGraph),
}

#[derive(Default)]
pub(crate) struct DagPlan {
    pub(crate) epochs: Vec<DagEpoch>,
}

fn step_system(step: Step) -> Option<usize> {
    match step {
        Step::System(index) => Some(index),
        Step::Regroup(_) => None,
    }
}

/// Compile from the final upstream schedule, after regroup preparation and
/// tracking registration succeeded. Conditions contribute their full footprint.
pub(crate) fn finish_registration(
    fallible_by_occurrence: &[bool],
    batches: &mut Batches,
    info: &WorkloadInfo,
) {
    let mut indexed: Vec<_> = info
        .batches_info
        .iter()
        .flat_map(|batch| batch.systems())
        .map(|system| {
            (
                system.occurrence.id,
                DagInput {
                    system_index: system.unique_id,
                    hard_after: system.after.clone(),
                    borrow_constraints: system.occurrence.footprint.clone(),
                    fallible: fallible_by_occurrence[system.occurrence.id],
                },
            )
        })
        .collect();
    indexed.sort_unstable_by_key(|(occurrence, _)| *occurrence);
    let inputs: Vec<_> = indexed
        .into_iter()
        .enumerate()
        .map(|(index, (occurrence, input))| {
            assert_eq!(
                index, occurrence,
                "upstream occurrence metadata must be complete"
            );
            input
        })
        .collect();
    let occurrences: Vec<Vec<_>> = batches
        .parallel_occurrences
        .iter()
        .map(|(owner, parallel)| {
            once(*owner)
                .chain(parallel.iter().copied())
                .filter(|&id| id != usize::MAX)
                .collect()
        })
        .collect();
    batches.dag = DagPlan::build(&inputs, &occurrences, batches)
        .expect("registered workload must have a complete acyclic schedule");
}

impl DagPlan {
    /// Input uses the upstream sorter's final occurrence metadata.
    /// Each integer identifies an occurrence, not a registered function/type.
    pub(crate) fn build(
        inputs: &[DagInput],
        occurrences: &[Vec<usize>],
        batches: &Batches,
    ) -> Option<Self> {
        if occurrences.len() != batches.parallel.len()
            || occurrences.len() != batches.parallel_run_if.len()
        {
            return None;
        }
        let mut batch_of = vec![usize::MAX; inputs.len()];
        for (batch, ids) in occurrences.iter().enumerate() {
            for &id in ids {
                let position = batch_of.get_mut(id)?;
                if *position != usize::MAX {
                    return None;
                }
                *position = batch;
            }
            // Check complete membership including repeated system functions.
            let mut expected: Vec<_> = batches.parallel[batch]
                .0
                .iter()
                .copied()
                .chain(batches.parallel[batch].1.iter().copied())
                .filter_map(step_system)
                .collect();
            let mut actual: Vec<_> = ids.iter().map(|&id| inputs[id].system_index).collect();
            expected.sort_unstable();
            actual.sort_unstable();
            if expected != actual {
                return None;
            }
        }
        if batch_of.contains(&usize::MAX) {
            return None;
        }
        // Hard requirements must have crossed a baseline completion barrier.
        for (id, input) in inputs.iter().enumerate() {
            for &predecessor in &input.hard_after {
                if *batch_of.get(predecessor)? >= batch_of[id] {
                    return None;
                }
            }
        }

        let mut epochs = Vec::new();
        let mut pending = Vec::new();
        for (batch, ids) in occurrences.iter().enumerate() {
            let conditions = &batches.parallel_run_if[batch];
            let is_fence = batches.parallel[batch].0.is_some()
                || batches.parallel[batch].1.iter().any(|step| matches!(step, Step::Regroup(_)))
                || conditions.0 != usize::MAX
                || conditions
                    .1
                    .iter()
                    .any(|&condition| condition != usize::MAX)
                || ids.iter().any(|&id| inputs[id].fallible);
            if is_fence {
                if !pending.is_empty() {
                    epochs.push(DagEpoch::Graph(DagGraph::build(
                        inputs, &pending, &batch_of,
                    )?));
                    pending.clear();
                }
                epochs.push(DagEpoch::Fence(batch));
            } else {
                pending.extend_from_slice(ids);
            }
        }
        if !pending.is_empty() {
            epochs.push(DagEpoch::Graph(DagGraph::build(
                inputs, &pending, &batch_of,
            )?));
        }
        Some(Self { epochs })
    }
}

impl DagGraph {
    fn build(inputs: &[DagInput], occurrences: &[usize], batch_of: &[usize]) -> Option<Self> {
        let mut local = vec![usize::MAX; inputs.len()];
        for (position, &id) in occurrences.iter().enumerate() {
            local[id] = position;
        }
        let mut nodes: Vec<_> = occurrences
            .iter()
            .map(|&id| DagNode {
                system_index: inputs[id].system_index,
                predecessors: 0,
                successors: Vec::new(),
            })
            .collect();
        for (after, &after_id) in occurrences.iter().enumerate() {
            let input = &inputs[after_id];
            // Unlike soft_after, these are execution-order requirements.
            for &predecessor in &input.hard_after {
                let before = *local.get(predecessor)?;
                if before != usize::MAX {
                    if before >= after {
                        return None;
                    }
                    add_edge(&mut nodes, before, after);
                }
            }
            // All conflicting pairs, not just the sorter's diagnostic conflict.
            // Orient by the actual baseline ordering; explicit ordering may have
            // deliberately moved a system relative to its registration order.
            for before in 0..after {
                let before_id = occurrences[before];
                let before_input = &inputs[before_id];
                // Registered functions share a last_run timestamp even when
                // their component views are read-only. Preserve completion
                // between their original batches, but retain the original
                // independence of repeated occurrences within one batch.
                let shared_system_across_batches = before_input.system_index == input.system_index
                    && batch_of.get(before_id)? != batch_of.get(after_id)?;
                if shared_system_across_batches
                    || conflicts(&before_input.borrow_constraints, &input.borrow_constraints)
                {
                    add_edge(&mut nodes, before, after);
                }
            }
        }
        reduce_transitive_edges(&mut nodes);
        let roots = nodes
            .iter()
            .enumerate()
            .filter_map(|(id, node)| (node.predecessors == 0).then_some(id))
            .collect();
        Some(Self { nodes, roots })
    }
}

fn add_edge(nodes: &mut [DagNode], before: usize, after: usize) {
    if !nodes[before].successors.contains(&after) {
        nodes[before].successors.push(after);
        nodes[after].predecessors += 1;
    }
}

/// Nodes are already topologically ordered. Visiting each row's successors in
/// ascending order means an earlier kept successor can prove a later edge
/// redundant. Reachability is computed once here, never during execution.
fn reduce_transitive_edges(nodes: &mut [DagNode]) {
    let words = nodes.len().div_ceil(64);
    let mut reachable = vec![vec![0u64; words]; nodes.len()];
    for from in (0..nodes.len()).rev() {
        nodes[from].successors.sort_unstable();
        nodes[from].successors.retain(|&to| {
            let bit = 1u64 << (to % 64);
            if reachable[from][to / 64] & bit != 0 {
                return false;
            }
            reachable[from][to / 64] |= bit;
            // `to > from`, so its complete reachability row is final.
            for word in 0..words {
                let inherited = reachable[to][word];
                reachable[from][word] |= inherited;
            }
            true
        });
        nodes[from].predecessors = 0;
    }
    for from in 0..nodes.len() {
        for position in 0..nodes[from].successors.len() {
            let to = nodes[from].successors[position];
            nodes[to].predecessors += 1;
        }
    }
}

fn conflicts(left: &[TypeInfo], right: &[TypeInfo]) -> bool {
    left.iter().any(|a| {
        right.iter().any(|b| {
            (!a.thread_safe && !b.thread_safe)
                || (a.storage_id == b.storage_id
                    && (a.mutability == Mutability::Exclusive
                        || b.mutability == Mutability::Exclusive))
                || a.storage_id == TypeId::of::<AllStorages>()
                || b.storage_id == TypeId::of::<AllStorages>()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::World;
    use crate::error::SystemError;
    use crate::scheduler::{SystemModificator, Workload};
    use crate::storage::StorageId;

    #[test]
    fn rejected_registration_rolls_back_fallibility_before_callable_reuse() {
        fn ordinary() {}
        fn fallible() -> Result<(), SystemError> {
            Ok(())
        }

        let world = World::new_for_test();
        assert!(
            Workload::new("rejected")
                .with_system(fallible.require_in_workload("absent"))
                .add_to_world(&world)
                .is_err()
        );
        {
            let Ok(scheduler) = world.scheduler.borrow() else {
                panic!("scheduler borrow");
            };
            assert!(scheduler.systems.is_empty());
            assert!(scheduler.dag_registered_fallible.is_empty());
        }
        assert!(
            Workload::new("ordinary")
                .with_system(ordinary)
                .add_to_world(&world)
                .is_ok()
        );
        assert!(
            Workload::new("fallible")
                .with_system(fallible)
                .add_to_world(&world)
                .is_ok()
        );
        assert!(
            Workload::new("reused")
                .with_system(fallible)
                .add_to_world(&world)
                .is_ok()
        );
        let Ok(scheduler) = world.scheduler.borrow() else {
            panic!("scheduler borrow");
        };
        assert_eq!(scheduler.dag_registered_fallible, vec![false, true]);
        for name in ["fallible", "reused"] {
            let Ok(batches) = scheduler.workload(&name) else {
                panic!("registered workload");
            };
            let plan = &batches.dag;
            assert!(matches!(plan.epochs.as_slice(), [DagEpoch::Fence(0)]));
        }
    }

    fn input(system: usize, hard_after: &[usize], borrow: Vec<TypeInfo>) -> DagInput {
        DagInput {
            system_index: system,
            hard_after: hard_after.to_vec(),
            borrow_constraints: borrow,
            fallible: false,
        }
    }

    fn borrow<T: 'static>(mutability: Mutability) -> TypeInfo {
        TypeInfo {
            name: "test".into(),
            storage_id: StorageId::of::<T>(),
            mutability,
            thread_safe: true,
        }
    }

    #[test]
    fn repeated_system_occurrences_are_distinct_and_all_conflicts_are_edges() {
        let inputs = vec![
            input(5, &[], vec![borrow::<u32>(Mutability::Shared)]),
            input(5, &[], vec![borrow::<u32>(Mutability::Shared)]),
            input(6, &[1], vec![borrow::<u32>(Mutability::Exclusive)]),
        ];
        let Some(graph) = DagGraph::build(&inputs, &[0, 1, 2], &[0, 0, 1]) else {
            panic!("valid graph");
        };


        assert_eq!(graph.nodes[2].predecessors, 2);
        assert_eq!(graph.nodes[0].successors, vec![2]);
        assert_eq!(graph.nodes[1].successors, vec![2]);
    }

    #[test]
    fn repeated_readers_keep_cross_batch_order_without_ordering_batch_siblings() {
        let inputs: Vec<_> = (0..4)
            .map(|_| input(5, &[], vec![borrow::<u32>(Mutability::Shared)]))
            .collect();
        // Also exercise an explicit reordering relative to occurrence IDs.
        // Every reader in the earlier batch must precede every later reader;
        // connecting just the immediately previous occurrence is insufficient.
        for (occurrences, batch_of) in [([0, 1, 2, 3], [0, 0, 1, 1]), ([2, 3, 0, 1], [1, 1, 0, 0])]
        {
            let Some(graph) = DagGraph::build(&inputs, &occurrences, &batch_of) else {
                panic!("valid repeated-reader graph");
            };
            assert_eq!(graph.roots, vec![0, 1]);

            assert_eq!(graph.nodes[0].successors, vec![2, 3]);
            assert_eq!(graph.nodes[1].successors, vec![2, 3]);
            assert_eq!(graph.nodes[2].predecessors, 2);
            assert_eq!(graph.nodes[3].predecessors, 2);
            assert!(graph.nodes[2].successors.is_empty());
            assert!(graph.nodes[3].successors.is_empty());
        }
    }

    #[test]
    fn explicit_reordering_orients_conflicts_in_baseline_order() {
        let inputs = vec![
            input(0, &[1], vec![borrow::<u32>(Mutability::Exclusive)]),
            input(1, &[], vec![borrow::<u32>(Mutability::Exclusive)]),
        ];
        let Some(graph) = DagGraph::build(&inputs, &[1, 0], &[1, 0]) else {
            panic!("valid graph");
        };
        assert_eq!(graph.roots, vec![0]);
        assert_eq!(graph.nodes[0].successors, vec![1]);
        assert!(DagGraph::build(&inputs, &[0, 1], &[1, 0]).is_none());
    }

    #[test]
    fn hard_barriers_are_kept_but_no_edge_is_invented_for_batch_membership() {
        let inputs = vec![
            input(0, &[], vec![]),
            input(1, &[], vec![]),
            input(2, &[0], vec![]),
            input(3, &[0, 1, 2], vec![]),
        ];
        let Some(graph) = DagGraph::build(&inputs, &[0, 1, 2, 3], &[0, 0, 1, 2]) else {
            panic!("valid graph");
        };
        assert_eq!(graph.nodes[2].predecessors, 1);
        assert_eq!(graph.nodes[1].successors, vec![3]);
        assert_eq!(graph.nodes[3].predecessors, 2);

    }

    #[test]
    fn conditional_owner_and_fallible_batches_are_fences() {
        let mut inputs: Vec<_> = (0..5).map(|id| input(id, &[], vec![])).collect();
        inputs[3].fallible = true;
        let batches = Batches {
            parallel: vec![
                (None, vec![Step::System(0)]),
                (None, vec![Step::System(1)]),
                (Some(Step::System(2)), vec![]),
                (None, vec![Step::System(3)]),
                (None, vec![Step::System(4)]),
            ],
            parallel_run_if: vec![
                (usize::MAX, vec![usize::MAX]),
                (usize::MAX, vec![0]),
                (usize::MAX, vec![]),
                (usize::MAX, vec![usize::MAX]),
                (usize::MAX, vec![usize::MAX]),
            ],
            ..Batches::default()
        };
        let Some(plan) = DagPlan::build(
            &inputs,
            &[vec![0], vec![1], vec![2], vec![3], vec![4]],
            &batches,
        ) else {
            panic!("valid plan");
        };
        assert!(matches!(&plan.epochs[0], DagEpoch::Graph(_)));
        assert!(matches!(&plan.epochs[1], DagEpoch::Fence(1)));
        assert!(matches!(&plan.epochs[2], DagEpoch::Fence(2)));
        assert!(matches!(&plan.epochs[3], DagEpoch::Fence(3)));
        assert!(matches!(&plan.epochs[4], DagEpoch::Graph(_)));
    }

    #[test]
    fn transitive_reduction_preserves_every_reachable_pair() {
        fn reach(nodes: &[DagNode]) -> Vec<Vec<bool>> {
            let mut paths = vec![vec![false; nodes.len()]; nodes.len()];
            for from in (0..nodes.len()).rev() {
                for &to in &nodes[from].successors {
                    paths[from][to] = true;
                    for target in 0..nodes.len() {
                        let inherited = paths[to][target];
                        paths[from][target] |= inherited;
                    }
                }
            }
            paths
        }
        let mut nodes: Vec<_> = (0..8)
            .map(|id| DagNode {
                system_index: id,
                predecessors: 0,
                successors: Vec::new(),
            })
            .collect();
        // Dense chain plus a branch/merge, with intentionally redundant edges.
        for (from, to) in [
            (0, 1),
            (0, 2),
            (0, 3),
            (0, 5),
            (0, 7),
            (1, 2),
            (1, 3),
            (1, 5),
            (2, 3),
            (2, 5),
            (3, 5),
            (4, 5),
            (4, 7),
            (5, 6),
            (5, 7),
            (6, 7),
        ] {
            add_edge(&mut nodes, from, to);
        }
        let before = reach(&nodes);
        let before_count: usize = nodes.iter().map(|node| node.successors.len()).sum();
        reduce_transitive_edges(&mut nodes);
        assert_eq!(reach(&nodes), before);
        assert_eq!(before_count, 16);
        assert_eq!(
            nodes
                .iter()
                .map(|node| node.successors.len())
                .sum::<usize>(),
            7
        );
        assert_eq!(nodes.iter().map(|node| node.predecessors).sum::<usize>(), 7);
    }
}
