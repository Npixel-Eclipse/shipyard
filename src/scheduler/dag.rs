//! Registration-time DAG for the explicitly enabled local scheduler experiment.

use crate::all_storages::AllStorages;
use crate::borrow::Mutability;
use crate::scheduler::Label;
use crate::scheduler::{Batches, TypeInfo};
use alloc::boxed::Box;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use core::any::TypeId;
use core::sync::atomic::{AtomicU64, Ordering};
use serde_json::{json, to_writer_pretty};
use std::env::var_os;
use std::error::Error;
use std::fs::{create_dir_all, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::process::id as process_id;

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
    pub(crate) occurrence: usize,
}

pub(crate) struct DagGraph {
    pub(crate) nodes: Vec<DagNode>,
    pub(crate) roots: Vec<usize>,
    pub(crate) raw_edge_count: usize,
}

pub(crate) enum DagEpoch {
    /// Retains original condition evaluation, caller affinity, and failure boundary.
    Fence(usize),
    Graph(DagGraph),
}

pub(crate) struct DagPlan {
    pub(crate) epochs: Vec<DagEpoch>,
}

impl DagPlan {
    /// Snapshot input is captured before the normal sorter consumes hard_after.
    /// Each integer identifies an occurrence, not a registered function/type.
    /// An unexpected mapping fails closed to the original batch executor.
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
                || conditions.0 != usize::MAX
                || conditions
                    .1
                    .iter()
                    .any(|&condition| condition != usize::MAX)
                || ids.iter().any(|&id| inputs[id].fallible);
            if is_fence {
                if !pending.is_empty() {
                    epochs.push(DagEpoch::Graph(DagGraph::build(inputs, &pending)?));
                    pending.clear();
                }
                epochs.push(DagEpoch::Fence(batch));
            } else {
                pending.extend_from_slice(ids);
            }
        }
        if !pending.is_empty() {
            epochs.push(DagEpoch::Graph(DagGraph::build(inputs, &pending)?));
        }
        Some(Self { epochs })
    }
}

impl DagGraph {
    fn build(inputs: &[DagInput], occurrences: &[usize]) -> Option<Self> {
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
                occurrence: id,
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
                if conflicts(
                    &inputs[occurrences[before]].borrow_constraints,
                    &input.borrow_constraints,
                ) {
                    add_edge(&mut nodes, before, after);
                }
            }
        }
        let raw_edge_count = nodes.iter().map(|node| node.successors.len()).sum();
        reduce_transitive_edges(&mut nodes);
        let roots = nodes
            .iter()
            .enumerate()
            .filter_map(|(id, node)| (node.predecessors == 0).then_some(id))
            .collect();
        Some(Self {
            nodes,
            roots,
            raw_edge_count,
        })
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

/// Optional registration-only evidence. Neither mode selection nor ticks perform IO.
pub(crate) fn write_plan_output(
    workload_name: &dyn Label,
    names: &[Box<dyn Label>],
    inputs: &[DagInput],
    occurrences: &[Vec<usize>],
    batches: &Batches,
) {
    let Some(directory) = var_os("BATTLE_SCHEDULER_PLAN_OUTPUT_DIR") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let write = || -> Result<(), Box<dyn Error>> {
        create_dir_all(&directory)?;
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let file = directory.join(format!(
            "scheduler-plan-{}-{sequence:04}.json",
            process_id()
        ));
        let mut graph_nodes = 0;
        let mut edges = 0;
        let mut raw_edges = 0;
        let mut fence_count = 0;
        let mut fence_nodes = 0;
        let mut graph_epochs = 0;
        let epochs: Vec<_> = batches.dag.as_ref().map_or(&[][..], |plan| &plan.epochs)
            .iter().enumerate().map(|(index, epoch)| match epoch {
                DagEpoch::Fence(batch) => {
                    fence_count += 1;
                    fence_nodes += occurrences[*batch].len();
                    json!({"index": index, "kind": "fence", "baseline_batch": batch,
                        "occurrences": occurrences[*batch]})
                }
                DagEpoch::Graph(graph) => {
                    graph_epochs += 1;
                    graph_nodes += graph.nodes.len();
                    edges += graph.nodes.iter().map(|node| node.successors.len()).sum::<usize>();
                    raw_edges += graph.raw_edge_count;
                    let nodes: Vec<_> = graph.nodes.iter().enumerate().map(|(local, node)| {
                        json!({"local": local, "occurrence": node.occurrence,
                            "system_index": node.system_index, "name": format!("{:?}", names[node.system_index]),
                            "predecessor_count": node.predecessors, "successors": node.successors})
                    }).collect();
                    json!({"index": index, "kind": "graph", "roots": graph.roots, "nodes": nodes,
                        "raw_edge_count": graph.raw_edge_count,
                        "reduced_edge_count": graph.nodes.iter().map(|node| node.successors.len()).sum::<usize>()})
                }
            }).collect();
        let baseline: Vec<_> = occurrences.iter().enumerate().map(|(index, ids)| {
            let condition = &batches.parallel_run_if[index];
            json!({"index": index, "occurrences": ids, "size": ids.len(),
                "required_caller_system": batches.parallel[index].0,
                "has_run_if": condition.0 != usize::MAX || condition.1.iter().any(|&id| id != usize::MAX),
                "has_registered_fallible": ids.iter().any(|&id| inputs[id].fallible)})
        }).collect();
        let input_nodes: Vec<_> = inputs.iter().enumerate().map(|(occurrence, input)| {
            let borrow: Vec<_> = input.borrow_constraints.iter().map(|constraint| json!({
                "name": constraint.name, "storage": format!("{:?}", constraint.storage_id),
                "mutability": format!("{:?}", constraint.mutability), "thread_safe": constraint.thread_safe,
            })).collect();
            json!({"occurrence": occurrence, "system_index": input.system_index,
                "name": format!("{:?}", names[input.system_index]),
                "hard_after": input.hard_after, "borrow": borrow, "registered_fallible": input.fallible})
        }).collect();
        let output = json!({"schema": 1, "workload_name": format!("{:?}", workload_name),
            "plan_valid": batches.dag.is_some(),
            "fallback_reason": if batches.dag.is_some() { None } else {
                Some("Occurrence membership, hard ordering, or forward-edge validation failed; original batch execution retained") },
            "node_count": inputs.len(), "ordinary_count": graph_nodes, "graph_node_count": graph_nodes,
            "fence_count": fence_count, "fence_node_count": fence_nodes,
            "epoch_count": epochs.len(), "graph_epoch_count": graph_epochs, "edge_count": edges,
            "raw_edge_count": raw_edges, "reduced_edge_count": edges,
            "baseline_batch_count": batches.parallel.len(), "baseline_batches": baseline,
            "input_nodes": input_nodes, "epochs": epochs,
            "contract": "local opt-in experiment; known-fallible/run_if/owner batches remain fenced; unexpected errors or panics may follow already admitted independent effects"});
        let mut writer = BufWriter::new(File::create(file)?);
        to_writer_pretty(&mut writer, &output)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    };
    if let Err(error) = write() {
        panic!("requested scheduler plan evidence write failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageId;

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
        let Some(graph) = DagGraph::build(&inputs, &[0, 1, 2]) else {
            panic!("valid graph");
        };
        assert_eq!(graph.nodes[0].occurrence, 0);
        assert_eq!(graph.nodes[1].occurrence, 1);
        assert_eq!(graph.nodes[2].predecessors, 2);
        assert_eq!(graph.nodes[0].successors, vec![2]);
        assert_eq!(graph.nodes[1].successors, vec![2]);
    }

    #[test]
    fn explicit_reordering_orients_conflicts_in_baseline_order() {
        let inputs = vec![
            input(0, &[1], vec![borrow::<u32>(Mutability::Exclusive)]),
            input(1, &[], vec![borrow::<u32>(Mutability::Exclusive)]),
        ];
        let Some(graph) = DagGraph::build(&inputs, &[1, 0]) else {
            panic!("valid graph");
        };
        assert_eq!(graph.roots, vec![0]);
        assert_eq!(graph.nodes[0].successors, vec![1]);
        assert!(DagGraph::build(&inputs, &[0, 1]).is_none());
    }

    #[test]
    fn hard_barriers_are_kept_but_no_edge_is_invented_for_batch_membership() {
        let inputs = vec![
            input(0, &[], vec![]),
            input(1, &[], vec![]),
            input(2, &[0], vec![]),
            input(3, &[0, 1, 2], vec![]),
        ];
        let Some(graph) = DagGraph::build(&inputs, &[0, 1, 2, 3]) else {
            panic!("valid graph");
        };
        assert_eq!(graph.nodes[2].predecessors, 1);
        assert_eq!(graph.nodes[1].successors, vec![3]);
        assert_eq!(graph.nodes[3].predecessors, 2);
        assert_eq!(graph.raw_edge_count, 4);
    }

    #[test]
    fn conditional_owner_and_fallible_batches_are_fences() {
        let mut inputs: Vec<_> = (0..5).map(|id| input(id, &[], vec![])).collect();
        inputs[3].fallible = true;
        let batches = Batches {
            parallel: vec![
                (None, vec![0]),
                (None, vec![1]),
                (Some(2), vec![]),
                (None, vec![3]),
                (None, vec![4]),
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
                occurrence: id,
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
