use crate::world::World;

pub mod function_system;
pub mod parameter;
pub mod systems;

pub trait System: 'static {
    fn run(&mut self, world: &mut World);

    fn initialize(&mut self, world: &mut World);
}

pub trait IntoSystem<Marker>: Sized {
    type System: System;

    fn into_system(self) -> Self::System;
}

// Every system can be converted into a system ... kinda obvious, isn't it?
impl<T: System> IntoSystem<()> for T {
    type System = T;

    fn into_system(self) -> Self::System {
        self
    }
}
