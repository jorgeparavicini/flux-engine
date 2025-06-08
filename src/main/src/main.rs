use flux_engine_ecs::resource::{Res, Resource};
use flux_engine_ecs::world::World;

fn main() {
    test();
}

struct Time {
    seconds: f32,
}

impl Resource for Time {}

fn test_system(r: Res<Time>) {
    println!("Seconds elapsed: {}", r.seconds);
}

fn empty_system() {}

fn test() {
    let mut world = World::new();
    world.resources.insert(Time { seconds: 42.0 });

    //Add a system that prints the entity index
    world.add_system(test_system);
    world.add_system(empty_system);

    // Run all systems
    for _ in 0..10 {
        world.get_resource_mut::<Time>().unwrap().seconds += 1.0;
        world.run_systems();
    }
}
