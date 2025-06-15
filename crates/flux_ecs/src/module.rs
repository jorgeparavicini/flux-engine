use crate::world::World;

pub trait Module {
    fn register(world: &mut World);
}
