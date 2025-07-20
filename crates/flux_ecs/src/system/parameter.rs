use crate::world::World;
use variadics_please::all_tuples;

pub trait SystemParam: Sized {
    type State: 'static;

    type Item<'world, 'state>: SystemParam<State=Self::State>;

    fn init_state(world: &mut World) -> Self::State;

    fn get_param<'world, 'state>(
        state: &'state Self::State,
        world: &'world mut World,
    ) -> Self::Item<'world, 'state>;

    fn apply_buffers(
        #[allow(unused_variables)] state: &Self::State,
        #[allow(unused_variables)] world: &mut World,
    ) {}
}

pub type SystemParamItem<'world, 'state, P> = <P as SystemParam>::Item<'world, 'state>;

macro_rules! impl_system_param {
    ($(($T:ident, $t:ident)),*) => {
        impl<$($T: SystemParam),*> SystemParam for ($($T,)*) {
            type State = ($($T::State,)*);
            type Item<'world, 'state> = ($($T::Item<'world, 'state>,)*);

            fn init_state(#[allow(unused_variables)]world: &mut World) -> Self::State {
                ($($T::init_state(world),)*)
            }

            fn get_param<'world, 'state>(
                state: &'state Self::State,
                #[allow(unused_variables)]
                world: &'world mut World,
            ) -> Self::Item<'world, 'state> {
                let ($($t,)*) = state;
                $(let $t = $T::get_param($t, unsafe { &mut *(world as *mut World) });)*
                ($($t,)*)
            }
        }
    };
}

all_tuples!(impl_system_param, 0, 15, T, t);