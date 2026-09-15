use crate::device::selection::QueueFamilyIndices;
use ash::vk;

/// What a physical device actually provides, measured at selection time.
#[derive(Debug)]
pub struct DeviceCapabilities {
    pub name: String,
    pub device_type: vk::PhysicalDeviceType,
    /// min(device apiVersion, instance's requested version) — the version
    /// that is actually usable on this device.
    pub api_version: u32,
    pub features: DeviceFeatures,
    pub portability_subset: bool,
    pub memory: vk::PhysicalDeviceMemoryProperties,
    pub queues: QueueFamilyIndices,
    pub max_bound_descriptor_sets: u32,
    pub max_sampler_anisotropy: f32,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DeviceFeatures {
    pub dynamic_rendering: bool,
    pub synchronization2: bool,
    pub timeline_semaphores: bool,
    pub buffer_device_address: bool,
    pub descriptor_indexing: bool,
    pub sampler_anisotropy: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CapabilityGates {
    pub synchronization2: bool,
    pub timeline_semaphores: bool,
    pub buffer_device_address: bool,
    pub descriptor_indexing: bool,
}

pub fn parse_disabled(capabilities: &str) -> Vec<String> {
    capabilities
        .split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

pub fn resolve_gates(
    device_features: DeviceFeatures,
    disabled_features: &[String],
) -> CapabilityGates {
    CapabilityGates {
        synchronization2: device_features.synchronization2
            && !disabled_features.contains(&"synchronization2".to_owned()),
        timeline_semaphores: device_features.timeline_semaphores
            && !disabled_features.contains(&"timeline_semaphores".to_owned()),
        buffer_device_address: device_features.buffer_device_address
            && !disabled_features.contains(&"buffer_device_address".to_owned()),
        descriptor_indexing: device_features.descriptor_indexing
            && !disabled_features.contains(&"descriptor_indexing".to_owned()),
    }
}

/// One line per fact, three states per gate: on, unsupported, or forced off.
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
        gate_state(f.timeline_semaphores, gates.timeline_semaphores)
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
            timeline_semaphores: true,
            buffer_device_address: true,
            descriptor_indexing: true,
            sampler_anisotropy: true,
        }
    }

    // ── parse_disabled: the FLUX_DISABLED_CAPABILITIES env var format ──────

    #[test]
    fn parse_disabled_splits_on_commas_and_trims_whitespace() {
        assert_eq!(
            parse_disabled(" timeline_semaphores,  buffer_device_address "),
            vec![
                "timeline_semaphores".to_owned(),
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
        assert!(gates.timeline_semaphores);
        assert!(gates.buffer_device_address);
        assert!(gates.descriptor_indexing);

        let gates = resolve_gates(DeviceFeatures::default(), &[]);
        assert!(!gates.synchronization2);
        assert!(!gates.timeline_semaphores);
        assert!(!gates.buffer_device_address);
        assert!(!gates.descriptor_indexing);
    }

    #[test]
    fn force_disabling_turns_a_supported_gate_off() {
        let disabled = vec!["timeline_semaphores".to_owned()];
        let gates = resolve_gates(all_features(), &disabled);
        assert!(!gates.timeline_semaphores, "the disabled gate is off");
        assert!(
            gates.synchronization2 && gates.buffer_device_address && gates.descriptor_indexing,
            "other gates are untouched"
        );
    }

    #[test]
    fn force_disabling_an_unsupported_gate_is_harmless() {
        let disabled = vec!["timeline_semaphores".to_owned()];
        let gates = resolve_gates(DeviceFeatures::default(), &disabled);
        assert!(!gates.timeline_semaphores);
    }

    #[test]
    fn unknown_capability_names_change_nothing() {
        let disabled = vec!["definitely_not_a_capability".to_owned()];
        let gates = resolve_gates(all_features(), &disabled);
        assert!(
            gates.synchronization2
                && gates.timeline_semaphores
                && gates.buffer_device_address
                && gates.descriptor_indexing
        );
    }
}
