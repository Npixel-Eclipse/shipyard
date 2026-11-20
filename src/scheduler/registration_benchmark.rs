//! Local registration measurements. No workload systems are run by this module.
use crate::error::AddWorkload;
use crate::scheduler::{Workload, WorkloadInfo};
use crate::World;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use serde::Serialize;
use std::env::{var, var_os, VarError};
use std::sync::OnceLock;
use std::thread_local;
use std::time::Instant;

/// Same process switch for the isolated example and actual Battle startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    /// No override: retain ordinary experimental DAG construction.
    Unset,
    /// Explicit construction enabled, with baseline execution.
    Graph,
    /// Skip DAG-only registration work, with baseline execution.
    NoGraph,
}

/// Explicit controls require baseline execution and no plan dump. Set the
/// environment before launch; mutation during process lifetime is unsupported.
pub fn control() -> Control {
    static CONTROL: OnceLock<Control> = OnceLock::new();
    *CONTROL.get_or_init(|| {
        let selected = match var("BATTLE_REGISTRATION_CONTROL") {
            Err(VarError::NotPresent) => return Control::Unset,
            Ok(value) if value == "graph" => Control::Graph,
            Ok(value) if value == "no_graph" => Control::NoGraph,
            other => panic!("Invalid BATTLE_REGISTRATION_CONTROL: {other:?}"),
        };
        assert_eq!(var("BATTLE_SCHEDULER_MODE").as_deref(), Ok("baseline"),
            "Registration controls require explicit BATTLE_SCHEDULER_MODE=baseline");
        assert!(var_os("BATTLE_SCHEDULER_PLAN_OUTPUT_DIR").is_none(),
            "Registration measurements require plan output to be disabled");
        selected
    })
}

pub(crate) fn build_graph_enabled() -> bool {
    control() != Control::NoGraph
}

pub(crate) fn validate_execution_mode(is_baseline: bool) {
    assert!(control() == Control::Unset || is_baseline,
        "Registration controls cannot be combined with direct/DAG execution");
}

#[derive(Default, Debug, Serialize)]
/// Instrumented registration phases; nested durations must not be added twice.
pub struct Phases {
    /// Registration of functions and their scheduler metadata.
    pub insert_systems_ns: u128,
    /// Requirement checks and ordering propagation.
    pub requirements_ns: u128,
    /// Copies needed only by the DAG builder.
    pub dag_input_capture_ns: u128,
    /// Original batch ordering, including occurrence capture when enabled.
    pub baseline_batches_ns: u128,
    /// Whole DAG construction, including edge construction and reduction.
    pub dag_plan_build_ns: u128,
    /// Graph node setup and explicit/borrow edge construction.
    pub dag_edges_ns: u128,
    /// Removal of transitively redundant edges.
    pub dag_transitive_reduction_ns: u128,
    /// Entire create_workload call, nested within add_to_world.
    pub create_workload_ns: u128,
    /// Number of actual create_workload calls.
    pub registrations: usize,
    /// Total system occurrences.
    pub nodes: usize,
    /// Total original parallel batches.
    pub batches: usize,
    /// Number of attempted DAG plans.
    pub graph_attempts: usize,
    /// Number of plans accepted by DAG validation.
    pub graph_successes: usize,
    /// Attempts that would use the original batch executor.
    pub fallback_count: usize,
}

pub(crate) enum Phase {
    InsertSystems, Requirements, DagInputCapture, BaselineBatches, DagPlanBuild,
    DagEdges, DagReduction, CreateWorkload,
}

thread_local! {
    static ACTIVE: RefCell<Option<Phases>> = const { RefCell::new(None) };
}

pub(crate) struct Timer(Option<Instant>);

impl Timer {
    pub(crate) fn start() -> Self {
        Self(ACTIVE.with(|state| state.borrow().is_some()).then(Instant::now))
    }

    pub(crate) fn finish(self, phase: Phase) {
        let Some(start) = self.0 else { return };
        let elapsed = start.elapsed().as_nanos();
        ACTIVE.with(|state| {
            if let Some(value) = state.borrow_mut().as_mut() {
                let target = match phase {
                    Phase::InsertSystems => &mut value.insert_systems_ns,
                    Phase::Requirements => &mut value.requirements_ns,
                    Phase::DagInputCapture => &mut value.dag_input_capture_ns,
                    Phase::BaselineBatches => &mut value.baseline_batches_ns,
                    Phase::DagPlanBuild => &mut value.dag_plan_build_ns,
                    Phase::DagEdges => &mut value.dag_edges_ns,
                    Phase::DagReduction => &mut value.dag_transitive_reduction_ns,
                    Phase::CreateWorkload => &mut value.create_workload_ns,
                };
                *target += elapsed;
            }
        });
    }
}

pub(crate) fn record(nodes: usize, batches: usize, attempted: bool, success: bool) {
    ACTIVE.with(|state| {
        if let Some(value) = state.borrow_mut().as_mut() {
            value.registrations += 1;
            value.nodes += nodes;
            value.batches += batches;
            value.graph_attempts += usize::from(attempted);
            value.graph_successes += usize::from(success);
            value.fallback_count += usize::from(attempted && !success);
        }
    });
}

struct ActiveGuard;
impl Drop for ActiveGuard {
    fn drop(&mut self) {
        ACTIVE.with(|state| { state.borrow_mut().take(); });
    }
}

/// Formatting/output is measured by the caller, outside all registration times.
#[derive(Debug, Serialize)]
pub struct Sample {
    /// Verified process registration control.
    pub control: Control,
    /// Empty ECS World construction.
    pub world_create_ns: u128,
    /// Actual Battle Workload builder construction.
    pub workload_builder_ns: u128,
    /// Real add_to_world, including scheduler registration and storage tracking.
    pub add_to_world_ns: u128,
    /// Post-measurement topology inspection.
    pub inspect_ns: u128,
    /// Destruction of the fresh World.
    pub world_drop_ns: u128,
    /// Internal registration phase timings and observed counts.
    pub phases: Phases,
    /// Ordered original-batch system names, for exact OFF/ON comparison.
    pub baseline_topology: Vec<Vec<String>>,
}

/// Measure one actual registration in a fresh World, without executing systems.
pub fn measure_registration(make_workload: impl FnOnce() -> Workload) -> Result<Sample, AddWorkload> {
    let selected = control();
    assert_ne!(selected, Control::Unset, "Select graph/no_graph explicitly");
    let start = Instant::now();
    let world = World::new();
    let world_create_ns = start.elapsed().as_nanos();
    let start = Instant::now();
    let workload = make_workload();
    let workload_builder_ns = start.elapsed().as_nanos();
    ACTIVE.with(|state| {
        assert!(state.borrow().is_none(), "Nested registration measurement");
        *state.borrow_mut() = Some(Phases::default());
    });
    let _guard = ActiveGuard;
    let start = Instant::now();
    workload.add_to_world(&world)?;
    let add_to_world_ns = start.elapsed().as_nanos();
    let phases = ACTIVE.with(|state| state.borrow_mut().take())
        .expect("registration phases remain available");
    let start = Instant::now();
    let info = world.workloads_info();
    let mut topology = Vec::new();
    for WorkloadInfo { batches_info, .. } in info.0.into_values() {
        for batch in batches_info {
            topology.push(batch.systems.0.into_iter().chain(batch.systems.1)
                .map(|system| system.name).collect());
        }
    }
    let inspect_ns = start.elapsed().as_nanos();
    let start = Instant::now();
    drop(world);
    let world_drop_ns = start.elapsed().as_nanos();
    Ok(Sample { control: selected, world_create_ns, workload_builder_ns, add_to_world_ns,
        inspect_ns, world_drop_ns, phases, baseline_topology: topology })
}
