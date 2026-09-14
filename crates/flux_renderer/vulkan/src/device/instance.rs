use crate::error::RendererError;
use ash::ext::debug_utils;
use ash::vk::DebugUtilsMessengerEXT;
use ash::{Instance, vk};
use flux_ecs::Commands;
use flux_ecs::Single;
use flux_renderer_api::surface::PresentTarget;
use log::{debug, error, info, warn};
use std::collections::HashSet;
use std::ffi::{CStr, c_void};
use std::ops::Deref;

const VALIDATION_ENABLED: bool = cfg!(debug_assertions);
const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

pub struct AppVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

// TODO: These are more related to the application than the renderer, create a separate module for application settings!
#[derive(flux_ecs::Component)]
pub struct RendererSettings {
    pub app_name: &'static str,
    pub app_version: AppVersion,
}

#[derive(flux_ecs::Component)]
pub struct VulkanInstance {
    pub(crate) entry: ash::Entry,
    pub(crate) instance: Instance,
    debug_messenger: Option<DebugUtilsMessengerEXT>,
}

impl Deref for VulkanInstance {
    type Target = Instance;

    fn deref(&self) -> &Self::Target {
        &self.instance
    }
}

pub fn create_instance(
    surface_provider_resource: Single<&PresentTarget>,
    renderer_settings: Option<Single<&RendererSettings>>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    info!("Creating the vulkan instance");
    let entry = ash::Entry::linked();

    // TODO: How do we make this configurable? As well as the application version?
    let app_name = renderer_settings.as_ref().map_or_else(
        || c"Flux Renderer",
        |settings| {
            CStr::from_bytes_with_nul(settings.app_name.as_bytes())
                .expect("Invalid application name")
        },
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
        .api_version(vk::make_api_version(0, 1, 3, 0));

    let data = unsafe { entry.enumerate_instance_layer_properties()? };

    let available_layers = data
        .iter()
        .map(|l| unsafe { CStr::from_ptr(l.layer_name.as_ptr()) })
        .collect::<HashSet<_>>();

    if VALIDATION_ENABLED && !available_layers.contains(&VALIDATION_LAYER) {
        error!("Validation layers are not available");
    }

    let enabled_layers = if VALIDATION_ENABLED {
        info!(
            "Enabling validation layers {}",
            VALIDATION_LAYER.to_str().unwrap()
        );
        vec![VALIDATION_LAYER.as_ptr()]
    } else {
        Vec::new()
    };

    let mut extensions = ash_window::enumerate_required_extensions(
        surface_provider_resource.provider.display_handle(),
    )?
    .to_vec();

    if VALIDATION_ENABLED {
        extensions.push(debug_utils::NAME.as_ptr());
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

    let mut create_info = vk::InstanceCreateInfo::default()
        .application_info(&app_info)
        .enabled_layer_names(&enabled_layers)
        .enabled_extension_names(&extensions)
        .flags(create_flags);

    let mut debug_info = get_debug_messenger_create_info();
    if VALIDATION_ENABLED {
        create_info = create_info.push_next(&mut debug_info);
    }

    let instance: Instance = unsafe { entry.create_instance(&create_info, None)? };

    let mut debug_messenger = None;
    if VALIDATION_ENABLED {
        let debug_utils_loader = debug_utils::Instance::new(&entry, &instance);
        debug_messenger =
            unsafe { Some(debug_utils_loader.create_debug_utils_messenger(&debug_info, None)?) };
    }

    commands.insert_singleton(VulkanInstance {
        entry,
        instance,
        debug_messenger,
    });

    Ok(())
}

fn get_debug_messenger_create_info<'a>() -> vk::DebugUtilsMessengerCreateInfoEXT<'a> {
    vk::DebugUtilsMessengerCreateInfoEXT::default()
        .message_severity(
            vk::DebugUtilsMessageSeverityFlagsEXT::VERBOSE
                | vk::DebugUtilsMessageSeverityFlagsEXT::INFO
                | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
        )
        .message_type(
            vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
        )
        .pfn_user_callback(Some(debug_callback))
}

extern "system" fn debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    type_: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT,
    _: *mut c_void,
) -> vk::Bool32 {
    let data = unsafe { *data };
    let message_id_name = unsafe { CStr::from_ptr(data.p_message_id_name).to_string_lossy() };
    let message_id_number = data.message_id_number;
    let message = unsafe { CStr::from_ptr(data.p_message).to_string_lossy() };

    if severity == vk::DebugUtilsMessageSeverityFlagsEXT::VERBOSE {
        debug!("{type_:?} [{message_id_name} ({message_id_number})]: {message}");
    } else if severity == vk::DebugUtilsMessageSeverityFlagsEXT::INFO {
        info!("{type_:?} [{message_id_name} ({message_id_number})]: {message}");
    } else if severity == vk::DebugUtilsMessageSeverityFlagsEXT::WARNING {
        warn!("{type_:?} [{message_id_name} ({message_id_number})]: {message}");
    } else if severity == vk::DebugUtilsMessageSeverityFlagsEXT::ERROR {
        error!("{type_:?} [{message_id_name} ({message_id_number})]: {message}");
    }

    vk::FALSE
}

pub fn destroy_instance(instance: Single<&VulkanInstance>, mut commands: Commands) {
    info!("Destroying vulkan instance");
    if let Some(debug_messenger) = instance.debug_messenger {
        unsafe {
            let debug_utils_loader = debug_utils::Instance::new(&instance.entry, &instance);
            debug_utils_loader.destroy_debug_utils_messenger(debug_messenger, None);
        }
    }

    unsafe {
        instance.destroy_instance(None);
    }

    commands.remove_singleton::<VulkanInstance>();
}
