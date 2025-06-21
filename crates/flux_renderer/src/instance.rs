use anyhow::{bail, Result};
use ash::{vk, Instance};
use flux_engine_ecs::resource::{Res, Resource};
use log::info;
use raw_window_handle::RawDisplayHandle;
use std::collections::HashSet;
use std::ffi::CStr;

const VALIDATION_ENABLED: bool = cfg!(debug_assertions);
const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

pub trait SurfaceProvider {
    fn get_window_handle(&self) -> RawDisplayHandle;
}

pub struct SurfaceProviderResource {
    pub provider: Box<dyn SurfaceProvider>,
}

impl Resource for SurfaceProviderResource {}

pub struct AppVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

// TODO: These are more related to the application than the renderer, create a separate module for application settings!
pub struct RendererSettings {
    pub app_name: &'static str,
    pub app_version: AppVersion,
}

impl Resource for RendererSettings {}

pub struct VulkanInstance {
    pub(crate) entry: ash::Entry,
    pub(crate) instance: Instance,
}

impl Resource for VulkanInstance {}

impl Drop for VulkanInstance {
    fn drop(&mut self) {
        unsafe {
            self.instance.destroy_instance(None);
        }
    }
}

fn create_instance(
    surface_provider_resource: Res<SurfaceProviderResource>,
    renderer_settings: Option<Res<RendererSettings>>,
) -> Result<VulkanInstance> {
    let entry = ash::Entry::linked();

    // TODO: How do we make this configurable? As well as the application version?
    let app_name = renderer_settings.as_ref().map_or_else(
        || c"Flux Renderer",
        |settings| CStr::from_bytes_with_nul(settings.app_name.as_bytes())
            .expect("Invalid application name"),
    );
    let engine_name = c"Flux Engine";

    let app_version = renderer_settings.as_ref().map_or_else(
        || vk::make_api_version(0, 1, 0, 0),
        |settings| {
            vk::make_api_version(
                0,
                settings.app_version.major,
                settings.app_version.minor,
                settings.app_version.patch,
            )
        },
    );

    let app_info = vk::ApplicationInfo::default()
        .application_name(app_name)
        .application_version(app_version)
        .engine_name(engine_name)
        .engine_version(vk::make_api_version(0, 1, 0, 0))
        .api_version(vk::make_api_version(0, 1, 4, 0));

    let available_layers;
    unsafe {
        available_layers = entry
            .enumerate_instance_layer_properties()?
            .iter()
            .map(|l| l.layer_name.as_ptr())
            .collect::<HashSet<_>>();
    }

    if VALIDATION_ENABLED && !available_layers.contains(&VALIDATION_LAYER.as_ptr()) {
        bail!("Validation layers are not supported");
    }

    let enabled_layers = if VALIDATION_ENABLED {
        info!("Enabling validation layers {}", VALIDATION_LAYER.to_str()?);
        vec![VALIDATION_LAYER.as_ptr()]
    } else {
        Vec::new()
    };

    let mut extensions = ash_window::enumerate_required_extensions(
        surface_provider_resource.provider.get_window_handle(),
    )?
    .to_vec();

    if VALIDATION_ENABLED {
        extensions.push(VALIDATION_LAYER.as_ptr());
    }

    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        info!("Enabling apple portability extensions");
        extensions.push(ash::khr::portability_enumeration::NAME.as_ptr());
        extensions.push(ash::khr::get_physical_device_properties2::NAME.as_ptr());
    }

    let create_flags = if cfg!(any(target_os = "macos", target_os = "ios")) {
        vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR
    } else {
        vk::InstanceCreateFlags::default()
    };

    let create_info = vk::InstanceCreateInfo::default()
        .application_info(&app_info)
        .enabled_layer_names(&enabled_layers)
        .enabled_extension_names(&extensions)
        .flags(create_flags);

    let instance: Instance;
    unsafe {
        instance = entry
            .create_instance(&create_info, None)
            .expect("Failed to create Vulkan instance");
    }

    Ok(VulkanInstance { entry, instance })
}
