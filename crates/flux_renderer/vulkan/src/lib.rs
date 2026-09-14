mod device;
mod pipeline;
mod present;
mod render;
mod resource;

pub use render::schedule::{render_schedule, setup, shutdown_schedule, startup_schedule};
