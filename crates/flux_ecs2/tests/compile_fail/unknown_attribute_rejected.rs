//! A typo in the attribute must not be silently ignored — `#[component(nonsend)]`
//! quietly compiling as a normal component would be a soundness bug in waiting.
use flux_ecs2::Component;

#[derive(Component)]
#[component(nonsend)]
struct Handle(u32);

fn main() {}
