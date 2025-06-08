use crate::world::World;

pub trait SystemParam: Sized {
    type State: 'static;

    type Item<'world, 'state>: SystemParam<State = Self::State>;

    fn init_state(world: &mut World) -> Self::State;

    fn get_param<'world, 'state>(
        state: &'state Self::State,
        world: &'world World,
    ) -> Self::Item<'world, 'state>;
}

pub type SystemParamItem<'world, 'state, P> = <P as SystemParam>::Item<'world, 'state>;

impl SystemParam for () {
    type State = ();

    type Item<'world, 'state> = ();

    fn init_state(_: &mut World) -> Self::State {
        // No state needed for unit type
        ()
    }

    fn get_param<'world, 'state>(
        _: &'state Self::State,
        _: &'world World,
    ) -> Self::Item<'world, 'state> {
        ()
    }
}

impl<P: SystemParam> SystemParam for (P,) {
    type State = (P::State,);

    type Item<'world, 'state> = (P::Item<'world, 'state>,);

    fn init_state(world: &mut World) -> Self::State {
        (P::init_state(world),)
    }

    fn get_param<'world, 'state>(
        state: &'state Self::State,
        world: &'world World,
    ) -> Self::Item<'world, 'state> {
        let (p,) = state;
        (P::get_param(p, world),)
    }
}
