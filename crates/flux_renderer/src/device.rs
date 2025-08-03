use crate::instance::VulkanInstance;
use crate::surface::VulkanSurface;
use ash::{khr, vk};
use flux_ecs::commands::Commands;
use flux_ecs::resource::{Res, Resource};
use log::{debug, info};
use std::collections::HashSet;
use std::ffi::CStr;
use std::fmt::{Debug, Display};
use std::ops::Deref;
use thiserror::Error;

// TODO: This should probably not be called `DeviceRequirements` as it is also used to create the logical device
#[derive(Debug, Clone)]
pub struct DeviceRequirements {
    pub extensions: Vec<&'static CStr>,
    pub prefer_discrete_gpu: bool,
}

impl Default for DeviceRequirements {
    fn default() -> Self {
        Self {
            extensions: vec![
                khr::swapchain::NAME,
                #[cfg(any(target_os = "macos", target_os = "ios"))]
                khr::portability_subset::NAME,
            ],
            prefer_discrete_gpu: true,
        }
    }
}

impl Resource for DeviceRequirements {}

#[derive(Error, Debug)]
pub enum SuitabilityError {
    #[error("device {device:?} does not support required queue family: {queue_family:?}")]
    MissingQueueFamily {
        device: vk::PhysicalDevice,
        queue_family: &'static str,
    },
    #[error("device {device:?} does not support required device extension: {extension:?}")]
    MissingDeviceExtension {
        device: vk::PhysicalDevice,
        extension: &'static CStr,
    },
    #[error("could not get device extensions for device {device:?}")]
    DeviceExtensionsNotFound { device: vk::PhysicalDevice },
    #[error("device {device:?} does not support required feature {feature:?}")]
    MissingDeviceFeatures {
        device: vk::PhysicalDevice,
        feature: &'static str,
    },
    #[error("device {device:?} does not support required swapchain surface: {surface:?}")]
    SurfaceNotSupported {
        device: vk::PhysicalDevice,
        surface: vk::SurfaceKHR,
    },
}

#[derive(Error, Debug)]
pub struct NoPhysicalDevicesFoundError;

impl Display for NoPhysicalDevicesFoundError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "No suitable physical device found")
    }
}

pub struct PhysicalDevice {
    pub physical_device: vk::PhysicalDevice,
    pub indices: QueueFamilyIndices,
    pub properties: vk::PhysicalDeviceProperties,
    pub features: vk::PhysicalDeviceFeatures,
    pub capabilities: vk::SurfaceCapabilitiesKHR,
    pub formats: Vec<vk::SurfaceFormatKHR>,
    pub present_modes: Vec<vk::PresentModeKHR>,
}

impl Debug for PhysicalDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PhysicalDevice")
            .field(
                "physical_device",
                &unsafe { CStr::from_ptr(self.properties.device_name.as_ptr()) }
                    .to_str()
                    .unwrap_or("Unknown Device"),
            )
            .field("indices", &self.indices)
            .finish()
    }
}

impl Resource for PhysicalDevice {}

impl Deref for PhysicalDevice {
    type Target = vk::PhysicalDevice;

    fn deref(&self) -> &Self::Target {
        &self.physical_device
    }
}

pub fn create_physical_device(
    instance: Res<VulkanInstance>,
    surface: Res<VulkanSurface>,
    device_requirements: Option<Res<DeviceRequirements>>,
    mut commands: Commands,
) -> Result<(), NoPhysicalDevicesFoundError> {
    info!("Selecting a physical device");
    let physical_devices = unsafe {
        instance
            .enumerate_physical_devices()
            .or(Err(NoPhysicalDevicesFoundError))?
    };

    let device_requirements = device_requirements
        .map(|res| res.into_inner())
        .unwrap_or_default();

    let best_device_evaluation = physical_devices
        .iter()
        .map(|&device| {
            evaluate_physical_device(
                &instance.entry,
                &instance,
                device,
                **surface,
                &device_requirements,
            )
        })
        .filter_map(|evaluation| match evaluation {
            Ok(evaluation) => Some(evaluation),
            Err(err) => {
                debug!("Physical device is not suitable: {err}");
                None
            }
        })
        .max_by_key(|evaluation| evaluation.score)
        .ok_or(NoPhysicalDevicesFoundError)?;

    info!(
        "Best physical device found: {0:?}",
        unsafe {
            CStr::from_ptr(
                best_device_evaluation
                    .physical_device
                    .properties
                    .device_name
                    .as_ptr(),
            )
        }
        .to_str()
        .unwrap()
    );

    commands.insert_resource(best_device_evaluation.physical_device);

    Ok(())
}

struct DeviceEvaluation {
    score: u32,
    physical_device: PhysicalDevice,
}

fn evaluate_physical_device(
    entry: &ash::Entry,
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
    device_requirements: &DeviceRequirements,
) -> Result<DeviceEvaluation, SuitabilityError> {
    let properties = unsafe {
        let mut properties = vk::PhysicalDeviceProperties2::default();
        instance.get_physical_device_properties2(physical_device, &mut properties);
        properties.properties
    };

    debug!(
        "Checking suitability of physical device: {0:}",
        unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
            .to_str()
            .unwrap_or("Unknown Device")
    );

    let indices = QueueFamilyIndices::get(entry, instance, physical_device, surface)?;
    check_required_device_extensions(instance, physical_device, &device_requirements.extensions)?;
    let features = query_required_features(instance, physical_device)?;
    let (capabilities, formats, present_modes) =
        query_swapchain_support(entry, instance, physical_device, surface)?;

    let score = get_physical_device_score(&properties, &indices, device_requirements);

    let physical_device = PhysicalDevice {
        features,
        physical_device,
        properties,
        indices,
        capabilities,
        formats,
        present_modes,
    };

    Ok(DeviceEvaluation {
        score,
        physical_device,
    })
}

fn check_required_device_extensions(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    required_extensions: &Vec<&'static CStr>,
) -> Result<(), SuitabilityError> {
    let available_extensions = unsafe {
        instance
            .enumerate_device_extension_properties(physical_device)
            .or(Err(SuitabilityError::DeviceExtensionsNotFound {
                device: physical_device,
            }))?
            .iter()
            .map(|ext| CStr::from_ptr(ext.extension_name.as_ptr()))
            .collect::<HashSet<_>>()
    };

    for required_extension in required_extensions {
        if !available_extensions.contains(required_extension) {
            return Err(SuitabilityError::MissingDeviceExtension {
                device: physical_device,
                extension: required_extension,
            });
        }
    }

    Ok(())
}

fn query_required_features(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
) -> Result<vk::PhysicalDeviceFeatures, SuitabilityError> {
    let features = unsafe {
        let mut features = vk::PhysicalDeviceFeatures2::default();
        instance.get_physical_device_features2(physical_device, &mut features);
        features
    };

    if features.features.sampler_anisotropy != vk::TRUE {
        return Err(SuitabilityError::MissingDeviceFeatures {
            device: physical_device,
            feature: "sampler_anisotropy",
        });
    }

    Ok(features.features)
}

fn query_swapchain_support(
    entry: &ash::Entry,
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    surface: vk::SurfaceKHR,
) -> Result<
    (
        vk::SurfaceCapabilitiesKHR,
        Vec<vk::SurfaceFormatKHR>,
        Vec<vk::PresentModeKHR>,
    ),
    SuitabilityError,
> {
    let surface_loader = khr::surface::Instance::new(entry, instance);

    let capablities = unsafe {
        surface_loader
            .get_physical_device_surface_capabilities(physical_device, surface)
            .or(Err(SuitabilityError::SurfaceNotSupported {
                device: physical_device,
                surface,
            }))?
    };

    let formats = unsafe {
        surface_loader
            .get_physical_device_surface_formats(physical_device, surface)
            .or(Err(SuitabilityError::SurfaceNotSupported {
                device: physical_device,
                surface,
            }))?
    };

    let present_modes = unsafe {
        surface_loader
            .get_physical_device_surface_present_modes(physical_device, surface)
            .or(Err(SuitabilityError::SurfaceNotSupported {
                device: physical_device,
                surface,
            }))?
    };

    if formats.is_empty() || present_modes.is_empty() {
        return Err(SuitabilityError::SurfaceNotSupported {
            device: physical_device,
            surface,
        });
    }

    Ok((capablities, formats, present_modes))
}

#[derive(Debug, Clone, Copy)]
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
    ) -> Result<Self, SuitabilityError> {
        let properties = unsafe {
            let len = instance.get_physical_device_queue_family_properties2_len(physical_device);
            let mut properties = vec![vk::QueueFamilyProperties2::default(); len];
            instance.get_physical_device_queue_family_properties2(physical_device, &mut properties);
            properties
        };

        let graphics = properties
            .iter()
            .position(|p| {
                p.queue_family_properties
                    .queue_flags
                    .contains(vk::QueueFlags::GRAPHICS)
            })
            .ok_or(SuitabilityError::MissingQueueFamily {
                device: physical_device,
                queue_family: "graphics",
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
            .position(|index| index.is_some())
            .ok_or(SuitabilityError::MissingQueueFamily {
                device: physical_device,
                queue_family: "present",
            })?;

        Ok(QueueFamilyIndices {
            graphics: graphics as u32,
            present: present as u32,
            transfer: transfer as u32,
        })
    }
}

fn get_physical_device_score(
    properties: &vk::PhysicalDeviceProperties,
    indices: &QueueFamilyIndices,
    device_requirements: &DeviceRequirements,
) -> u32 {
    let mut score = 0;

    if device_requirements.prefer_discrete_gpu {
        if properties.device_type == vk::PhysicalDeviceType::DISCRETE_GPU {
            score += 10000;
        } else if properties.device_type == vk::PhysicalDeviceType::INTEGRATED_GPU {
            score += 5000;
        } else {
            score += 1000;
        }
    }

    score += properties.limits.max_image_dimension2_d / 1000;

    if indices.graphics != u32::MAX && indices.present != u32::MAX {
        score += 100;
    }

    score
}

pub struct Device {
    pub device: ash::Device,
    pub graphics_queue: vk::Queue,
    pub present_queue: vk::Queue,
    pub transfer_queue: vk::Queue,
}

impl Resource for Device {}

impl Deref for Device {
    type Target = ash::Device;

    fn deref(&self) -> &Self::Target {
        &self.device
    }
}

pub fn create_logical_device(
    instance: Res<VulkanInstance>,
    physical_device: Res<PhysicalDevice>,
    device_requirements: Option<Res<DeviceRequirements>>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    info!("Creating logical device for physical device: {physical_device:?}",);

    let mut unique_indices = HashSet::new();
    unique_indices.insert(physical_device.indices.graphics);
    unique_indices.insert(physical_device.indices.present);
    unique_indices.insert(physical_device.indices.transfer);

    debug!(
        "Creating logical device with {} queue families",
        unique_indices.len()
    );

    let queue_create_infos: Vec<vk::DeviceQueueCreateInfo> = unique_indices
        .into_iter()
        .map(|index| {
            vk::DeviceQueueCreateInfo::default()
                .queue_family_index(index)
                .queue_priorities(&[1.0])
        })
        .collect();

    let requirements = device_requirements
        .map(|res| res.into_inner())
        .unwrap_or_default();

    let extensions = requirements
        .extensions
        .iter()
        .map(|&e| e.as_ptr())
        .collect::<Vec<_>>();

    let mut physical_device_features_2 =
        vk::PhysicalDeviceFeatures2::default().features(physical_device.features);

    let create_info = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queue_create_infos)
        .enabled_extension_names(&extensions)
        .push_next(&mut physical_device_features_2);

    let device = unsafe { instance.create_device(**physical_device, &create_info, None) }?;

    let graphics_queue = unsafe { device.get_device_queue(physical_device.indices.graphics, 0) };
    let present_queue = unsafe { device.get_device_queue(physical_device.indices.present, 0) };
    let transfer_queue = unsafe { device.get_device_queue(physical_device.indices.transfer, 0) };

    let logical_device = Device {
        device,
        graphics_queue,
        present_queue,
        transfer_queue,
    };

    commands.insert_resource(logical_device);

    Ok(())
}

pub fn destroy_logical_device(device: Res<Device>, mut commands: Commands) {
    info!("Destroying logical device");

    unsafe { device.destroy_device(None) };

    commands.remove_resource::<Device>();
}
