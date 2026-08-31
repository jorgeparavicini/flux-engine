//! Generic components are rejected for now: the KEY is derived from the type
//! *name*, so Wrapper<A> and Wrapper<B> would share a key — a silent
//! ComponentKey collision. Reject loudly until generic keys are designed.
use flux_ecs2::Component;

#[derive(Component)]
struct Wrapper<T>(T);

fn main() {}
