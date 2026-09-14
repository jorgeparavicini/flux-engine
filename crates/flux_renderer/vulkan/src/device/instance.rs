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

/// Oldest Vulkan version the renderer accepts. The frame path relies on 1.3
/// core features (dynamic rendering), and every supported platform provides
/// at least 1.3.
const MIN_SUPPORTED_API: u32 = vk::API_VERSION_1_3;

/// Newest Vulkan version the renderer requests. `apiVersion` is a promise
/// that the application copes with everything the newer core version
/// changes, so this is raised only after validating against the newer spec
/// and bindings.
const MAX_SUPPORTED_API: u32 = vk::API_VERSION_1_3;

const VALIDATION_ENABLED: bool = cfg!(debug_assertions);
const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

#[derive(Component)]
#[component(non_send)]
pub struct Instance {
    entry: ash::Entry,
    raw: ash::Instance,
    debug: Option<(debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    api_version: u32,
    pub extensions: InstanceExtensions,
}

/// The instance extensions actually enabled, after negotiation against what
/// the loader offers.
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
        }
        names
    }
}

impl Instance {
    pub const fn entry(&self) -> &ash::Entry {
        &self.entry
    }

    /// The Vulkan version requested at creation. Devices may support less;
    /// the effective version per device is the minimum of the two.
    pub const fn api_version(&self) -> u32 {
        self.api_version
    }

    pub fn new(
        display_handle: RawDisplayHandle,
        app: &AppInfo,
        _settings: &RendererSettings,
    ) -> Result<Self, RendererError> {
        info!("Creating the vulkan instance");
        let entry = ash::Entry::linked();

        let loader_version =
            unsafe { entry.try_enumerate_instance_version()? }.unwrap_or(vk::API_VERSION_1_0);
        let api_version = negotiate_api_version(loader_version)?;
        info!(
            "Vulkan loader {}, requesting {}",
            format_version(loader_version),
            format_version(api_version)
        );

        let validation = VALIDATION_ENABLED && available_layers(&entry)?.contains(VALIDATION_LAYER);
        if VALIDATION_ENABLED && !validation {
            warn!("Validation layers requested but not available; continuing without them");
        }

        let surface_extensions = surface_extensions(display_handle)?;
        let extensions = negotiate_extensions(
            &available_extensions(&entry, validation)?,
            &surface_extensions,
            cfg!(any(target_os = "macos", target_os = "ios")),
            validation,
        )?;
        if validation && !extensions.debug_utils {
            warn!("Debug utils extension unavailable; validation messages will not be captured");
        }

        let raw = create_raw_instance(
            &entry,
            app,
            api_version,
            validation,
            &surface_extensions,
            extensions,
        )?;

        let debug = if extensions.debug_utils {
            let loader = debug_utils::Instance::new(&entry, &raw);
            let messenger = unsafe {
                loader.create_debug_utils_messenger(&debug_messenger_create_info(), None)?
            };
            Some((loader, messenger))
        } else {
            None
        };

        Ok(Self {
            entry,
            raw,
            debug,
            api_version,
            extensions,
        })
    }
}

/// Clamps the loader's version into the renderer's supported range, rejecting
/// loaders below the minimum.
fn negotiate_api_version(loader_version: u32) -> Result<u32, RendererError> {
    if loader_version < MIN_SUPPORTED_API {
        return Err(RendererError::UnsupportedLoader {
            found: format_version(loader_version),
            minimum: format_version(MIN_SUPPORTED_API),
        });
    }
    Ok(loader_version.min(MAX_SUPPORTED_API))
}

/// Decides which extensions to enable given what the loader offers. Surface
/// extensions and, where demanded, portability enumeration are required;
/// debug utils degrades gracefully.
fn negotiate_extensions(
    available: &HashSet<CString>,
    surface_extensions: &[&'static CStr],
    portability_required: bool,
    debug_utils_wanted: bool,
) -> Result<InstanceExtensions, RendererError> {
    let mut missing: Vec<String> = surface_extensions
        .iter()
        .filter(|name| !available.contains(**name))
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    if portability_required && !available.contains(ash::khr::portability_enumeration::NAME) {
        missing.push(
            ash::khr::portability_enumeration::NAME
                .to_string_lossy()
                .into_owned(),
        );
    }
    if !missing.is_empty() {
        return Err(RendererError::MissingInstanceExtensions(missing));
    }

    Ok(InstanceExtensions {
        debug_utils: debug_utils_wanted && available.contains(debug_utils::NAME),
        portability: portability_required,
    })
}

fn available_layers(entry: &ash::Entry) -> Result<HashSet<CString>, RendererError> {
    let props = unsafe { entry.enumerate_instance_layer_properties()? };
    Ok(props
        .iter()
        .map(|l| unsafe { CStr::from_ptr(l.layer_name.as_ptr()) }.to_owned())
        .collect())
}

/// Extensions the loader and drivers offer. Layers contribute their own
/// extensions, so the validation layer's set is included when it is enabled.
fn available_extensions(
    entry: &ash::Entry,
    validation: bool,
) -> Result<HashSet<CString>, RendererError> {
    let mut available: HashSet<CString> =
        unsafe { entry.enumerate_instance_extension_properties(None)? }
            .iter()
            .map(|e| unsafe { CStr::from_ptr(e.extension_name.as_ptr()) }.to_owned())
            .collect();
    if validation {
        available.extend(
            unsafe { entry.enumerate_instance_extension_properties(Some(VALIDATION_LAYER))? }
                .iter()
                .map(|e| unsafe { CStr::from_ptr(e.extension_name.as_ptr()) }.to_owned()),
        );
    }
    Ok(available)
}

/// The window system's required surface extensions for this display.
fn surface_extensions(
    display_handle: RawDisplayHandle,
) -> Result<Vec<&'static CStr>, RendererError> {
    Ok(ash_window::enumerate_required_extensions(display_handle)?
        .iter()
        .map(|&ptr| unsafe { CStr::from_ptr(ptr) })
        .collect())
}

fn create_raw_instance(
    entry: &ash::Entry,
    app: &AppInfo,
    api_version: u32,
    validation: bool,
    surface_extensions: &[&'static CStr],
    extensions: InstanceExtensions,
) -> Result<ash::Instance, RendererError> {
    let app_name =
        CString::new(app.name.as_str()).expect("application name must not contain NUL bytes");
    let application_info = vk::ApplicationInfo::default()
        .application_name(&app_name)
        .application_version(vk::make_api_version(
            0,
            app.version.major,
            app.version.minor,
            app.version.patch,
        ))
        .engine_name(c"Flux Engine")
        .engine_version(vk::make_api_version(0, 1, 0, 0))
        .api_version(api_version);

    let enabled_layers = if validation {
        info!(
            "Enabling validation layers {}",
            VALIDATION_LAYER.to_str().unwrap()
        );
        vec![VALIDATION_LAYER.as_ptr()]
    } else {
        Vec::new()
    };

    let extension_ptrs: Vec<*const std::ffi::c_char> = surface_extensions
        .iter()
        .copied()
        .chain(extensions.names())
        .map(CStr::as_ptr)
        .collect();

    let create_flags = if extensions.portability {
        info!("Enabling apple portability enumeration");
        vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR
    } else {
        vk::InstanceCreateFlags::default()
    };

    let mut create_info = vk::InstanceCreateInfo::default()
        .application_info(&application_info)
        .enabled_layer_names(&enabled_layers)
        .enabled_extension_names(&extension_ptrs)
        .flags(create_flags);

    let mut debug_info = debug_messenger_create_info();
    if extensions.debug_utils {
        create_info = create_info.push_next(&mut debug_info);
    }

    Ok(unsafe { entry.create_instance(&create_info, None)? })
}

fn format_version(version: u32) -> String {
    format!(
        "{}.{}.{}",
        vk::api_version_major(version),
        vk::api_version_minor(version),
        vk::api_version_patch(version)
    )
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

fn debug_messenger_create_info<'a>() -> vk::DebugUtilsMessengerCreateInfoEXT<'a> {
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
    info!(
        "Instance created: Vulkan {}, extensions {:?}",
        format_version(instance.api_version()),
        instance.extensions
    );
    commands.insert_singleton(instance);

    Ok(())
}

pub fn destroy_instance(mut commands: Commands) {
    commands.remove_singleton::<Instance>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&CStr]) -> HashSet<CString> {
        names.iter().map(|&n| n.to_owned()).collect()
    }

    const SURFACE: &[&CStr] = &[ash::khr::surface::NAME, ash::khr::win32_surface::NAME];

    #[test]
    fn loader_above_maximum_is_clamped() {
        let requested = negotiate_api_version(vk::make_api_version(0, 1, 4, 0)).unwrap();
        assert_eq!(requested, MAX_SUPPORTED_API);
    }

    #[test]
    fn loader_at_minimum_is_accepted_verbatim() {
        let requested = negotiate_api_version(MIN_SUPPORTED_API).unwrap();
        assert_eq!(requested, MIN_SUPPORTED_API);
    }

    #[test]
    fn loader_below_minimum_is_rejected() {
        let result = negotiate_api_version(vk::make_api_version(0, 1, 2, 0));
        assert!(matches!(
            result,
            Err(RendererError::UnsupportedLoader { .. })
        ));
    }

    #[test]
    fn all_extensions_available_enables_everything() {
        let available = set(&[
            ash::khr::surface::NAME,
            ash::khr::win32_surface::NAME,
            ash::khr::portability_enumeration::NAME,
            debug_utils::NAME,
        ]);
        let extensions = negotiate_extensions(&available, SURFACE, true, true).unwrap();
        assert!(extensions.debug_utils);
        assert!(extensions.portability);
    }

    #[test]
    fn every_missing_required_extension_is_reported() {
        let result = negotiate_extensions(&set(&[]), SURFACE, true, false);
        let Err(RendererError::MissingInstanceExtensions(missing)) = result else {
            panic!("expected MissingInstanceExtensions");
        };
        assert_eq!(missing.len(), 3, "both surface extensions and portability");
    }

    #[test]
    fn missing_debug_utils_degrades_instead_of_failing() {
        let available = set(&[ash::khr::surface::NAME, ash::khr::win32_surface::NAME]);
        let extensions = negotiate_extensions(&available, SURFACE, false, true).unwrap();
        assert!(!extensions.debug_utils);
        assert!(!extensions.portability);
    }

    #[test]
    fn portability_is_required_where_demanded() {
        let available = set(&[ash::khr::surface::NAME, ash::khr::win32_surface::NAME]);
        let result = negotiate_extensions(&available, SURFACE, true, false);
        assert!(matches!(
            result,
            Err(RendererError::MissingInstanceExtensions(missing)) if missing.len() == 1
        ));
    }
}
