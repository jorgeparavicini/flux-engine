use crate::error::RendererError;
use ash::ext::debug_utils;
use ash::vk;
use flux_ecs::Single;
use flux_ecs::{Commands, Component};
use flux_renderer_api::settings::{AppInfo, RendererSettings};
use flux_renderer_api::surface::PresentTarget;
use log::{debug, error, info, warn};
use raw_window_handle::RawDisplayHandle;
use std::collections::HashSet;
use std::ffi::{CStr, CString, c_void};
use std::ops::Deref;

const VALIDATION_ENABLED: bool = cfg!(debug_assertions);
const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

#[derive(Component)]
#[component(non_send)]
pub struct Instance {
    entry: ash::Entry,
    raw: ash::Instance,
    debug: Option<(debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    pub extensions: InstanceExtensions,
}

impl Instance {
    pub fn new(
        display_handle: RawDisplayHandle,
        app_info: &AppInfo,
        _settings: &RendererSettings,
    ) -> Result<Self, RendererError> {
        info!("Creating the vulkan instance");
        let entry = ash::Entry::linked();

        let app_name = CString::new(app_info.name.as_str())
            .expect("application name must not contain NUL bytes");
        let engine_name = c"Flux Engine";
        let app_version = vk::make_api_version(
            0,
            app_info.version.major,
            app_info.version.minor,
            app_info.version.patch,
        );

        let app_info = vk::ApplicationInfo::default()
            .application_name(&app_name)
            .application_version(app_version)
            .engine_name(engine_name)
            .engine_version(vk::make_api_version(0, 1, 0, 0))
            .api_version(vk::make_api_version(0, 1, 3, 0));

        let data = unsafe { entry.enumerate_instance_layer_properties()? };

        let available_layers = data
            .iter()
            .map(|l| unsafe { CStr::from_ptr(l.layer_name.as_ptr()) })
            .collect::<HashSet<_>>();

        let validation = VALIDATION_ENABLED && available_layers.contains(&VALIDATION_LAYER);
        if VALIDATION_ENABLED && !validation {
            warn!("Validation layers requested but not available; continuing without them");
        }

        let enabled_layers = if validation {
            info!(
                "Enabling validation layers {}",
                VALIDATION_LAYER.to_str().unwrap()
            );
            vec![VALIDATION_LAYER.as_ptr()]
        } else {
            Vec::new()
        };

        let extensions = InstanceExtensions {
            debug_utils: validation,
            portability: cfg!(any(target_os = "macos", target_os = "ios")),
        };
        if extensions.portability {
            info!("Enabling apple portability extensions");
        }

        let mut extension_ptrs =
            ash_window::enumerate_required_extensions(display_handle)?.to_vec();
        extension_ptrs.extend(extensions.names().iter().map(|name| name.as_ptr()));

        let create_flags = if extensions.portability {
            vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR
        } else {
            vk::InstanceCreateFlags::default()
        };

        let mut create_info = vk::InstanceCreateInfo::default()
            .application_info(&app_info)
            .enabled_layer_names(&enabled_layers)
            .enabled_extension_names(&extension_ptrs)
            .flags(create_flags);

        let mut debug_info = get_debug_messenger_create_info();
        if extensions.debug_utils {
            create_info = create_info.push_next(&mut debug_info);
        }

        let instance: ash::Instance = unsafe { entry.create_instance(&create_info, None)? };

        let debug = if extensions.debug_utils {
            let loader = debug_utils::Instance::new(&entry, &instance);
            let messenger = unsafe { loader.create_debug_utils_messenger(&debug_info, None)? };
            Some((loader, messenger))
        } else {
            None
        };

        Ok(Self {
            entry,
            raw: instance,
            debug,
            extensions,
        })
    }

    pub const fn entry(&self) -> &ash::Entry {
        &self.entry
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct InstanceExtensions {
    pub debug_utils: bool,
    pub portability: bool,
}

impl InstanceExtensions {
    fn names(self) -> Vec<&'static CStr> {
        let mut names = Vec::new();
        if self.debug_utils {
            names.push(debug_utils::NAME);
        }
        if self.portability {
            names.push(ash::khr::portability_enumeration::NAME);
            names.push(ash::khr::get_physical_device_properties2::NAME);
        }
        names
    }
}

impl Deref for Instance {
    type Target = ash::Instance;

    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        info!("Destroying vulkan instance");
        if let Some((loader, messenger)) = self.debug.take() {
            unsafe {
                loader.destroy_debug_utils_messenger(messenger, None);
            }
        }

        unsafe {
            self.raw.destroy_instance(None);
        }
    }
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

pub fn create_instance(
    target: Single<&PresentTarget>,
    app_info: Single<&AppInfo>,
    settings: Single<&RendererSettings>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    let instance = Instance::new(target.display_handle(), &app_info, &settings)?;
    info!("Instance extensions enabled: {:?}", instance.extensions);
    commands.insert_singleton(instance);

    Ok(())
}

pub fn destroy_instance(mut commands: Commands) {
    commands.remove_singleton::<Instance>();
}
