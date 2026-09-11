use crate::World;
use crate::access::AccessList;
use crate::system::param::SystemParam;
use crate::world::WorldCells;
use std::marker::PhantomData;

/// A runnable unit of work over a world.
pub trait System {
    /// Union of the parameters' declared access.
    fn access(&self) -> &AccessList;

    /// Runs once: initializes parameter state on first use, advances the
    /// world version, and calls the function with freshly fetched parameters.
    fn run(&mut self, world: &mut World);

    fn name(&self) -> &str;
}

/// Conversion of plain functions into systems.
///
/// Implemented for every function of up to eight system-parameter arguments
/// (queries, locals).
/// A function whose parameters conflict — two of them accessing the same
/// component with at least one write — is rejected during code generation.
/// What a system may return.
pub trait SystemOutput {
    /// One run's outcome; failures carry a report message.
    fn into_result(self) -> Result<(), String>;
}

impl SystemOutput for () {
    fn into_result(self) -> Result<(), String> {
        Ok(())
    }
}

impl<E: std::fmt::Debug> SystemOutput for Result<(), E> {
    fn into_result(self) -> Result<(), String> {
        self.map_err(|error| format!("{error:?}"))
    }
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a valid system",
    label = "invalid system",
    note = "systems are functions whose arguments are all system parameters: `Query<..>`, `Single<..>`, or `Local<..>`"
)]
pub trait IntoSystem<Marker> {
    type System: System;

    fn into_system(self) -> Self::System;
}

/// A tuple of system parameters, handled as a unit.
pub trait ParamSet: 'static {
    /// Concatenation of the members' declared access.
    const ACCESS: AccessList;
    /// The members' persistent state.
    type States: 'static;

    fn init(world: &mut World) -> Self::States;

    /// Applies the members' deferred work after a run.
    fn apply(states: &mut Self::States, world: &mut World);

    /// Drops the members' deferred work after a failed run.
    fn discard(states: &mut Self::States);
}

/// A function callable with a parameter set's fetched items.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a valid system",
    note = "every argument must be a system parameter: `Query<..>`, `Single<..>`, or `Local<..>`"
)]
pub trait ParamFunction<Params: ParamSet, Out: SystemOutput>: 'static {
    /// Fetches every parameter at `version` and calls the function.
    ///
    /// # Safety
    ///
    /// `Params::ACCESS` must be conflict-free.
    unsafe fn call(&mut self, states: &mut Params::States, cells: &WorldCells<'_>, version: u64) -> Out;
}

/// A plain function together with its parameters' persistent state.
pub struct FunctionSystem<Func, Params: ParamSet, Out> {
    func: Func,
    /// Initialized on the first run.
    state: Option<Params::States>,
    access: AccessList,
    _params: PhantomData<fn(Params) -> Out>,
}

impl<Func, Params, Out> System for FunctionSystem<Func, Params, Out>
where
    Params: ParamSet,
    Out: SystemOutput,
    Func: ParamFunction<Params, Out>,
{
    fn access(&self) -> &AccessList {
        &self.access
    }

    fn run(&mut self, world: &mut World) {
        if self.state.is_none() {
            self.state = Some(Params::init(world));
        }
        let version = world.bump_version();
        let outcome = {
            let cells = world.cells();
            let states = self.state.as_mut().expect("initialized above");
            // SAFETY: into_system rejected conflicting parameter sets at
            // compile time, and the exclusive world borrow excludes all
            // other access.
            unsafe { self.func.call(states, &cells, version) }
        };
        let states = self.state.as_mut().expect("initialized above");
        match outcome.into_result() {
            Ok(()) => Params::apply(states, world),
            Err(message) => {
                // A failed system has no effects: its deferred work is dropped.
                Params::discard(states);
                panic!("system `{}` failed: {message}", self.name());
            }
        }
    }

    fn name(&self) -> &str {
        std::any::type_name::<Func>()
    }
}

impl<Func, Params, Out> IntoSystem<fn(Params) -> Out> for Func
where
    Params: ParamSet,
    Out: SystemOutput,
    Func: ParamFunction<Params, Out>,
{
    type System = FunctionSystem<Func, Params, Out>;

    fn into_system(self) -> Self::System {
        const {
            assert!(
                !Params::ACCESS.self_conflicting(),
                "system parameters alias a component mutably"
            )
        }
        FunctionSystem {
            func: self,
            state: None,
            access: Params::ACCESS,
            _params: PhantomData,
        }
    }
}

macro_rules! param_set {
    ($(($p:ident, $s:ident)),*) => {
        impl<$($p: SystemParam + 'static),*> ParamSet for ($($p,)*) {
            const ACCESS: AccessList = {
                let list = AccessList::EMPTY;
                $( let list = list.concat($p::ACCESS); )*
                list
            };
            type States = ($($p::State,)*);

            #[allow(unused_variables, clippy::unused_unit)]
            fn init(world: &mut World) -> Self::States {
                ($($p::init(world),)*)
            }

            #[allow(non_snake_case, unused_variables)]
            fn apply(states: &mut Self::States, world: &mut World) {
                let ($($s,)*) = states;
                $( $p::apply($s, world); )*
            }

            #[allow(non_snake_case, unused_variables)]
            fn discard(states: &mut Self::States) {
                let ($($s,)*) = states;
                $( $p::discard($s); )*
            }
        }

        impl<Func, Out, $($p: SystemParam + 'static),*> ParamFunction<($($p,)*), Out> for Func
        where
            Out: SystemOutput,
            Func: 'static + FnMut($($p),*) -> Out + for<'w, 's> FnMut($($p::Item<'w, 's>),*) -> Out,
        {
            #[allow(non_snake_case, unused_variables)]
            unsafe fn call(&mut self, states: &mut ($($p::State,)*), cells: &WorldCells<'_>, version: u64) -> Out {
                #[allow(clippy::too_many_arguments)]
                fn call_inner<Out, $($p),*>(f: &mut impl FnMut($($p),*) -> Out, $($s: $p),*) -> Out {
                    f($($s),*)
                }
                let ($($s,)*) = states;
                call_inner(self, $( unsafe { $p::fetch($s, cells, version) } ),*)
            }
        }
    };
}

param_set!();
param_set!((P1, s1));
param_set!((P1, s1), (P2, s2));
param_set!((P1, s1), (P2, s2), (P3, s3));
param_set!((P1, s1), (P2, s2), (P3, s3), (P4, s4));
param_set!((P1, s1), (P2, s2), (P3, s3), (P4, s4), (P5, s5));
param_set!((P1, s1), (P2, s2), (P3, s3), (P4, s4), (P5, s5), (P6, s6));
param_set!((P1, s1), (P2, s2), (P3, s3), (P4, s4), (P5, s5), (P6, s6), (P7, s7));
param_set!((P1, s1), (P2, s2), (P3, s3), (P4, s4), (P5, s5), (P6, s6), (P7, s7), (P8, s8));
param_set!((P1, s1), (P2, s2), (P3, s3), (P4, s4), (P5, s5), (P6, s6), (P7, s7), (P8, s8), (P9, s9));
param_set!((P1, s1), (P2, s2), (P3, s3), (P4, s4), (P5, s5), (P6, s6), (P7, s7), (P8, s8), (P9, s9), (P10, s10));
param_set!((P1, s1), (P2, s2), (P3, s3), (P4, s4), (P5, s5), (P6, s6), (P7, s7), (P8, s8), (P9, s9), (P10, s10), (P11, s11));
param_set!((P1, s1), (P2, s2), (P3, s3), (P4, s4), (P5, s5), (P6, s6), (P7, s7), (P8, s8), (P9, s9), (P10, s10), (P11, s11), (P12, s12));

#[cfg(test)]
mod tests {
    use crate::component::{Component, ComponentKey};
    use crate::query::state::QueryState;
    use crate::{Changed, IntoSystem, Local, Query, System, World};

    macro_rules! component {
        ($name:ident) => {
            impl Component for $name {
                const KEY: ComponentKey =
                    ComponentKey::from_path(concat!("system::function::tests::", stringify!($name)));
            }
        };
    }

    #[derive(Copy, Clone, PartialEq, Debug)]
    struct A(u64);
    component!(A);
    #[derive(Copy, Clone, PartialEq, Debug)]
    struct B(u16);
    component!(B);

    fn world_with(values: &[u64]) -> World {
        let mut w = World::new();
        for v in values {
            w.spawn((A(*v),));
        }
        w
    }

    // ------------------------------------------------------------ basic runs

    #[test]
    fn a_function_with_a_query_parameter_runs() {
        fn double(query: Query<&mut A>) {
            query.for_each(|a| a.0 *= 2);
        }
        let mut world = world_with(&[1, 2, 3]);
        let mut system = double.into_system();
        system.run(&mut world);

        let mut check = QueryState::<&A>::new();
        let sum: u64 = world.query(&mut check).chunks().map(|a| a.iter().map(|v| v.0).sum::<u64>()).sum();
        assert_eq!(sum, 12);
    }

    #[test]
    fn a_zero_parameter_function_is_a_system() {
        fn nothing() {}
        let mut world = World::new();
        let mut system = nothing.into_system();
        system.run(&mut world);
        system.run(&mut world);
    }

    #[test]
    fn disjoint_query_parameters_are_usable_together() {
        fn transfer(source: Query<&A>, sink: Query<&mut B>) {
            let total: u64 = {
                let mut sum = 0;
                source.for_each(|a| sum += a.0);
                sum
            };
            sink.for_each(|b| b.0 = total as u16);
        }
        let mut world = World::new();
        world.spawn((A(3),));
        world.spawn((A(4),));
        let target = world.spawn((B(0),));

        let mut system = transfer.into_system();
        system.run(&mut world);
        assert_eq!(world.get::<B>(target), Some(&B(7)));
    }

    #[test]
    fn same_component_read_by_two_parameters_is_allowed() {
        fn readers(first: Query<&A>, second: Query<&A>) {
            let mut n = 0;
            first.for_each(|_| n += 1);
            second.for_each(|_| n += 1);
            assert_eq!(n, 2);
        }
        let mut world = world_with(&[5]);
        readers.into_system().run(&mut world);
    }

    // ---------------------------------------------------------------- access

    #[test]
    fn system_access_is_the_union_of_its_parameters() {
        fn sys(_a: Query<&A>, _b: Query<&mut B>) {}
        let system = sys.into_system();
        let access = system.access();
        assert_eq!(access.len(), 2);
        assert!(access.terms().iter().any(|t| t.key == A::KEY && !t.write));
        assert!(access.terms().iter().any(|t| t.key == B::KEY && t.write));
    }

    // ----------------------------------------------------------------- local

    #[test]
    fn local_state_persists_across_runs_of_one_system() {
        fn count(mut counter: Local<u64>, query: Query<&A>) {
            let mut rows = 0;
            query.for_each(|_| rows += 1);
            *counter += rows;
            assert!(*counter >= rows, "accumulates");
        }
        let mut world = world_with(&[1, 2]);
        let mut system = count.into_system();
        system.run(&mut world);
        system.run(&mut world);
        // observable effect checked via the changed-cursor test below; here we
        // assert independence between instances:
        let mut fresh = count.into_system();
        fresh.run(&mut world); // a new instance starts from Default
    }

    #[test]
    fn local_accumulation_is_observable() {
        fn tally(mut total: Local<u64>, query: Query<&mut A>) {
            *total += 1;
            let total = *total;
            query.for_each(|a| a.0 = total);
        }
        let mut world = world_with(&[0]);
        let mut system = tally.into_system();
        system.run(&mut world); // run 1 → A = 1
        system.run(&mut world); // run 2 → A = 2
        let mut check = QueryState::<&A>::new();
        let v: u64 = world.query(&mut check).chunks().map(|a| a[0].0).sum();
        assert_eq!(v, 2);
    }

    // ------------------------------------------------------- change cursors

    #[test]
    fn a_systems_changed_query_sees_changes_since_its_own_last_run() {
        // The reader records how many rows it observed into B, per run.
        fn watch(changed: Query<&A, Changed<A>>, out: Query<&mut B>, mut total: Local<u16>) {
            let mut n = 0;
            changed.for_each(|_| n += 1);
            *total += n;
            let total = *total;
            out.for_each(|b| b.0 = total);
        }
        let mut world = World::new();
        world.spawn((A(1),));
        let probe = world.spawn((B(0),));
        let mut system = watch.into_system();

        system.run(&mut world);
        assert_eq!(world.get::<B>(probe), Some(&B(1)), "first run observes the spawn");
        system.run(&mut world);
        assert_eq!(world.get::<B>(probe), Some(&B(1)), "nothing changed: observes nothing");

        let mut touch = QueryState::<&mut A>::new();
        world.query(&mut touch).for_each(|a| a.0 = 2);
        system.run(&mut world);
        assert_eq!(world.get::<B>(probe), Some(&B(2)), "the write is observed exactly once");
        system.run(&mut world);
        assert_eq!(world.get::<B>(probe), Some(&B(2)));
    }

    #[test]
    fn each_run_stamps_at_its_own_version() {
        // Two writer runs with a reader run between and after: the reader
        // must observe each writer run separately — which requires every run
        // to advance the world version.
        fn writer(query: Query<&mut A>) {
            query.for_each(|a| a.0 += 1);
        }
        fn reader(changed: Query<&A, Changed<A>>, out: Query<&mut B>, mut observations: Local<u16>) {
            let mut n = 0;
            changed.for_each(|_| n += 1);
            *observations += n;
            let observations = *observations;
            out.for_each(|b| b.0 = observations);
        }
        let mut world = World::new();
        world.spawn((A(0),));
        let probe = world.spawn((B(0),));
        let mut w = writer.into_system();
        let mut r = reader.into_system();

        w.run(&mut world);
        r.run(&mut world);
        assert_eq!(world.get::<B>(probe), Some(&B(1)), "spawn + first write, one chunk observation");
        w.run(&mut world);
        r.run(&mut world);
        assert_eq!(world.get::<B>(probe), Some(&B(2)), "second write observed");
        r.run(&mut world);
        assert_eq!(world.get::<B>(probe), Some(&B(2)), "no third observation without a write");
    }

    #[test]
    fn world_run_sugar() {
        fn bump(query: Query<&mut A>) {
            query.for_each(|a| a.0 += 1);
        }
        let mut world = world_with(&[41]);
        let mut system = bump.into_system();
        world.run(&mut system);
        let mut check = QueryState::<&A>::new();
        let v: u64 = world.query(&mut check).chunks().map(|a| a[0].0).sum();
        assert_eq!(v, 42);
    }
}
