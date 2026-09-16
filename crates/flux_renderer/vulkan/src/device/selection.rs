use crate::device::capabilities::{
    CapabilityGates, DeviceCapabilities, DeviceFeatures, parse_disabled, resolve_gates,
};
use crate::device::instance::{Instance, MIN_SUPPORTED_API, format_version};
use crate::error::RendererError;
use ash::vk::{
    PhysicalDeviceFeatures2, PhysicalDeviceVulkan12Features, PhysicalDeviceVulkan13Features,
};
use ash::{khr, vk};
use log::{debug, info};
use std::ffi::CStr;
use std::fmt::Debug;
use std::ops::Deref;
use thiserror::Error;

pub const REQUIRED_DEVICE_EXTENSIONS: &[&CStr] = &[khr::swapchain::NAME];

/// The selected physical device and everything measured about it, produced
/// by [`select_physical_device`] and consumed by device creation.
pub struct PhysicalDevice {
    pub raw: vk::PhysicalDevice,
    /// Whether the device was selected for presenting to a surface.
    /// Presentation extensions are enabled at device creation only then.
    pub presentation: bool,
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

/// The queue family index chosen for each kind of work. Families may
/// coincide: on many devices one family serves all three roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueFamilyIndices {
    pub graphics: u32,
    pub present: u32,
    pub transfer: u32,
}

impl QueueFamilyIndices {
    /// Picks a family for each role: any graphics-capable family, any
    /// transfer-capable family (falling back to graphics), and the first
    /// family that can present to `surface`. Returns `None` when no
    /// combination covers all three.
    pub fn get(
        entry: &ash::Entry,
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        surface: Option<vk::SurfaceKHR>,
    ) -> Option<Self> {
        let properties = unsafe {
            let len = instance.get_physical_device_queue_family_properties2_len(physical_device);
            let mut properties = vec![vk::QueueFamilyProperties2::default(); len];
            instance.get_physical_device_queue_family_properties2(physical_device, &mut properties);
            properties
        };

        let flags: Vec<vk::QueueFlags> = properties
            .iter()
            .map(|p| p.queue_family_properties.queue_flags)
            .collect();

        let present_support: Vec<bool> = surface.map_or_else(
            || vec![false; properties.len()],
            |surface| {
                let loader = khr::surface::Instance::new(entry, instance);
                (0..properties.len())
                    .map(|index| {
                        matches!(
                            unsafe {
                                loader.get_physical_device_surface_support(
                                    physical_device,
                                    index as u32,
                                    surface,
                                )
                            },
                            Ok(true)
                        )
                    })
                    .collect()
            },
        );

        choose_families(&flags, &present_support, surface.is_some())
    }
}

/// Selects the best suitable physical device, logging why each rejected
/// candidate was passed over, and stores it as the [`PhysicalDevice`]
/// singleton. Fails with [`RendererError::NoSuitableDevice`] when no device
/// passes the floor.
/// Picks a family per role from the queried family table: any graphics
/// family, the first family that can present, and for transfer a family that
/// does nothing else where one exists, so uploads can overlap rendering. The
/// graphics family handles transfer otherwise.
fn choose_families(
    flags: &[vk::QueueFlags],
    present_support: &[bool],
    require_present: bool,
) -> Option<QueueFamilyIndices> {
    let graphics = flags
        .iter()
        .position(|f| f.contains(vk::QueueFlags::GRAPHICS))?;
    let present = if require_present {
        present_support.iter().position(|&supported| supported)?
    } else {
        graphics
    };
    let dedicated_transfer = flags.iter().position(|f| {
        f.contains(vk::QueueFlags::TRANSFER)
            && !f.contains(vk::QueueFlags::GRAPHICS)
            && !f.contains(vk::QueueFlags::COMPUTE)
    });
    let transfer = dedicated_transfer.unwrap_or(graphics);

    Some(QueueFamilyIndices {
        graphics: graphics as u32,
        present: present as u32,
        transfer: transfer as u32,
    })
}

/// Selects the best suitable physical device for `surface`, or for headless
/// use when no surface is given. Logs why each rejected candidate was passed
/// over and prints the capability report for the winner. Fails with
/// [`RendererError::NoSuitableDevice`] when no device passes the floor.
pub fn select_physical_device(
    instance: &Instance,
    surface: Option<vk::SurfaceKHR>,
) -> Result<PhysicalDevice, RendererError> {
    info!("Selecting a physical device");
    let disabled = parse_disabled(&std::env::var("FLUX_DISABLED_CAPABILITIES").unwrap_or_default());

    let mut best: Option<(u32, vk::PhysicalDevice, DeviceCapabilities, QueriedDevice)> = None;
    for physical_device in unsafe { instance.enumerate_physical_devices()? } {
        let queried = query_device(instance, physical_device, surface)?;
        match evaluate(&queried, instance.api_version(), surface.is_some()) {
            Ok((capabilities, score)) => {
                debug!("{} is suitable, score {score}", queried.name);
                if best.as_ref().is_none_or(|(top, ..)| score > *top) {
                    best = Some((score, physical_device, capabilities, queried));
                }
            }
            Err(rejection) => info!("Rejecting {}: {rejection}", queried.name),
        }
    }

    let Some((_, raw, capabilities, queried)) = best else {
        return Err(RendererError::NoSuitableDevice);
    };

    let physical_device = PhysicalDevice {
        raw,
        presentation: surface.is_some(),
        gates: resolve_gates(capabilities.features, &disabled),
        capabilities,
        formats: queried.formats,
        present_modes: queried.present_modes,
    };
    info!(
        "{}",
        crate::device::capabilities::report(&physical_device.capabilities, physical_device.gates)
    );
    Ok(physical_device)
}

/// Gathers everything [`evaluate`] needs from one device. All Vulkan queries
/// for selection happen here; no decisions do.
fn query_device(
    instance: &Instance,
    physical_device: vk::PhysicalDevice,
    surface: Option<vk::SurfaceKHR>,
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
            timeline_semaphore: vulkan_12_features.timeline_semaphore > 0,
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
    let (formats, present_modes) = match surface {
        Some(surface) => {
            let loader = khr::surface::Instance::new(instance.entry(), instance);
            (
                unsafe { loader.get_physical_device_surface_formats(physical_device, surface)? },
                unsafe {
                    loader.get_physical_device_surface_present_modes(physical_device, surface)?
                },
            )
        }
        None => (Vec::new(), Vec::new()),
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
    needs_presentation: bool,
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
    if needs_presentation {
        if !candidate.has_swapchain_extension {
            return Err(Rejection::MissingExtension("VK_KHR_swapchain"));
        }
        if candidate.formats.is_empty() || candidate.present_modes.is_empty() {
            return Err(Rejection::NoSurfaceSupport);
        }
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
                timeline_semaphore: true,
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
        evaluate(candidate, INSTANCE_API, true)
            .expect("candidate must be suitable")
            .1
    }

    // ── the floor: hard filters, each with a named rejection ───────────────

    #[test]
    fn a_healthy_device_is_accepted() {
        assert!(evaluate(&candidate(), INSTANCE_API, true).is_ok());
    }

    #[test]
    fn device_below_the_minimum_version_is_rejected() {
        let mut c = candidate();
        c.api_version = vk::make_api_version(0, 1, 2, 198);
        assert!(matches!(
            evaluate(&c, INSTANCE_API, true),
            Err(Rejection::BelowMinimumVersion { .. })
        ));
    }

    #[test]
    fn missing_dynamic_rendering_is_rejected() {
        let mut c = candidate();
        c.features.dynamic_rendering = false;
        assert!(matches!(
            evaluate(&c, INSTANCE_API, true),
            Err(Rejection::MissingFeature(_))
        ));
    }

    #[test]
    fn missing_queue_families_are_rejected() {
        let mut c = candidate();
        c.indices = None;
        assert!(matches!(
            evaluate(&c, INSTANCE_API, true),
            Err(Rejection::MissingQueueFamilies)
        ));
    }

    #[test]
    fn missing_swapchain_extension_is_rejected() {
        let mut c = candidate();
        c.has_swapchain_extension = false;
        assert!(matches!(
            evaluate(&c, INSTANCE_API, true),
            Err(Rejection::MissingExtension(_))
        ));
    }

    #[test]
    fn no_surface_formats_or_present_modes_is_rejected() {
        let mut c = candidate();
        c.formats.clear();
        assert!(matches!(
            evaluate(&c, INSTANCE_API, true),
            Err(Rejection::NoSurfaceSupport)
        ));

        let mut c = candidate();
        c.present_modes.clear();
        assert!(matches!(
            evaluate(&c, INSTANCE_API, true),
            Err(Rejection::NoSurfaceSupport)
        ));
    }

    // ── optional features degrade, they never reject ───────────────────────

    #[test]
    fn missing_optional_features_are_recorded_not_rejected() {
        let mut c = candidate();
        c.features.timeline_semaphore = false;
        c.features.buffer_device_address = false;
        let (capabilities, _) =
            evaluate(&c, INSTANCE_API, true).expect("optional features never reject");
        assert!(!capabilities.features.timeline_semaphore);
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
        assert!(evaluate(&c, INSTANCE_API, true).is_ok());
    }

    // ── choose_families: role assignment over the family table ──────────────

    const GFX: vk::QueueFlags = vk::QueueFlags::from_raw(
        vk::QueueFlags::GRAPHICS.as_raw()
            | vk::QueueFlags::COMPUTE.as_raw()
            | vk::QueueFlags::TRANSFER.as_raw(),
    );

    #[test]
    fn one_do_everything_family_serves_all_roles() {
        let chosen = choose_families(&[GFX], &[true], true).expect("suitable");
        assert_eq!(
            (chosen.graphics, chosen.present, chosen.transfer),
            (0, 0, 0)
        );
    }

    #[test]
    fn a_dedicated_transfer_family_is_preferred_over_graphics() {
        // Typical discrete GPU: family 0 does everything, family 1 is
        // transfer-only.
        let chosen = choose_families(&[GFX, vk::QueueFlags::TRANSFER], &[true, false], true)
            .expect("suitable");
        assert_eq!(chosen.graphics, 0);
        assert_eq!(chosen.transfer, 1, "transfer-only family wins");
    }

    #[test]
    fn a_compute_transfer_family_does_not_count_as_dedicated() {
        let compute_transfer = vk::QueueFlags::from_raw(
            vk::QueueFlags::COMPUTE.as_raw() | vk::QueueFlags::TRANSFER.as_raw(),
        );
        let chosen =
            choose_families(&[GFX, compute_transfer], &[true, false], true).expect("suitable");
        assert_eq!(chosen.transfer, 0, "async compute families are left alone");
    }

    #[test]
    fn no_present_capable_family_means_unsuitable() {
        assert!(choose_families(&[GFX], &[false], true).is_none());
    }

    // ── headless: presentation requirements are waived ──────────────────────

    #[test]
    fn headless_accepts_a_device_with_no_surface_support() {
        let mut c = candidate();
        c.has_swapchain_extension = false;
        c.formats.clear();
        c.present_modes.clear();
        assert!(
            evaluate(&c, INSTANCE_API, false).is_ok(),
            "surface checks apply only when presentation is needed"
        );
    }

    #[test]
    fn headless_family_choice_needs_no_present_family() {
        let chosen = choose_families(&[GFX], &[false], false).expect("suitable headless");
        assert_eq!(chosen.present, chosen.graphics);
    }

    // ── the record: measured values, not assumptions ───────────────────────

    #[test]
    fn effective_version_is_the_lower_of_device_and_instance() {
        let mut c = candidate();
        c.api_version = vk::make_api_version(0, 1, 4, 0);
        let (capabilities, _) = evaluate(&c, INSTANCE_API, true).expect("1.4 device is fine");
        assert_eq!(
            capabilities.api_version, INSTANCE_API,
            "a 1.4 device behind a 1.3 request is effectively 1.3"
        );
    }

    #[test]
    fn capabilities_carry_the_measured_identity() {
        let mut c = candidate();
        c.portability_subset = true;
        let (capabilities, _) = evaluate(&c, INSTANCE_API, true).expect("suitable");
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
