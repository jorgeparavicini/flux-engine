//! What a device can do, and what the renderer chooses to use.
//!
//! Feature information flows one way through three shapes:
//!
//! 1. [`DeviceFeatures`]: what the hardware supports. Measured during
//!    device selection, never edited afterwards.
//! 2. [`CapabilityGates`]: what the renderer will actually use. A feature's
//!    gate is open when the device supports it and it was not force-disabled
//!    (see [`parse_disabled`]). Code that can take a fast path branches on
//!    the gate, never on the raw feature.
//! 3. `EnabledFeatures` (device creation): what is requested from the
//!    driver: floor features plus every open gate.
//!
//! Floor features (currently dynamic rendering) have no gate: the renderer
//! has no fallback path for them, so unsuitable devices are rejected during
//! selection instead.

use crate::device::selection::QueueFamilyIndices;
use ash::vk;
use log::warn;

/// What a physical device actually provides, measured at selection time.
#[derive(Debug, Clone)]
pub struct DeviceCapabilities {
    pub name: String,
    pub device_type: vk::PhysicalDeviceType,
    /// The version actually usable on this device: the minimum of the
    /// device's apiVersion and the version the instance requested.
    pub api_version: u32,
    pub features: DeviceFeatures,
    pub portability_subset: bool,
    pub memory: vk::PhysicalDeviceMemoryProperties,
    pub queues: QueueFamilyIndices,
    pub max_bound_descriptor_sets: u32,
    pub max_sampler_anisotropy: f32,
}

/// The optional features a physical device supports, measured once during
/// selection. Support alone does not mean the renderer uses a feature; that
/// decision lives in [`CapabilityGates`].
#[derive(Debug, Default, Clone, Copy)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a capability record is a set of flags"
)]
pub struct DeviceFeatures {
    pub dynamic_rendering: bool,
    pub synchronization2: bool,
    pub timeline_semaphore: bool,
    pub buffer_device_address: bool,
    pub descriptor_indexing: bool,
    pub sampler_anisotropy: bool,
}

/// The renderer's per-device decisions: a gate is open when the feature is
/// supported and was not force-disabled. Fast-path code branches on these;
/// each closed gate has a working fallback.
#[derive(Debug, Default, Clone, Copy)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a capability record is a set of flags"
)]
pub struct CapabilityGates {
    pub synchronization2: bool,
    pub timeline_semaphore: bool,
    pub buffer_device_address: bool,
    pub descriptor_indexing: bool,
}

/// Parses the `FLUX_DISABLED_CAPABILITIES` environment variable: a
/// comma-separated list of gate names (the field names of
/// [`CapabilityGates`], e.g. `"timeline_semaphore, buffer_device_address"`).
pub fn parse_disabled(capabilities: &str) -> Vec<String> {
    capabilities
        .split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

/// Opens a gate for every supported feature, then closes the ones named in
/// `disabled`. Unknown names are reported and ignored, so a typo in an
/// override is visible instead of silently doing nothing.
pub fn resolve_gates(features: DeviceFeatures, disabled: &[String]) -> CapabilityGates {
    let mut gates = CapabilityGates {
        synchronization2: features.synchronization2,
        timeline_semaphore: features.timeline_semaphore,
        buffer_device_address: features.buffer_device_address,
        descriptor_indexing: features.descriptor_indexing,
    };
    for name in disabled {
        // The single place a gate name in the override vocabulary is tied to
        // its field; extend this match when adding a gate.
        let gate = match name.as_str() {
            "synchronization2" => &mut gates.synchronization2,
            "timeline_semaphore" => &mut gates.timeline_semaphore,
            "buffer_device_address" => &mut gates.buffer_device_address,
            "descriptor_indexing" => &mut gates.descriptor_indexing,
            _ => {
                warn!("unknown capability {name:?} in FLUX_DISABLED_CAPABILITIES; ignoring");
                continue;
            }
        };
        *gate = false;
    }
    gates
}

/// Renders the record as a human-readable multi-line summary for the startup
/// log. Each gate shows one of three states: on, unsupported, or forced off.
pub fn report(capabilities: &DeviceCapabilities, gates: CapabilityGates) -> String {
    use crate::device::instance::format_version;
    use std::fmt::Write;

    const fn gate_state(supported: bool, gate: bool) -> &'static str {
        match (supported, gate) {
            (true, true) => "on",
            (true, false) => "forced off",
            (false, _) => "unsupported",
        }
    }

    let f = &capabilities.features;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "Selected device: {} ({:?})",
        capabilities.name, capabilities.device_type
    );
    let _ = writeln!(
        out,
        "  effective Vulkan version: {}",
        format_version(capabilities.api_version)
    );
    let _ = writeln!(
        out,
        "  queues: graphics {}, present {}, transfer {}",
        capabilities.queues.graphics, capabilities.queues.present, capabilities.queues.transfer
    );
    let _ = writeln!(
        out,
        "  portability subset: {}",
        capabilities.portability_subset
    );
    let device_local: u64 = capabilities
        .memory
        .memory_heaps_as_slice()
        .iter()
        .filter(|heap| heap.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL))
        .map(|heap| heap.size)
        .sum();
    let _ = writeln!(
        out,
        "  device-local memory: {} MiB",
        device_local / (1024 * 1024)
    );
    let _ = writeln!(
        out,
        "  limits: max bound descriptor sets {}, max sampler anisotropy {}",
        capabilities.max_bound_descriptor_sets, capabilities.max_sampler_anisotropy
    );
    let _ = writeln!(
        out,
        "  sampler anisotropy: {}",
        if capabilities.features.sampler_anisotropy {
            "supported"
        } else {
            "unsupported"
        }
    );
    let _ = writeln!(out, "  dynamic rendering: on (floor)");
    let _ = writeln!(
        out,
        "  synchronization2: {}",
        gate_state(f.synchronization2, gates.synchronization2)
    );
    let _ = writeln!(
        out,
        "  timeline semaphores: {}",
        gate_state(f.timeline_semaphore, gates.timeline_semaphore)
    );
    let _ = writeln!(
        out,
        "  buffer device address: {}",
        gate_state(f.buffer_device_address, gates.buffer_device_address)
    );
    let _ = write!(
        out,
        "  descriptor indexing: {}",
        gate_state(f.descriptor_indexing, gates.descriptor_indexing)
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── parse_disabled: the FLUX_DISABLED_CAPABILITIES env var format ──────

    #[test]
    fn parse_disabled_splits_on_commas_and_trims_whitespace() {
        assert_eq!(
            parse_disabled(" timeline_semaphore,  buffer_device_address "),
            vec![
                "timeline_semaphore".to_owned(),
                "buffer_device_address".to_owned()
            ]
        );
    }

    #[test]
    fn parse_disabled_yields_nothing_for_empty_or_blank_input() {
        assert_eq!(parse_disabled(""), Vec::<String>::new());
        assert_eq!(parse_disabled("  , ,, "), Vec::<String>::new());
    }

    // ── resolve_gates: gate = supported ∧ not force-disabled ───────────────

    #[test]
    fn gates_mirror_features_when_nothing_is_disabled() {
        let gates = resolve_gates(all_features(), &[]);
        assert!(gates.synchronization2);
        assert!(gates.timeline_semaphore);
        assert!(gates.buffer_device_address);
        assert!(gates.descriptor_indexing);

        let gates = resolve_gates(DeviceFeatures::default(), &[]);
        assert!(!gates.synchronization2);
        assert!(!gates.timeline_semaphore);
        assert!(!gates.buffer_device_address);
        assert!(!gates.descriptor_indexing);
    }

    #[test]
    fn force_disabling_turns_a_supported_gate_off() {
        let disabled = vec!["timeline_semaphore".to_owned()];
        let gates = resolve_gates(all_features(), &disabled);
        assert!(!gates.timeline_semaphore, "the disabled gate is off");
        assert!(
            gates.synchronization2 && gates.buffer_device_address && gates.descriptor_indexing,
            "other gates are untouched"
        );
    }

    #[test]
    fn force_disabling_an_unsupported_gate_is_harmless() {
        let disabled = vec!["timeline_semaphore".to_owned()];
        let gates = resolve_gates(DeviceFeatures::default(), &disabled);
        assert!(!gates.timeline_semaphore);
    }

    #[test]
    fn unknown_capability_names_change_nothing() {
        let disabled = vec!["definitely_not_a_capability".to_owned()];
        let gates = resolve_gates(all_features(), &disabled);
        assert!(
            gates.synchronization2
                && gates.timeline_semaphore
                && gates.buffer_device_address
                && gates.descriptor_indexing
        );
    }
}
