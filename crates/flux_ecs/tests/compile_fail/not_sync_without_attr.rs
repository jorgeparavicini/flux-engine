//! Send but !Sync (Cell) must also be rejected — the assertion is Send + Sync,
//! not just Send.
use flux_ecs::Component;
use std::cell::Cell;

#[derive(Component)]
struct Counter(Cell<u32>);

fn main() {}
