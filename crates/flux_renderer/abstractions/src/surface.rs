use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use std::ops::Deref;

/// Window-system integration the renderer draws to, supplied by the
/// application before the render backend is initialized.
pub trait SurfaceProvider {
    fn get_display_handle(&self) -> RawDisplayHandle;

    fn get_window_handle(&self) -> RawWindowHandle;

    fn get_extent(&self) -> (u32, u32);

    fn request_redraw(&self);
}

#[derive(flux_ecs::Component)]
#[component(non_send)]
pub struct SurfaceProviderResource {
    pub provider: Box<dyn SurfaceProvider>,
}

impl Deref for SurfaceProviderResource {
    type Target = Box<dyn SurfaceProvider>;

    fn deref(&self) -> &Self::Target {
        &self.provider
    }
}
