use crate::device::capabilities::{
    CapabilityGates, DeviceCapabilities, DeviceFeatures, parse_disabled, resolve_gates,
};
use crate::device::instance::{Instance, MIN_SUPPORTED_API, format_version};
use crate::error::RendererError;
use crate::present::surface::VulkanSurface;
use ash::vk::{
    PhysicalDeviceFeatures2, PhysicalDeviceVulkan12Features, PhysicalDeviceVulkan13Features,
};
use ash::{khr, vk};
use flux_ecs::Commands;
use flux_ecs::Single;
use log::{debug, info};
use std::ffi::CStr;
use std::fmt::Debug;
use std::ops::Deref;
use thiserror::Error;

pub const REQUIRED_DEVICE_EXTENSIONS: &[&CStr] = &[khr::swapchain::NAME];

#[derive(flux_ecs::Component)]
pub struct PhysicalDevice {
    pub raw: vk::PhysicalDevice,
    pub capabilities: DeviceCapabilities,
    pub gates: CapabilityGates,
    pub formats: Vec<vk::SurfaceFormatKHR>,
    pub present_modes: Vec<vk::PresentModeKHR>,
}

impl Debug for PhysicalDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhysicalDevice")
            .field("name", &self.capabilities.name)
            .finish_non_exhaustive()
    }
}

impl Deref for PhysicalDevice {
    type Target = vk::PhysicalDevice;

    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

struct QueriedDevice {
    name: String,
    device_type: vk::PhysicalDeviceType,
    api_version: u32,
    features: DeviceFeatures,
    portability_subset: bool,
    indices: Option<QueueFamilyIndices>,
    has_swapchain_extension: bool,
    formats: Vec<vk::SurfaceFormatKHR>,
    present_modes: Vec<vk::PresentModeKHR>,
    memory: vk::PhysicalDeviceMemoryProperties,
    limits: vk::PhysicalDeviceLimits,
}

#[derive(Debug, Error)]
enum Rejection {
    #[error("supports Vulkan {}, below the renderer's minimum {}",
        format_version(*.version),
        format_version(MIN_SUPPORTED_API)
    )]
    BelowMinimumVersion { version: u32 },
    #[error("missing required feature {0}")]
    MissingFeature(&'static str),
    #[error("no queue family combination covers graphics, present, and transfer")]
    MissingQueueFamilies,
    #[error("missing required device extension {0}")]
    MissingExtension(&'static str),
    #[error("the surface offers no formats or present modes on this device")]
    NoSurfaceSupport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFamilyIndices {
    pub graphics: u32,
    pub present: u32,
    pub transfer: u32,
}

impl QueueFamilyIndices {
    pub fn get(
        entry: &ash::Entry,
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
    ) -> Option<Self> {
        let properties = unsafe {
            let len = instance.get_physical_device_queue_family_properties2_len(physical_device);
            let mut properties = vec![vk::QueueFamilyProperties2::default(); len];
            instance.get_physical_device_queue_family_properties2(physical_device, &mut properties);
            properties
        };

        let graphics = properties.iter().position(|p| {
            p.queue_family_properties
                .queue_flags
                .contains(vk::QueueFlags::GRAPHICS)
        })?;

        let transfer = properties
            .iter()
            .position(|p| {
                p.queue_family_properties
                    .queue_flags
                    .contains(vk::QueueFlags::TRANSFER)
            })
            .unwrap_or(graphics); // The graphics queue can also handle transfers

        let surface_loader = khr::surface::Instance::new(entry, instance);
        let present = properties
            .iter()
            .enumerate()
            .map(|(index, _)| unsafe {
                surface_loader
                    .get_physical_device_surface_support(physical_device, index as u32, surface)
                    .ok()
            })
            .position(|index| index.is_some())?;

        Some(Self {
            graphics: graphics as u32,
            present: present as u32,
            transfer: transfer as u32,
        })
    }
}

pub fn create_physical_device(
    instance: Single<&Instance>,
    surface: Single<&VulkanSurface>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    info!("Selecting a physical device");
    let disabled = parse_disabled(&std::env::var("FLUX_DISABLED_CAPABILITIES").unwrap_or_default());

    let mut best: Option<(u32, vk::PhysicalDevice, DeviceCapabilities, QueriedDevice)> = None;
    for physical_device in unsafe { instance.enumerate_physical_devices()? } {
        let queried = query_device(&instance, physical_device, **surface)?;
        match evaluate(&queried, instance.api_version()) {
            Ok((capabilities, score)) => {
                debug!("{} is suitable, score {score}", queried.name);
                if best.as_ref().is_none_or(|(top, ..)| score > *top) {
                    best = Some((score, physical_device, capabilities, queried));
                }
            }
            Err(rejection) => info!("Rejecting {}: {rejection}", queried.name),
        }
    }

    let Some((_, physical_device, capabilities, queried)) = best else {
        return Err(RendererError::NoSuitableDevice);
    };

    let physical_device = PhysicalDevice {
        raw: physical_device,
        gates: resolve_gates(capabilities.features, &disabled),
        capabilities,
        formats: queried.formats,
        present_modes: queried.present_modes,
    };
    info!(
        "{}",
        crate::device::capabilities::report(&physical_device.capabilities, physical_device.gates)
    );
    commands.insert_singleton(physical_device);

    Ok(())
}

/// Gathers everything [`evaluate`] needs from one device. All Vulkan queries
/// for selection happen here; no decisions do.
fn query_device(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
) -> Result<QueriedDevice, RendererError> {
    let properties = unsafe { instance.get_physical_device_properties(physical_device) };
    let features = if properties.api_version >= MIN_SUPPORTED_API {
        let mut vulkan_12_features = PhysicalDeviceVulkan12Features::default();
        let mut vulkan_13_features = PhysicalDeviceVulkan13Features::default();
        let mut device_features = PhysicalDeviceFeatures2::default()
            .push_next(&mut vulkan_12_features)
            .push_next(&mut vulkan_13_features);
        unsafe { instance.get_physical_device_features2(physical_device, &mut device_features) };

        DeviceFeatures {
            sampler_anisotropy: device_features.features.sampler_anisotropy > 0,
            descriptor_indexing: vulkan_12_features.descriptor_indexing > 0,
            buffer_device_address: vulkan_12_features.buffer_device_address > 0,
            timeline_semaphores: vulkan_12_features.timeline_semaphore > 0,
            dynamic_rendering: vulkan_13_features.dynamic_rendering > 0,
            synchronization2: vulkan_13_features.synchronization2 > 0,
        }
    } else {
        DeviceFeatures::default()
    };

    let extension_properties =
        unsafe { instance.enumerate_device_extension_properties(physical_device)? };
    let extension_property_names = extension_properties
        .iter()
        .map(|p| {
            p.extension_name_as_c_str()
                .unwrap_or(c"<unknown extension property>")
        })
        .collect::<Vec<_>>();
    let indices = QueueFamilyIndices::get(instance.entry(), instance, physical_device, surface);
    let surface_instance = khr::surface::Instance::new(instance.entry(), instance);
    let formats =
        unsafe { surface_instance.get_physical_device_surface_formats(physical_device, surface)? };
    let present_modes = unsafe {
        surface_instance.get_physical_device_surface_present_modes(physical_device, surface)?
    };
    let memory = unsafe { instance.get_physical_device_memory_properties(physical_device) };

    Ok(QueriedDevice {
        name: properties
            .device_name_as_c_str()
            .unwrap_or(c"<unnamed device>")
            .to_string_lossy()
            .into_owned(),
        device_type: properties.device_type,
        api_version: properties.api_version,
        limits: properties.limits,
        features,
        portability_subset: extension_property_names.contains(&khr::portability_subset::NAME),
        has_swapchain_extension: REQUIRED_DEVICE_EXTENSIONS
            .iter()
            .all(|name| extension_property_names.contains(name)),
        indices,
        formats,
        present_modes,
        memory,
    })
}

/// The floor filter and preference scoring. Pure: the tests are its contract.
fn evaluate(
    candidate: &QueriedDevice,
    instance_api: u32,
) -> Result<(DeviceCapabilities, u32), Rejection> {
    let effective_api = candidate.api_version.min(instance_api);
    if effective_api < MIN_SUPPORTED_API {
        return Err(Rejection::BelowMinimumVersion {
            version: candidate.api_version,
        });
    }
    if !candidate.features.dynamic_rendering {
        return Err(Rejection::MissingFeature("dynamic_rendering"));
    }
    let Some(queues) = candidate.indices else {
        return Err(Rejection::MissingQueueFamilies);
    };
    if !candidate.has_swapchain_extension {
        return Err(Rejection::MissingExtension("VK_KHR_swapchain"));
    }
    if candidate.formats.is_empty() || candidate.present_modes.is_empty() {
        return Err(Rejection::NoSurfaceSupport);
    }

    let mut score = match candidate.device_type {
        vk::PhysicalDeviceType::DISCRETE_GPU => 1000,
        vk::PhysicalDeviceType::INTEGRATED_GPU => 500,
        vk::PhysicalDeviceType::VIRTUAL_GPU => 250,
        vk::PhysicalDeviceType::CPU => 100,
        _ => 0,
    };
    if queues.transfer != queues.graphics {
        score += 50;
    }

    let capabilities = DeviceCapabilities {
        name: candidate.name.clone(),
        device_type: candidate.device_type,
        api_version: effective_api,
        features: candidate.features,
        portability_subset: candidate.portability_subset,
        memory: candidate.memory,
        queues,
        max_bound_descriptor_sets: candidate.limits.max_bound_descriptor_sets,
        max_sampler_anisotropy: candidate.limits.max_sampler_anisotropy,
    };

    Ok((capabilities, score))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::capabilities::DeviceFeatures;

    const INSTANCE_API: u32 = vk::API_VERSION_1_3;

    /// A healthy 1.3 discrete GPU; individual tests break one thing at a time.
    fn candidate() -> QueriedDevice {
        QueriedDevice {
            name: "test gpu".to_owned(),
            device_type: vk::PhysicalDeviceType::DISCRETE_GPU,
            api_version: vk::API_VERSION_1_3,
            features: DeviceFeatures {
                dynamic_rendering: true,
                synchronization2: true,
                timeline_semaphores: true,
                buffer_device_address: true,
                descriptor_indexing: true,
                sampler_anisotropy: true,
            },
            portability_subset: false,
            indices: Some(QueueFamilyIndices {
                graphics: 0,
                present: 0,
                transfer: 1,
            }),
            has_swapchain_extension: true,
            formats: vec![vk::SurfaceFormatKHR::default()],
            present_modes: vec![vk::PresentModeKHR::FIFO],
            memory: vk::PhysicalDeviceMemoryProperties::default(),
            limits: vk::PhysicalDeviceLimits::default(),
        }
    }

    fn score_of(candidate: &QueriedDevice) -> u32 {
        evaluate(candidate, INSTANCE_API)
            .expect("candidate must be suitable")
            .1
    }

    // ── the floor: hard filters, each with a named rejection ───────────────

    #[test]
    fn a_healthy_device_is_accepted() {
        assert!(evaluate(&candidate(), INSTANCE_API).is_ok());
    }

    #[test]
    fn device_below_the_minimum_version_is_rejected() {
        let mut c = candidate();
        c.api_version = vk::make_api_version(0, 1, 2, 198);
        assert!(matches!(
            evaluate(&c, INSTANCE_API),
            Err(Rejection::BelowMinimumVersion { .. })
        ));
    }

    #[test]
    fn missing_dynamic_rendering_is_rejected() {
        let mut c = candidate();
        c.features.dynamic_rendering = false;
        assert!(matches!(
            evaluate(&c, INSTANCE_API),
            Err(Rejection::MissingFeature(_))
        ));
    }

    #[test]
    fn missing_queue_families_are_rejected() {
        let mut c = candidate();
        c.indices = None;
        assert!(matches!(
            evaluate(&c, INSTANCE_API),
            Err(Rejection::MissingQueueFamilies)
        ));
    }

    #[test]
    fn missing_swapchain_extension_is_rejected() {
        let mut c = candidate();
        c.has_swapchain_extension = false;
        assert!(matches!(
            evaluate(&c, INSTANCE_API),
            Err(Rejection::MissingExtension(_))
        ));
    }

    #[test]
    fn no_surface_formats_or_present_modes_is_rejected() {
        let mut c = candidate();
        c.formats.clear();
        assert!(matches!(
            evaluate(&c, INSTANCE_API),
            Err(Rejection::NoSurfaceSupport)
        ));

        let mut c = candidate();
        c.present_modes.clear();
        assert!(matches!(
            evaluate(&c, INSTANCE_API),
            Err(Rejection::NoSurfaceSupport)
        ));
    }

    // ── optional features degrade, they never reject ───────────────────────

    #[test]
    fn missing_optional_features_are_recorded_not_rejected() {
        let mut c = candidate();
        c.features.timeline_semaphores = false;
        c.features.buffer_device_address = false;
        let (capabilities, _) = evaluate(&c, INSTANCE_API).expect("optional features never reject");
        assert!(!capabilities.features.timeline_semaphores);
        assert!(!capabilities.features.buffer_device_address);
        assert!(capabilities.features.dynamic_rendering);
    }

    // ── scoring: preferences, not filters ──────────────────────────────────

    #[test]
    fn discrete_outscores_integrated_outscores_software() {
        let discrete = score_of(&candidate());

        let mut c = candidate();
        c.device_type = vk::PhysicalDeviceType::INTEGRATED_GPU;
        let integrated = score_of(&c);

        let mut c = candidate();
        c.device_type = vk::PhysicalDeviceType::CPU;
        let software = score_of(&c);

        assert!(discrete > integrated, "discrete beats integrated");
        assert!(integrated > software, "integrated beats software");
    }

    #[test]
    fn a_software_rasterizer_is_still_acceptable() {
        // lavapipe in CI reports PhysicalDeviceType::CPU; it must pass the floor.
        let mut c = candidate();
        c.device_type = vk::PhysicalDeviceType::CPU;
        assert!(evaluate(&c, INSTANCE_API).is_ok());
    }

    // ── the record: measured values, not assumptions ───────────────────────

    #[test]
    fn effective_version_is_the_lower_of_device_and_instance() {
        let mut c = candidate();
        c.api_version = vk::make_api_version(0, 1, 4, 0);
        let (capabilities, _) = evaluate(&c, INSTANCE_API).expect("1.4 device is fine");
        assert_eq!(
            capabilities.api_version, INSTANCE_API,
            "a 1.4 device behind a 1.3 request is effectively 1.3"
        );
    }

    #[test]
    fn capabilities_carry_the_measured_identity() {
        let mut c = candidate();
        c.portability_subset = true;
        let (capabilities, _) = evaluate(&c, INSTANCE_API).expect("suitable");
        assert_eq!(capabilities.name, "test gpu");
        assert!(capabilities.portability_subset);
        assert_eq!(
            capabilities.queues,
            QueueFamilyIndices {
                graphics: 0,
                present: 0,
                transfer: 1,
            }
        );
    }
}
