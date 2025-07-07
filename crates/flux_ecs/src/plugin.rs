use crate::world::World;

pub trait Plugin {
    fn init(&self, world: &mut World);
}