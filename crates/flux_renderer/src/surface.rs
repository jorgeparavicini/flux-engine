use crate::instance::{SurfaceProviderResource, VulkanInstance};
use flux_engine_ecs::resource::{Res, Resource};

struct VulkanSurface {
    pub surface: ash::vk::SurfaceKHR,
}

impl Resource for VulkanSurface {}

impl Drop for VulkanSurface {
    fn drop(&mut self) {
        unsafe {
            // TODO: Somehow get the instance here
        }
    }
}

fn create_surface(
    surface_provider_resource: Res<SurfaceProviderResource>,
    instance: Res<VulkanInstance>,
) -> Result<VulkanSurface> {
}
