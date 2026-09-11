//! A type that is not Send + Sync must fail to derive Component unless it
//! opts out with #[component(non_send)]. Silent acceptance here would let a
//! Vulkan handle wander onto a worker thread.
use flux_ecs::Component;
use std::rc::Rc;

#[derive(Component)]
struct Handle(Rc<u8>);

fn main() {}
