use crate::scheduler::Batches;
use crate::scheduler::dag::DagEpoch;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::iter::once;

#[doc(hidden)]
pub struct WorkloadExecutionOrder {
    epoch_by_occurrence: Vec<usize>,
    successors: Vec<Vec<usize>>,
}

impl WorkloadExecutionOrder {
    pub(crate) fn from_batches(batches: &Batches) -> Self {
        let count = batches
            .sequential_occurrences
            .iter()
            .filter(|&&id| id != usize::MAX)
            .count();
        let mut order = Self {
            epoch_by_occurrence: vec![usize::MAX; count],
            successors: vec![Vec::new(); count],
        };
        for (epoch, entry) in batches.dag.epochs.iter().enumerate() {
            match entry {
                DagEpoch::Fence(batch) => {
                    let (caller, workers) = &batches.parallel_occurrences[*batch];
                    for id in once(*caller)
                        .chain(workers.iter().copied())
                        .filter(|&id| id != usize::MAX)
                    {
                        order.epoch_by_occurrence[id] = epoch;
                    }
                }
                DagEpoch::Graph(graph) => {
                    for (node, &id) in graph.nodes.iter().zip(&graph.occurrences) {
                        order.epoch_by_occurrence[id] = epoch;
                        order.successors[id] = node
                            .successors
                            .iter()
                            .map(|&next| graph.occurrences[next])
                            .collect();
                    }
                }
            }
        }
        assert!(!order.epoch_by_occurrence.contains(&usize::MAX));
        order
    }

    #[doc(hidden)]
    pub fn happens_before(&self, before_occurrence: usize, after_occurrence: usize) -> bool {
        let (Some(before), Some(after)) = (
            self.epoch_by_occurrence.get(before_occurrence),
            self.epoch_by_occurrence.get(after_occurrence),
        ) else {
            return false;
        };
        match before.cmp(after) {
            Ordering::Less => true,
            Ordering::Greater => false,
            Ordering::Equal => {
                let mut visited = vec![false; self.successors.len()];
                let mut pending = self.successors[before_occurrence].clone();
                while let Some(next) = pending.pop() {
                    if next == after_occurrence {
                        return true;
                    }
                    if !visited[next] {
                        visited[next] = true;
                        pending.extend_from_slice(&self.successors[next]);
                    }
                }
                false
            }
        }
    }
}
