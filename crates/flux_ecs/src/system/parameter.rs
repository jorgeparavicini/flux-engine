use crate::world::World;

pub trait SystemParam: Sized {
    type State: 'static;

    type Item<'world, 'state>: SystemParam<State=Self::State>;

    fn init_state(world: &mut World) -> Self::State;

    fn get_param<'world, 'state>(
        state: &'state Self::State,
        world: &'world mut World,
    ) -> Self::Item<'world, 'state>;

    fn apply_buffers(state: &Self::State, world: &mut World) {}
}

pub type SystemParamItem<'world, 'state, P> = <P as SystemParam>::Item<'world, 'state>;

impl SystemParam for () {
    type State = ();

    type Item<'world, 'state> = ();

    fn init_state(_: &mut World) -> Self::State {}

    fn get_param<'world, 'state>(
        _: &'state Self::State,
        _: &'world mut World,
    ) -> Self::Item<'world, 'state> {}
}

// TODO: Create macro to generate tuples of SystemParams
impl<P: SystemParam> SystemParam for (P,) {
    type State = (P::State,);

    type Item<'world, 'state> = (P::Item<'world, 'state>,);

    fn init_state(world: &mut World) -> Self::State {
        (P::init_state(world),)
    }

    fn get_param<'world, 'state>(
        state: &'state Self::State,
        world: &'world mut World,
    ) -> Self::Item<'world, 'state> {
        let (p, ) = state;
        (P::get_param(p, world),)
    }
}

impl<P: SystemParam, Q: SystemParam> SystemParam for (P, Q) {
    type State = (P::State, Q::State);

    type Item<'world, 'state> = (P::Item<'world, 'state>, Q::Item<'world, 'state>);

    fn init_state(world: &mut World) -> Self::State {
        (P::init_state(world), Q::init_state(world))
    }

    fn get_param<'world, 'state>(
        state: &'state Self::State,
        world: &'world mut World,
    ) -> Self::Item<'world, 'state> {
        let (p, q) = state;
        // SAFETY: The implementor of `SystemParam` must ensure that the state for `P` and `Q` 
        // do not overlap in memory.
        let p = P::get_param(p, unsafe {&mut *(world as *mut World)});
        let q = Q::get_param(q, world);
        (p, q)
    }
}

impl<P: SystemParam, Q: SystemParam, R: SystemParam> SystemParam for (P, Q, R) {
    type State = (P::State, Q::State, R::State);

    type Item<'world, 'state> = (P::Item<'world, 'state>, Q::Item<'world, 'state>, R::Item<'world, 'state>);

    fn init_state(world: &mut World) -> Self::State {
        (P::init_state(world), Q::init_state(world), R::init_state(world))
    }

    fn get_param<'world, 'state>(
        state: &'state Self::State,
        world: &'world mut World,
    ) -> Self::Item<'world, 'state> {
        let (p, q, r) = state;
        let p = P::get_param(p, unsafe {&mut *(world as *mut World)});
        let q = Q::get_param(q, unsafe {&mut *(world as *mut World)});
        let r = R::get_param(r, unsafe {&mut *(world as *mut World)});
        (p, q, r)
    }
}
