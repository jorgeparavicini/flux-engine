use crate::{ComponentKey, IntoSystem, System, World};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
/// Names one or more systems so ordering constraints can reference them.
///
/// Several systems may share a label; a constraint against it applies to all
/// of them.
pub struct SystemLabel(pub &'static str);

/// A pair of systems whose accesses conflict without an ordering between
/// them: their relative execution order is unspecified.
pub struct Ambiguity {
    /// Name of the earlier-added system.
    pub first: String,
    /// Name of the later-added system.
    pub second: String,
    /// The contested component.
    pub key: ComponentKey,
}

struct Entry {
    system: Box<dyn System>,
    label: Option<SystemLabel>,
    before: Vec<SystemLabel>,
    after: Vec<SystemLabel>,
    conditions: Vec<RunCondition>,
}

type RunCondition = Box<dyn Fn(&World) -> bool>;

#[derive(Default)]
/// An ordered collection of systems, run one after another.
///
/// Order is the systems' insertion order, refined by `before`/`after`
/// constraints; the compiled order is deterministic across rebuilds. Ordering
/// cycles and constraints against labels no system carries are rejected.
pub struct Schedule {
    entries: Vec<Entry>,
    order: Option<Vec<usize>>,
    ambiguities: Vec<Ambiguity>,
}

impl Schedule {
    /// Creates an empty schedule.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a system; the returned configuration orders and conditions it.
    pub fn add<M>(&mut self, system: impl IntoSystem<M, System: 'static>) -> SystemConfig<'_> {
        self.order = None;
        self.entries.push(Entry {
            system: Box::new(system.into_system()),
            label: None,
            before: Vec::new(),
            after: Vec::new(),
            conditions: Vec::new(),
        });
        SystemConfig(self.entries.last_mut().expect("system just added"))
    }

    /// Runs every system whose conditions all hold, in the compiled order.
    ///
    /// # Panics
    ///
    /// On ordering cycles, and on constraints naming a label no system
    /// carries.
    pub fn run(&mut self, world: &mut World) {
        self.compile();
        let order = self.order.as_ref().expect("schedule not compiled");
        for &index in order {
            let entry = &mut self.entries[index];
            if entry.conditions.iter().all(|condition| condition(world)) {
                entry.system.run(world);
            }
        }
    }

    /// The conflicting, unordered system pairs of this schedule.
    ///
    /// Ambiguities are legal — the serial executor runs them in insertion
    /// order — but their relative order is not part of the schedule's
    /// contract.
    pub fn ambiguities(&mut self) -> &[Ambiguity] {
        self.compile();
        &self.ambiguities
    }

    fn compile(&mut self) {
        if self.order.is_some() {
            return;
        }

        let n = self.entries.len();
        let mut successors = vec![Vec::new(); n];
        let mut indegree = vec![0; n];
        for (index, entry) in self.entries.iter().enumerate() {
            for constraint in &entry.before {
                for target in self.labelled(*constraint) {
                    successors[index].push(target);
                    indegree[target] += 1;
                }
            }

            for constraint in &entry.after {
                for target in self.labelled(*constraint) {
                    successors[target].push(index);
                    indegree[index] += 1;
                }
            }
        }

        let mut read: BinaryHeap<Reverse<usize>> = (0..n).filter(|i| indegree[*i] == 0).map(Reverse).collect();
        let mut order = Vec::with_capacity(n);

        while let Some(Reverse(index)) = read.pop() {
            order.push(index);
            for &next in &successors[index] {
                indegree[next] -= 1;
                if indegree[next] == 0 {
                    read.push(Reverse(next));
                }
            }
        }

        if order.len() != n {
            let stuck: Vec<&str> = (0..n)
                .filter(|i| indegree[*i] > 0)
                .map(|i| self.entries[i].system.name())
                .collect();
            panic!("system ordering cycle between: {stuck:?}");
        }

        self.ambiguities = self.find_ambiguities(&successors);
        self.order = Some(order);
    }

    /// Indices of every system labelled `label`; panics if there are none.
    fn labelled(&self, label: SystemLabel) -> Vec<usize> {
        let targets: Vec<usize> = self
            .entries.iter().enumerate().filter(|(_, entry)| entry.label == Some(label))
            .map(|(index, _)| index)
            .collect();
        assert!(
            !targets.is_empty(),
            "ordering constraint names the label {:?}, which no system carries",
            label.0
        );
        targets
    }

    /// Pairs with conflicting access and no path between them in the edge
    /// graph, in either direction.
    fn find_ambiguities(&self, successors: &[Vec<usize>]) -> Vec<Ambiguity> {
        let n = self.entries.len();

        let mut reachable = vec![vec![false; n]; n];
        fn mark(from: usize, at: usize, successors: &[Vec<usize>], reachable: &mut [Vec<bool>]) {
            for &next in &successors[at] {
                if !reachable[from][next] {
                    reachable[from][next] = true;
                    mark(from, next, successors, reachable);
                }
            }
        }

        for index in 0..n {
            mark(index, index, successors, &mut reachable);
        }

        let mut out = Vec::new();
        for first in 0..n {
            for second in first + 1..n {
                if reachable[first][second] || reachable[second][first] {
                    continue;
                }
                let a = self.entries[first].system.access();
                let b = self.entries[second].system.access();
                if a.conflicts_with(b) {
                    let key = a
                        .terms()
                        .iter()
                        .find(|t| b.terms().iter().any(|o| o.key == t.key && (o.write && t.write)))
                        .map(|t| t.key)
                        .expect("conflict implies a contested key");

                    out.push(Ambiguity {
                        first: self.entries[first].system.name().to_string(),
                        second: self.entries[second].system.name().to_string(),
                        key,
                    })
                }
            }
        }
        out
    }
}

/// Configuration of one added system, chainable.
pub struct SystemConfig<'a>(&'a mut Entry);

impl SystemConfig<'_> {
    /// Names this system.
    pub fn label(self, label: SystemLabel) -> Self {
        self.0.label = Some(label);
        self
    }

    /// Orders this system before every system labelled `label`.
    pub fn before(self, label: SystemLabel) -> Self {
        self.0.before.push(label);
        self
    }

    /// Orders this system after every system labelled `label`.
    pub fn after(self, label: SystemLabel) -> Self {
        self.0.after.push(label);
        self
    }

    /// Skips this system on runs where `condition` is false. Several
    /// conditions must all hold.
    pub fn run_if(self, condition: impl Fn(&World) -> bool + 'static) -> Self {
        self.0.conditions.push(Box::new(condition));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{Component, ComponentKey};
    use crate::{Query, Single, World};

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("schedule::tests::", stringify!($name)));
            }
        };
    }

    /// Execution trace: systems append their tag.
    #[derive(Default)]
    struct Trace(Vec<&'static str>);
    component!(Trace);
    #[derive(Copy, Clone, PartialEq, Debug, Default)]
    struct Flag(bool);
    component!(Flag);
    #[derive(Copy, Clone)]
    struct A(#[allow(dead_code)] u64);
    component!(A);

    fn traced(tag: &'static str) -> impl FnMut(Single<'_, &'_ mut Trace>) + 'static {
        move |mut trace: Single<&mut Trace>| trace.0.push(tag)
    }

    fn world() -> World {
        let mut w = World::new();
        w.insert_singleton(Trace(Vec::new()));
        w.insert_singleton(Flag(false));
        w
    }

    fn trace(w: &World) -> Vec<&'static str> {
        w.singleton::<Trace>().unwrap().0.clone()
    }

    // -------------------------------------------------------------- ordering

    #[test]
    fn insertion_order_is_the_default() {
        let mut schedule = Schedule::new();
        schedule.add(traced("a"));
        schedule.add(traced("b"));
        schedule.add(traced("c"));
        let mut w = world();
        schedule.run(&mut w);
        assert_eq!(trace(&w), vec!["a", "b", "c"]);
    }

    #[test]
    fn after_reorders() {
        const FIRST: SystemLabel = SystemLabel("first");
        let mut schedule = Schedule::new();
        schedule.add(traced("b")).after(FIRST);
        schedule.add(traced("a")).label(FIRST);
        let mut w = world();
        schedule.run(&mut w);
        assert_eq!(trace(&w), vec!["a", "b"]);
    }

    #[test]
    fn before_reorders() {
        const LAST: SystemLabel = SystemLabel("last");
        let mut schedule = Schedule::new();
        schedule.add(traced("z")).label(LAST);
        schedule.add(traced("a")).before(LAST);
        let mut w = world();
        schedule.run(&mut w);
        assert_eq!(trace(&w), vec!["a", "z"]);
    }

    #[test]
    fn chains_compose() {
        const MIDDLE: SystemLabel = SystemLabel("middle");
        const END: SystemLabel = SystemLabel("end");
        let mut schedule = Schedule::new();
        schedule.add(traced("c")).label(END).after(MIDDLE);
        schedule.add(traced("b")).label(MIDDLE);
        schedule.add(traced("a")).before(MIDDLE);
        let mut w = world();
        schedule.run(&mut w);
        assert_eq!(trace(&w), vec!["a", "b", "c"]);
    }

    #[test]
    fn a_label_shared_by_several_systems_constrains_all_of_them() {
        const PHASE: SystemLabel = SystemLabel("phase");
        let mut schedule = Schedule::new();
        schedule.add(traced("p1")).label(PHASE);
        schedule.add(traced("p2")).label(PHASE);
        schedule.add(traced("early")).before(PHASE);
        let mut w = world();
        schedule.run(&mut w);
        let t = trace(&w);
        assert_eq!(t[0], "early", "before(PHASE) precedes every PHASE system");
        assert_eq!(
            &t[1..],
            &["p1", "p2"],
            "labelled systems keep insertion order"
        );
    }

    #[test]
    fn unconstrained_ties_keep_insertion_order_deterministically() {
        // build the same schedule twice; order must be identical
        for _ in 0..2 {
            const LAST: SystemLabel = SystemLabel("last");
            let mut schedule = Schedule::new();
            schedule.add(traced("x")).before(LAST);
            schedule.add(traced("y")).before(LAST);
            schedule.add(traced("z")).label(LAST);
            let mut w = world();
            schedule.run(&mut w);
            assert_eq!(trace(&w), vec!["x", "y", "z"]);
        }
    }

    #[test]
    #[should_panic(expected = "cycle")]
    fn ordering_cycles_are_rejected() {
        const A_L: SystemLabel = SystemLabel("a");
        const B_L: SystemLabel = SystemLabel("b");
        let mut schedule = Schedule::new();
        schedule.add(traced("a")).label(A_L).before(B_L);
        schedule.add(traced("b")).label(B_L).before(A_L);
        schedule.run(&mut world());
    }

    #[test]
    #[should_panic(expected = "nonexistent")]
    fn unknown_labels_are_rejected() {
        let mut schedule = Schedule::new();
        schedule.add(traced("a")).after(SystemLabel("nonexistent"));
        schedule.run(&mut world());
    }

    // -------------------------------------------------------- run conditions

    #[test]
    fn run_if_gates_execution_per_run() {
        let mut schedule = Schedule::new();
        schedule
            .add(traced("gated"))
            .run_if(|w: &World| w.singleton::<Flag>().unwrap().0);
        let mut w = world();
        schedule.run(&mut w);
        assert_eq!(trace(&w), Vec::<&str>::new(), "false condition skips");

        w.singleton_mut::<Flag>().unwrap().0 = true;
        schedule.run(&mut w);
        assert_eq!(trace(&w), vec!["gated"], "condition re-evaluated per run");
    }

    #[test]
    fn every_condition_must_hold() {
        let mut schedule = Schedule::new();
        schedule
            .add(traced("picky"))
            .run_if(|_: &World| true)
            .run_if(|w: &World| w.singleton::<Flag>().unwrap().0);
        let mut w = world();
        schedule.run(&mut w);
        assert!(trace(&w).is_empty());
    }

    #[test]
    fn skipped_systems_do_not_block_the_rest() {
        let mut schedule = Schedule::new();
        schedule.add(traced("skipped")).run_if(|_: &World| false);
        schedule.add(traced("runs"));
        let mut w = world();
        schedule.run(&mut w);
        assert_eq!(trace(&w), vec!["runs"]);
    }

    // ------------------------------------------------------------ ambiguities

    fn writes_a(_q: Query<&mut A>) {}
    fn also_writes_a(_q: Query<&mut A>) {}
    fn reads_a(_q: Query<&A>) {}

    #[test]
    fn unordered_conflicting_systems_are_reported() {
        let mut schedule = Schedule::new();
        schedule.add(writes_a);
        schedule.add(also_writes_a);
        let ambiguities = schedule.ambiguities();
        assert_eq!(ambiguities.len(), 1);
        assert_eq!(
            ambiguities[0].key,
            A::KEY,
            "the contested component is named"
        );
        assert!(ambiguities[0].first.contains("writes_a"));
        assert!(ambiguities[0].second.contains("also_writes_a"));
    }

    #[test]
    fn ordering_resolves_the_ambiguity() {
        const W: SystemLabel = SystemLabel("w");
        let mut schedule = Schedule::new();
        schedule.add(writes_a).label(W);
        schedule.add(also_writes_a).after(W);
        assert!(schedule.ambiguities().is_empty());
    }

    #[test]
    fn shared_reads_and_disjoint_systems_are_not_ambiguous() {
        let mut schedule = Schedule::new();
        schedule.add(reads_a);
        schedule.add(reads_a);
        schedule.add(traced("other"));
        assert!(schedule.ambiguities().is_empty(), "reads never contest");
    }

    #[test]
    fn transitive_order_counts_as_ordered() {
        const M: SystemLabel = SystemLabel("m");
        const E: SystemLabel = SystemLabel("e");
        let mut schedule = Schedule::new();
        schedule.add(writes_a).before(M);
        schedule.add(traced("mid")).label(M).before(E);
        schedule.add(also_writes_a).label(E);
        assert!(
            schedule.ambiguities().is_empty(),
            "a path through an unrelated system still orders the pair"
        );
    }

    // -------------------------------------------------------------- reuse

    #[test]
    fn schedules_recompile_after_additions() {
        const FIRST: SystemLabel = SystemLabel("first");
        let mut schedule = Schedule::new();
        schedule.add(traced("a")).label(FIRST);
        let mut w = world();
        schedule.run(&mut w);
        schedule.add(traced("b")).before(FIRST);
        schedule.run(&mut w);
        assert_eq!(
            trace(&w),
            vec!["a", "b", "a"],
            "new constraint honoured on the next run"
        );
    }
}
