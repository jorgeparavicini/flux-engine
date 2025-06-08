use crate::world::World;
use crate::{
    system::parameter::{SystemParam, SystemParamItem},
    system::{IntoSystem, System},
};
use std::marker::PhantomData;

pub trait SystemParamFunction<Marker>: 'static {
    type Param: SystemParam;

    fn run(&mut self, param: SystemParamItem<Self::Param>);
}

pub struct FunctionSystem<Marker, F>
where
    F: SystemParamFunction<Marker>,
{
    func: F,
    state: Option<FunctionSystemState<F::Param>>,
    _marker: PhantomData<fn() -> Marker>,
}

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

        let state = self.state.as_mut().expect("State isn't initialized");
        let params = F::Param::get_param(&mut state.param, world);

        self.func.run(params);
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

impl<Func> SystemParamFunction<fn() -> ()> for Func
where
    Func: 'static,
    for<'a> &'a mut Func: FnMut() -> () + FnMut() -> (),
{
    type Param = ();

    fn run(&mut self, param: SystemParamItem<Self::Param>) {
        fn call_inner<F: FnMut() -> ()>(mut f: F) {
            f();
        }
        let () = param;
        call_inner(self);
    }
}

impl<Func, F0: SystemParam> SystemParamFunction<fn(F0) -> ()> for Func
where
    Func: 'static,
    for<'a> &'a mut Func: FnMut(F0) -> () + FnMut(SystemParamItem<F0>) -> (),
{
    type Param = (F0,);

    fn run(&mut self, param: SystemParamItem<Self::Param>) {
        fn call_inner<F, F0>(mut f: F, f0: F0)
        where
            F: FnMut(F0) -> (),
        {
            f(f0);
        }
        let (f0,) = param;
        call_inner(self, f0);
    }
}
