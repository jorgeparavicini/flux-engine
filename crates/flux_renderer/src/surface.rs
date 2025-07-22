use crate::instance::{SurfaceProviderResource, VulkanInstance};
use ash::khr::surface;
use ash::vk;
use flux_ecs::commands::Commands;
use flux_ecs::resource::{Res, Resource};
use log::info;
use std::ops::Deref;

pub struct VulkanSurface {
    pub surface: vk::SurfaceKHR,
}

impl Resource for VulkanSurface {}

impl Deref for VulkanSurface {
    type Target = vk::SurfaceKHR;

    fn deref(&self) -> &Self::Target {
        &self.surface
    }
}

pub fn create_surface(
    surface_provider_resource: Res<SurfaceProviderResource>,
    instance: Res<VulkanInstance>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    info!("Creating vulkan surface");
    let surface = unsafe {
        ash_window::create_surface(
            &instance.entry,
            &instance,
            surface_provider_resource.get_display_handle(),
            surface_provider_resource.get_window_handle(),
            None,
        )
    }?;

    commands.insert_resource(VulkanSurface { surface });

    Ok(())
}

pub fn destroy_surface(
    surface: Res<VulkanSurface>,
    instance: Res<VulkanInstance>,
    mut commands: Commands,
) {
    info!("Destroying vulkan surface");
    unsafe {
        let surface_loader = surface::Instance::new(&instance.entry, &instance);
        surface::Instance::destroy_surface(&surface_loader, **surface, None)
    }
    commands.remove_resource::<VulkanSurface>();
}
