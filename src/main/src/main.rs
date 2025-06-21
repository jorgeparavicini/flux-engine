use flux_engine_ecs::component::Component;
use flux_engine_ecs::query::Query;
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

#[derive(Debug)]
struct TestComponent {}

impl Component for TestComponent {}

fn query_system(q: Query<&TestComponent>) {
    for item in q {
        println!("Querying TestComponent: {item:?}");
    }
}

fn test() {
    let mut world = World::new();
    world.insert_resource(Time { seconds: 42.0 });
    world.spawn((TestComponent {},));

    //Add a system that prints the entity index
    world.add_system(test_system);
    world.add_system(empty_system);
    world.add_system(query_system);

    // Run all systems
    for _ in 0..10 {
        world.get_resource_mut::<Time>().unwrap().seconds += 1.0;
        world.run_systems();
    }
}
