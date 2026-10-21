use crate::device::capabilities::{CapabilityGates, DeviceCapabilities, DeviceFeatures};
use crate::device::debug::DebugUtils;
use crate::device::instance::Instance;
use crate::device::selection::{
    PhysicalDevice, QueueFamilyIndices, REQUIRED_DEVICE_EXTENSIONS, select_physical_device,
};
use crate::error::RendererError;
use crate::present::surface::VulkanSurface;
use ash::vk;
use flux_ecs::Commands;
use flux_ecs::Single;
use flux_renderer_api::settings::{AppInfo, RendererSettings};
use log::info;
use std::ops::Deref;

/// A device queue paired with the family it belongs to. Commands that care
/// about families (pool creation, queue ownership transfers) take both from
/// one place.
#[derive(Debug, Clone, Copy)]
pub struct Queue {
    pub raw: vk::Queue,
    pub family: u32,
}

/// The renderer's logical device: the ash device, its queues, and the
/// measured record it was created from.
///
/// Derefs to [`ash::Device`]. Destroys the device on drop, so it must
/// outlive every object created from it.
#[derive(flux_ecs::Component)]
pub struct RenderDevice {
    raw: ash::Device,
    pub physical: vk::PhysicalDevice,
    pub capabilities: DeviceCapabilities,
    pub gates: CapabilityGates,
    pub debug_utils: DebugUtils,
    pub graphics: Queue,
    pub present: Queue,
    pub transfer: Queue,
}

impl RenderDevice {
    /// Creates the logical device from a selected physical device, enabling
    /// exactly the features its record and gates allow.
    fn new(instance: &Instance, physical_device: &PhysicalDevice) -> Result<Self, RendererError> {
        info!(
            "Creating render device on {}",
            physical_device.capabilities.name
        );
        let mut enabled_features =
            enabled_features(physical_device.capabilities.features, physical_device.gates);
        let queue_families = unique_queue_families(physical_device.capabilities.queues);

        let queue_create_infos = queue_families
            .into_iter()
            .map(|queue_family| {
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(queue_family)
                    .queue_priorities(&[1.0])
            })
            .collect::<Vec<_>>();

        let mut features = vk::PhysicalDeviceFeatures2::default()
            .features(enabled_features.base)
            .push_next(&mut enabled_features.v12)
            .push_next(&mut enabled_features.v13);

        let mut extensions = Vec::new();
        if physical_device.presentation {
            extensions.extend(REQUIRED_DEVICE_EXTENSIONS.iter().map(|&e| e.as_ptr()));
        }
        if physical_device.capabilities.portability_subset {
            extensions.push(ash::khr::portability_subset::NAME.as_ptr());
        }

        let create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_create_infos)
            .enabled_extension_names(&extensions)
            .push_next(&mut features);

        let device = unsafe { instance.create_device(**physical_device, &create_info, None)? };
        let debug_utils = DebugUtils::new(instance, &device);
        debug_utils.set_name(device.handle(), "flux device");

        let queue = |family: u32| Queue {
            raw: unsafe { device.get_device_queue(family, 0) },
            family,
        };
        let graphics = queue(physical_device.capabilities.queues.graphics);
        let present = queue(physical_device.capabilities.queues.present);
        let transfer = queue(physical_device.capabilities.queues.transfer);

        Ok(Self {
            raw: device,
            physical: physical_device.raw,
            capabilities: physical_device.capabilities.clone(),
            gates: physical_device.gates,
            debug_utils,
            graphics,
            present,
            transfer,
        })
    }

    /// Creates a device with no surface attached, for tests and offscreen
    /// work: a headless instance, the best headless-suitable physical
    /// device, and a `RenderDevice` whose surface snapshot is empty. The
    /// returned instance must outlive the device.
    ///
    /// # Errors
    ///
    /// Fails when instance creation fails, no device passes the selection
    /// floor, or device creation fails.
    pub fn new_headless(
        app: &AppInfo,
        settings: &RendererSettings,
    ) -> Result<(Instance, Self), RendererError> {
        let instance = Instance::new_headless(app, settings)?;
        let physical_device = select_physical_device(&instance, None)?;
        let device = Self::new(&instance, &physical_device)?;
        Ok((instance, device))
    }

    /// Blocks until the device finishes all submitted work. Shutdown-path
    /// only; the frame path synchronizes with fences.
    pub fn wait_idle(&self) {
        if let Err(e) = unsafe { self.raw.device_wait_idle() } {
            log::error!("device_wait_idle failed: {e}");
        }
    }
}

impl Deref for RenderDevice {
    type Target = ash::Device;

    fn deref(&self) -> &Self::Target {
        &self.raw
    }
}

impl Drop for RenderDevice {
    fn drop(&mut self) {
        info!("Destroying render device");
        unsafe { self.raw.destroy_device(None) };
    }
}

/// The feature structs handed to device creation, derived from the measured
/// capabilities and resolved gates.
struct EnabledFeatures {
    base: vk::PhysicalDeviceFeatures,
    v12: vk::PhysicalDeviceVulkan12Features<'static>,
    v13: vk::PhysicalDeviceVulkan13Features<'static>,
}

fn enabled_features(features: DeviceFeatures, gates: CapabilityGates) -> EnabledFeatures {
    let base =
        vk::PhysicalDeviceFeatures::default().sampler_anisotropy(features.sampler_anisotropy);
    let v12 = vk::PhysicalDeviceVulkan12Features::default()
        .timeline_semaphore(features.timeline_semaphore && gates.timeline_semaphore)
        .buffer_device_address(features.buffer_device_address && gates.buffer_device_address)
        .descriptor_indexing(features.descriptor_indexing && gates.descriptor_indexing);
    let v13 = vk::PhysicalDeviceVulkan13Features::default()
        .dynamic_rendering(features.dynamic_rendering)
        .synchronization2(features.synchronization2 && gates.synchronization2);

    EnabledFeatures { base, v12, v13 }
}

/// The distinct queue family indices, sorted and deduplicated, so device
/// creation is deterministic and each family is created exactly once.
fn unique_queue_families(queues: QueueFamilyIndices) -> Vec<u32> {
    let mut families = vec![queues.graphics, queues.present, queues.transfer];
    families.sort_unstable();
    families.dedup();
    families
}

/// Selects the best physical device for the window surface and creates the
/// [`RenderDevice`] singleton from it.
pub fn create_render_device(
    instance: Single<&Instance>,
    surface: Single<&VulkanSurface>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    let physical_device = select_physical_device(&instance, Some(**surface))?;
    commands.insert_singleton(RenderDevice::new(&instance, &physical_device)?);
    Ok(())
}

/// Removes the [`RenderDevice`] singleton; destruction happens in its `Drop`.
pub fn destroy_render_device(mut commands: Commands) {
    commands.remove_singleton::<RenderDevice>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::capabilities::{CapabilityGates, DeviceFeatures};

    fn all_features() -> DeviceFeatures {
        DeviceFeatures {
            dynamic_rendering: true,
            synchronization2: true,
            timeline_semaphore: true,
            buffer_device_address: true,
            descriptor_indexing: true,
            sampler_anisotropy: true,
        }
    }

    fn all_gates() -> CapabilityGates {
        CapabilityGates {
            synchronization2: true,
            timeline_semaphore: true,
            buffer_device_address: true,
            descriptor_indexing: true,
        }
    }

    // ── enabled_features: the chain mirrors record + gates, nothing else ────

    #[test]
    fn floor_features_are_always_enabled() {
        let enabled = enabled_features(all_features(), CapabilityGates::default());
        assert_eq!(
            enabled.v13.dynamic_rendering,
            vk::TRUE,
            "dynamic rendering is floor: on even with every gate closed"
        );
    }

    #[test]
    fn gated_features_mirror_their_gate_not_their_support() {
        // Supported by the device, but every gate is closed (forced off).
        let enabled = enabled_features(all_features(), CapabilityGates::default());
        assert_eq!(enabled.v13.synchronization2, vk::FALSE);
        assert_eq!(enabled.v12.timeline_semaphore, vk::FALSE);
        assert_eq!(enabled.v12.buffer_device_address, vk::FALSE);
        assert_eq!(enabled.v12.descriptor_indexing, vk::FALSE);

        let enabled = enabled_features(all_features(), all_gates());
        assert_eq!(enabled.v13.synchronization2, vk::TRUE);
        assert_eq!(enabled.v12.timeline_semaphore, vk::TRUE);
        assert_eq!(enabled.v12.buffer_device_address, vk::TRUE);
        assert_eq!(enabled.v12.descriptor_indexing, vk::TRUE);
    }

    #[test]
    fn base_features_mirror_support() {
        let enabled = enabled_features(all_features(), all_gates());
        assert_eq!(enabled.base.sampler_anisotropy, vk::TRUE);

        let mut features = all_features();
        features.sampler_anisotropy = false;
        let enabled = enabled_features(features, all_gates());
        assert_eq!(
            enabled.base.sampler_anisotropy,
            vk::FALSE,
            "never enable what the device did not report"
        );
    }

    // ── unique_queue_families: deterministic device creation ────────────────

    #[test]
    fn one_family_for_everything_yields_one_entry() {
        let families = unique_queue_families(QueueFamilyIndices {
            graphics: 0,
            present: 0,
            transfer: 0,
        });
        assert_eq!(families, vec![0]);
    }

    #[test]
    fn distinct_families_are_all_present_sorted_and_unique() {
        let families = unique_queue_families(QueueFamilyIndices {
            graphics: 2,
            present: 0,
            transfer: 2,
        });
        assert_eq!(families, vec![0, 2], "sorted and deduplicated");
    }
}
