use crate::world::World;
use crate::{
    system::parameter::{SystemParam, SystemParamItem},
    system::{IntoSystem, System},
};
use std::convert::Infallible;
use std::error::Error;
use std::marker::PhantomData;
use variadics_please::all_tuples;

/// The user-defined function that will be executed as a system.
pub trait SystemParamFunction<Marker>: 'static {
    type Param: SystemParam;
    type Error: Error + 'static;

    fn run(&mut self, param: SystemParamItem<Self::Param>) -> Result<(), Self::Error>;
}

/// A system that runs a user-defined function with system parameters.
pub struct FunctionSystem<Marker, F>
where
    F: SystemParamFunction<Marker>,
{
    func: F,
    state: Option<FunctionSystemState<F::Param>>,
    name: &'static str,
    _marker: PhantomData<fn() -> Marker>,
}

/// The state of the function system that holds data over multiple runs.
struct FunctionSystemState<P: SystemParam> {
    param: P::State,
}

pub struct IsFunctionSystem;

impl<Marker, F> IntoSystem<(IsFunctionSystem, Marker)> for F
where
    Marker: 'static,
    F: SystemParamFunction<Marker>,
{
    type System = FunctionSystem<Marker, F>;

    fn into_system(self) -> Self::System {
        FunctionSystem {
            func: self,
            state: None,
            name: std::any::type_name::<F>(),
            _marker: PhantomData,
        }
    }
}

impl<Marker, F> System for FunctionSystem<Marker, F>
where
    Marker: 'static,
    F: SystemParamFunction<Marker>,
{
    fn run(&mut self, world: &mut World) {
        if self.state.is_none() {
            self.initialize(world);
        }

        let state = self
            .state
            .as_ref()
            .expect("FunctionSystem::run called before FunctionSystem::initialize");
        let params = F::Param::get_param(&state.param, world);

        if let Err(e) = self.func.run(params) {
            panic!("Error in function system '{}': {}", self.name, e);
        }

        // TODO: This is just a placeholder.
        F::Param::apply_buffers(&state.param, world);
    }

    fn initialize(&mut self, world: &mut World) {
        if self.state.is_some() {
            return;
        }

        self.state = Some(FunctionSystemState {
            param: F::Param::init_state(world),
        });
    }
}

// TODO: Convert to a macro to avoid boilerplate code.
macro_rules! impl_system_param_function {
        ($(($P:ident,$p:ident)),*) => {
        impl<Func, $($P: SystemParam),*> SystemParamFunction<fn($($P),*) -> ()> for Func
        where
            Func: 'static,
            for<'a> &'a mut Func: FnMut($($P),*) + FnMut($(SystemParamItem<$P>,)*)
        {
            type Param = ($($P,)*);
            type Error = Infallible;

            fn run(&mut self, param: SystemParamItem<Self::Param>) -> Result<(), Self::Error> {
                fn call_inner<F, $($P),*>(mut f: F, $($p: $P),*)
                where
                F: FnMut($($P),*) {
                    f($($p),*)
                }
                let ($($p,)*) = param;
                call_inner(self, $($p),*);
                Ok(())
            }
        }
    }
}


// TODO: For more parameters we need to create a macro for the SystemParamItem first
all_tuples!(impl_system_param_function, 0, 3, P, p);

impl<Func, Error> SystemParamFunction<fn() -> Result<(), Error>> for Func
where
    Func: 'static,
    for<'a> &'a mut Func: FnMut() -> Result<(), Error>,
    Error: std::error::Error + 'static,
{
    type Param = ();
    type Error = Error;

    fn run(&mut self, param: SystemParamItem<Self::Param>) -> Result<(), Self::Error> {
        fn call_inner<F: FnMut() -> Result<(), E>, E: std::error::Error>(
            mut f: F,
        ) -> Result<(), E> {
            f()
        }
        let () = param;
        call_inner(self)
    }
}

impl<Func, F0: SystemParam, Error> SystemParamFunction<fn(F0) -> Result<(), Error>> for Func
where
    Func: 'static,
    for<'a> &'a mut Func:
        FnMut(F0) -> Result<(), Error> + FnMut(SystemParamItem<F0>) -> Result<(), Error>,
    Error: std::error::Error + 'static,
{
    type Param = (F0,);
    type Error = Error;

    fn run(&mut self, param: SystemParamItem<Self::Param>) -> Result<(), Self::Error> {
        fn call_inner<F, F0, E>(mut f: F, f0: F0) -> Result<(), E>
        where
            F: FnMut(F0) -> Result<(), E>,
            E: std::error::Error,
        {
            f(f0)
        }
        let (f0,) = param;
        call_inner(self, f0)
    }
}

impl<Func, F0: SystemParam, F1: SystemParam, Error>
    SystemParamFunction<fn(F0, F1) -> Result<(), Error>> for Func
where
    Func: 'static,
    for<'a> &'a mut Func: FnMut(F0, F1) -> Result<(), Error>
        + FnMut(SystemParamItem<F0>, SystemParamItem<F1>) -> Result<(), Error>,
    Error: std::error::Error + 'static,
{
    type Param = (F0, F1);
    type Error = Error;

    fn run(&mut self, param: SystemParamItem<Self::Param>) -> Result<(), Self::Error> {
        fn call_inner<F, F0, F1, E>(mut f: F, f0: F0, f1: F1) -> Result<(), E>
        where
            F: FnMut(F0, F1) -> Result<(), E>,
            E: std::error::Error,
        {
            f(f0, f1)
        }
        let (f0, f1) = param;
        call_inner(self, f0, f1)
    }
}

impl<Func, F0: SystemParam, F1: SystemParam, F2: SystemParam, Error>
    SystemParamFunction<fn(F0, F1, F2) -> Result<(), Error>> for Func
where
    Func: 'static,
    for<'a> &'a mut Func: FnMut(F0, F1, F2) -> Result<(), Error>
        + FnMut(SystemParamItem<F0>, SystemParamItem<F1>, SystemParamItem<F2>) -> Result<(), Error>,
    Error: std::error::Error + 'static,
{
    type Param = (F0, F1, F2);
    type Error = Error;

    fn run(&mut self, param: SystemParamItem<Self::Param>) -> Result<(), Self::Error> {
        fn call_inner<F, F0, F1, F2, E>(mut f: F, f0: F0, f1: F1, f2: F2) -> Result<(), E>
        where
            F: FnMut(F0, F1, F2) -> Result<(), E>,
            E: std::error::Error,
        {
            f(f0, f1, f2)
        }
        let (f0, f1, f2) = param;
        call_inner(self, f0, f1, f2)
    }
}
