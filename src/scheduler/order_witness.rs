//! Explicit offline order witnesses. Never selected by the ordinary modes.
use super::dag::{DagEpoch, DagInput};
use super::{Batches, Label};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use serde_json::{from_slice, json, to_writer, Value};
use std::env::var_os;
use std::ffi::OsStr;
use std::fs::{read, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const MAIN_FIRST_SYSTEM: &str = "System(battle_core::content::cheat::cast_range_mode::apply_cast_range_mode_changes)";
static BOUND: AtomicBool = AtomicBool::new(false);

struct Request {
    manifest: Value,
    source: Value,
    order: Vec<usize>,
    output: PathBuf,
    id: String,
}

pub(crate) struct OrderWitness {
    /// Graph local IDs; fence entries are empty and use their original batch.
    pub(crate) epoch_orders: Vec<Vec<usize>>,
    pub(crate) batch_occurrences: Vec<Vec<usize>>,
    pub(crate) occurrence_systems: Vec<usize>,
    id: String,
    output: Mutex<BufWriter<File>>,
    ticks: AtomicU64,
    running: AtomicBool,
}

pub(crate) struct WitnessTick<'a> {
    owner: &'a OrderWitness,
    pub(crate) tick: u64,
}

impl Drop for WitnessTick<'_> {
    fn drop(&mut self) {
        self.owner.running.store(false, Ordering::Release);
    }
}

impl OrderWitness {
    pub(crate) fn begin(&self) -> WitnessTick<'_> {
        assert!(self.running.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_ok(),
            "order witness main workload may not be run concurrently");
        WitnessTick { owner: self, tick: self.ticks.fetch_add(1, Ordering::Relaxed) + 1 }
    }

    pub(crate) fn record_tick(&self, tick: u64, status: &str, events: &[[usize; 2]]) {
        // Event [0, occurrence] precedes the body, [1, occurrence] follows its
        // success; 2/3 mark error/panic. Skipped run_if bodies have no events.
        self.write(&json!({"schema": 1, "kind": "tick", "witness_id": self.id,
            "tick": tick, "status": status, "events": events}));
    }

    fn write(&self, value: &Value) {
        let mut writer = self.output.lock().unwrap_or_else(|p| p.into_inner());
        to_writer(&mut *writer, value).expect("cannot write order witness evidence");
        writer.write_all(b"\n").expect("cannot write order witness newline");
        writer.flush().expect("cannot flush order witness evidence");
    }
}

fn array_ids(value: &Value) -> Vec<usize> {
    value.as_array().expect("witness order must be an array").iter().map(|id| {
        usize::try_from(id.as_u64().expect("witness IDs must be nonnegative integers"))
            .expect("witness ID does not fit usize")
    }).collect()
}

fn request() -> Option<&'static Request> {
    static REQUEST: OnceLock<Option<Request>> = OnceLock::new();
    REQUEST.get_or_init(|| {
        let path = var_os("BATTLE_ORDER_WITNESS_FILE")?;
        assert_eq!(var_os("BATTLE_SCHEDULER_MODE").as_deref(), Some(OsStr::new("dag")),
            "BATTLE_ORDER_WITNESS_FILE requires explicit BATTLE_SCHEDULER_MODE=dag");
        assert!(var_os("BATTLE_REGISTRATION_CONTROL").is_none(),
            "order witnesses cannot be combined with registration controls");
        let manifest: Value = from_slice(&read(&path).expect("cannot read witness manifest"))
            .expect("invalid witness manifest JSON");
        assert_eq!(manifest["schema"], 1, "unsupported witness manifest schema");
        let source_path = manifest["source_plan"].as_str().expect("source_plan is required");
        let source: Value = from_slice(&read(source_path).expect("cannot read witness source plan"))
            .expect("invalid witness source plan JSON");
        assert_eq!(source["schema"], 1, "unsupported witness source plan schema");
        assert_eq!(source["plan_valid"], true, "witness source must be a valid DAG plan");
        assert_eq!(source["input_nodes"][0]["name"], MAIN_FIRST_SYSTEM,
            "order witness source must identify the Battle main workload");
        let id = manifest["witness_id"].as_str().expect("witness_id is required").to_string();
        assert!(!id.is_empty(), "witness_id must not be empty");
        let output = PathBuf::from(manifest["output"].as_str().expect("output is required"));
        let order = array_ids(&manifest["occurrences"]);
        Some(Request { manifest, source, order, output, id })
    }).as_ref()
}

pub(crate) fn ensure_bound() {
    if request().is_some() {
        assert!(BOUND.load(Ordering::Acquire),
            "requested order witness did not match a registered Battle main workload");
    }
}

/// Validate complete topology by occurrence/name, never unstable Rust TypeIds.
pub(crate) fn prepare(
    name: &dyn Label,
    names: &[Box<dyn Label>],
    inputs: &[DagInput],
    occurrences: &[Vec<usize>],
    batches: &Batches,
) -> Option<OrderWitness> {
    let request = request()?;
    let first = inputs.first()?;
    if format!("{:?}", names[first.system_index]) != MAIN_FIRST_SYSTEM {
        return None;
    }
    let source = &request.source;
    assert_eq!(source["workload_name"], format!("{name:?}"), "witness workload label mismatch");
    assert_eq!(source["node_count"], inputs.len(), "witness node count mismatch");
    let source_nodes = source["input_nodes"].as_array().expect("source nodes missing");
    assert_eq!(source_nodes.len(), inputs.len(), "witness input count mismatch");
    for (occurrence, input) in inputs.iter().enumerate() {
        let old = &source_nodes[occurrence];
        assert_eq!(old["occurrence"], occurrence, "witness occurrence mapping mismatch");
        assert_eq!(old["name"], format!("{:?}", names[input.system_index]), "witness system name mismatch");
        assert_eq!(old["hard_after"], json!(input.hard_after), "witness hard ordering mismatch");
        assert_eq!(old["registered_fallible"], input.fallible, "witness fallibility mismatch");
        let borrows = old["borrow"].as_array().expect("source borrow list missing");
        assert_eq!(borrows.len(), input.borrow_constraints.len(), "witness borrow count mismatch");
        for (old, current) in borrows.iter().zip(&input.borrow_constraints) {
            assert_eq!(old["name"].as_str(), Some(current.name.as_ref()), "witness storage name mismatch");
            assert_eq!(old["mutability"], format!("{:?}", current.mutability), "witness mutability mismatch");
            assert_eq!(old["thread_safe"], current.thread_safe, "witness affinity mismatch");
        }
    }
    let old_batches = source["baseline_batches"].as_array().expect("source batches missing");
    assert_eq!(old_batches.len(), occurrences.len(), "witness batch count mismatch");
    for (batch, ids) in occurrences.iter().enumerate() {
        let old = &old_batches[batch];
        let condition = &batches.parallel_run_if[batch];
        assert_eq!(old["index"], batch, "witness batch index mismatch");
        assert_eq!(old["occurrences"], json!(ids), "witness batch membership mismatch");
        assert_eq!(!old["required_caller_system"].is_null(), batches.parallel[batch].0.is_some(),
            "witness required caller mismatch");
        if let Some(current) = batches.parallel[batch].0 {
            let old_index = old["required_caller_system"].as_u64().expect("invalid source caller") as usize;
            let old_occurrence = ids.iter().find(|&&id| source_nodes[id]["system_index"] == old_index)
                .expect("source caller not in its batch");
            assert_eq!(inputs[*old_occurrence].system_index, current, "witness caller occurrence mismatch");
        }
        assert_eq!(old["has_run_if"], condition.0 != usize::MAX || condition.1.iter().any(|&id| id != usize::MAX),
            "witness conditions mismatch");
        assert_eq!(old["has_registered_fallible"], ids.iter().any(|&id| inputs[id].fallible),
            "witness batch fallibility mismatch");
        for (position, &id) in ids.iter().enumerate() {
            assert!(!ids[..position].iter().any(|&other| inputs[other].system_index == inputs[id].system_index),
                "witness observer requires unambiguous function occurrences within a fence/batch");
        }
    }
    assert_eq!(request.order.len(), inputs.len(), "witness must include every occurrence");
    let mut rank = vec![usize::MAX; inputs.len()];
    for (position, &occurrence) in request.order.iter().enumerate() {
        let slot = rank.get_mut(occurrence).expect("witness occurrence out of range");
        assert_eq!(*slot, usize::MAX, "witness repeats an occurrence");
        *slot = position;
    }
    let plan = batches.dag.as_ref().expect("witness requires current DAG, no fallback permitted");
    let old_epochs = source["epochs"].as_array().expect("source epochs missing");
    assert_eq!(old_epochs.len(), plan.epochs.len(), "witness epoch count mismatch");
    let mut epoch_orders = Vec::new();
    let mut offset = 0;
    for (epoch_index, epoch) in plan.epochs.iter().enumerate() {
        let old = &old_epochs[epoch_index];
        assert_eq!(old["index"], epoch_index, "witness epoch index mismatch");
        match epoch {
            DagEpoch::Fence(batch) => {
                assert_eq!(old["kind"], "fence", "witness epoch kind mismatch");
                assert_eq!(old["baseline_batch"], *batch, "witness fence mismatch");
                assert_eq!(old["occurrences"], json!(occurrences[*batch]), "witness fence membership mismatch");
                let end = offset + occurrences[*batch].len();
                assert_eq!(request.order[offset..end], occurrences[*batch], "witness must retain original fence slots");
                offset = end;
                epoch_orders.push(Vec::new());
            }
            DagEpoch::Graph(graph) => {
                assert_eq!(old["kind"], "graph", "witness epoch kind mismatch");
                assert_eq!(old["roots"], json!(graph.roots), "witness roots mismatch");
                let old_nodes = old["nodes"].as_array().expect("source graph nodes missing");
                assert_eq!(old_nodes.len(), graph.nodes.len(), "witness graph node count mismatch");
                let mut order = Vec::with_capacity(graph.nodes.len());
                for (local, node) in graph.nodes.iter().enumerate() {
                    let old = &old_nodes[local];
                    assert_eq!(old["local"], local, "witness local ID mismatch");
                    assert_eq!(old["occurrence"], node.occurrence, "witness graph occurrence mismatch");
                    assert_eq!(old["name"], format!("{:?}", names[node.system_index]), "witness graph name mismatch");
                    assert_eq!(old["predecessor_count"], node.predecessors, "witness indegree mismatch");
                    assert_eq!(old["successors"], json!(node.successors), "witness edge mismatch");
                    assert!((offset..offset + graph.nodes.len()).contains(&rank[node.occurrence]),
                        "witness moved an occurrence across an epoch/fence");
                    for &next in &node.successors {
                        assert!(rank[node.occurrence] < rank[graph.nodes[next].occurrence],
                            "witness violates a current dependency edge");
                    }
                    order.push(local);
                }
                order.sort_unstable_by_key(|&local| rank[graph.nodes[local].occurrence]);
                offset += graph.nodes.len();
                epoch_orders.push(order);
            }
        }
    }
    assert_eq!(offset, inputs.len(), "witness incomplete epoch coverage");
    let file = OpenOptions::new().write(true).create_new(true).open(&request.output)
        .expect("cannot create new witness output (existing evidence is never overwritten)");
    let witness = OrderWitness {
        epoch_orders,
        batch_occurrences: occurrences.to_vec(),
        occurrence_systems: inputs.iter().map(|input| input.system_index).collect(),
        id: request.id.clone(),
        output: Mutex::new(BufWriter::new(file)),
        ticks: AtomicU64::new(0),
        running: AtomicBool::new(false),
    };
    witness.write(&json!({"schema": 1, "kind": "registration", "witness_id": request.id,
        "manifest": request.manifest, "node_count": inputs.len(), "epoch_count": plan.epochs.len(),
        "occurrence_names": inputs.iter().map(|n| format!("{:?}", names[n.system_index])).collect::<Vec<_>>(),
        "validation": "all occurrence names, borrow names/mutability/affinity, hard_after, baseline batches, fences and graph edges match; Rust TypeIds ignored",
        "execution": "serial topological graph epochs; original fence conditions, caller affinity and error handling",
        "event_codes": {"0": "body_start", "1": "body_ok", "2": "body_error", "3": "body_panic"}}));
    BOUND.store(true, Ordering::Release);
    Some(witness)
}
