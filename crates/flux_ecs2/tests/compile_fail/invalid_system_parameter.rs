//! A function whose argument is not a system parameter is not a system; an
//! API accepting systems must reject it with an error naming the function
//! and suggesting the valid parameter kinds.
use flux_ecs2::IntoSystem;

fn accepts_systems<M>(_system: impl IntoSystem<M>) {}

fn not_a_system(_x: u32) {}

fn main() {
    accepts_systems(not_a_system);
}
