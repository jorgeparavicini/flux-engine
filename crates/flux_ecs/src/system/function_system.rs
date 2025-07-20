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

macro_rules! impl_infallible_system_param_function {
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
                    F: FnMut($($P),*)
                {
                    f($($p),*)
                }
                let ($($p,)*) = param;
                call_inner(self, $($p),*);
                Ok(())
            }
        }
    }
}

all_tuples!(impl_infallible_system_param_function, 0, 15, P, p);

macro_rules! impl_fallible_system_param_function {
    ($(($P:ident,$p:ident)),*) => {
        impl<Func, $($P: SystemParam),*, Error> SystemParamFunction<fn($($P),*) -> Result<(), Error>> for Func
        where
            Func: 'static,
            for<'a> &'a mut Func: FnMut($($P),*) -> Result<(), Error> + FnMut($(SystemParamItem<$P>,)*) -> Result<(), Error>,
            Error: std::error::Error + 'static
        {
            type Param = ($($P,)*);
            type Error = Infallible;

            fn run(&mut self, param: SystemParamItem<Self::Param>) -> Result<(), Self::Error> {
                fn call_inner<F, $($P),*, E>(mut f: F, $($p: $P),*) -> Result<(), E>
                where
                    F: FnMut($($P),*) -> Result<(), E>,
                    Error: std::error::Error + 'static
                {
                    f($($p),*)
                }
                let ($($p,)*) = param;
                call_inner(self, $($p),*)
            }
        }
    }
}

all_tuples!(impl_fallible_system_param_function, 0, 15, P, p);
