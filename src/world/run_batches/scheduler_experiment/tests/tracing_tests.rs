use super::*;
use core::fmt::Debug;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::set_global_default;
use tracing::{Event, Metadata, Subscriber};

#[derive(Default)]
struct NameVisitor(String);

impl Visit for NameVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "name" {
            self.0 = format!("{value:?}");
        }
    }
}

struct SpanRecord {
    kind: &'static str,
    label: String,
    parent: Option<u64>,
    refs: usize,
    enters: usize,
    exits: usize,
}

#[derive(Default)]
struct State {
    sequence: AtomicU64,
    spans: Mutex<BTreeMap<u64, SpanRecord>>,
}

struct Collector(Arc<State>);

impl Subscriber for Collector {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, attrs: &Attributes<'_>) -> Id {
        let id = self.0.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let mut name = NameVisitor::default();
        attrs.record(&mut name);
        self.0.spans.lock().expect("span records").insert(
            id,
            SpanRecord {
                kind: attrs.metadata().name(),
                label: name.0,
                parent: attrs.parent().map(Id::into_u64),
                refs: 1,
                enters: 0,
                exits: 0,
            },
        );
        Id::from_u64(id)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, _: &Event<'_>) {}

    fn enter(&self, id: &Id) {
        self.0
            .spans
            .lock()
            .expect("span records")
            .get_mut(&id.into_u64())
            .expect("known span")
            .enters += 1;
    }

    fn exit(&self, id: &Id) {
        self.0
            .spans
            .lock()
            .expect("span records")
            .get_mut(&id.into_u64())
            .expect("known span")
            .exits += 1;
    }

    fn clone_span(&self, id: &Id) -> Id {
        self.0
            .spans
            .lock()
            .expect("span records")
            .get_mut(&id.into_u64())
            .expect("known span")
            .refs += 1;
        id.clone()
    }

    fn try_close(&self, id: Id) -> bool {
        let mut records = self.0.spans.lock().expect("span records");
        let record = records.get_mut(&id.into_u64()).expect("known span");
        record.refs -= 1;
        record.refs == 0
    }
}

#[test]
fn tracing_keeps_system_names_parent_and_balanced_lifetimes() {
    let state = Arc::new(State::default());
    set_global_default(Collector(state.clone())).expect("one global tracing test collector");
    for workers in [1, 4] {
        let pool = ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .expect("trace pool");
        let systems: Vec<System> = (0..8)
            .map(|_| Box::new(|_: &World| Ok(())) as System)
            .collect();
        let mut batch = batches(vec![(None, (0..8).collect())]);
        batch.parallel_run_if[0].1[3] = 0;
        batch.systems_run_if.push(Box::new(|_| Ok(false)));
        let label = format!("trace-direct-{workers}");
        pool.install(|| World::new().run_batches_direct(&systems, &names(8), &batch, &label))
            .expect("traced workload");
        let records = state.spans.lock().expect("span records");
        let parents: Vec<_> = records
            .iter()
            .filter(|(_, r)| r.kind == "workload" && r.label == label)
            .collect();
        assert_eq!(parents.len(), 1);
        let (&parent_id, parent) = parents[0];
        assert_eq!((parent.refs, parent.enters, parent.exits), (0, 1, 1));
        let children: Vec<_> = records
            .values()
            .filter(|r| r.parent == Some(parent_id))
            .collect();
        assert_eq!(children.len(), 7);
        let mut labels: Vec<_> = children.iter().map(|r| r.label.as_str()).collect();
        labels.sort_unstable();
        assert_eq!(labels, vec!["0", "1", "2", "4", "5", "6", "7"]);
        for child in children {
            assert_eq!(child.kind, "system");
            assert_eq!((child.refs, child.enters, child.exits), (0, 1, 1));
        }
    }
}
