//! A lifetime parameter also violates `Component: 'static` and shares a key
//! across instantiations. Same rule as type generics.
use flux_ecs2::Component;

#[derive(Component)]
struct Borrowed<'a>(&'a u32);

fn main() {}
