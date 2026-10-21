use ash::vk;

pub fn choose_surface_format(formats: &[vk::SurfaceFormatKHR]) -> vk::SurfaceFormatKHR {
    formats
        .iter()
        .copied()
        .find(|f| {
            f.format == vk::Format::B8G8R8A8_SRGB
                && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        })
        .unwrap_or(formats[0])
}

pub fn choose_present_mode(present_modes: &[vk::PresentModeKHR]) -> vk::PresentModeKHR {
    *present_modes
        .iter()
        .find(|mode| **mode == vk::PresentModeKHR::MAILBOX)
        .unwrap_or(&vk::PresentModeKHR::FIFO)
}

pub fn choose_image_count(capabilities: &vk::SurfaceCapabilitiesKHR) -> u32 {
    let max = if capabilities.max_image_count == 0 {
        u32::MAX
    } else {
        capabilities.max_image_count
    };
    (capabilities.min_image_count + 1).min(max)
}

pub fn choose_extent(
    capabilities: &vk::SurfaceCapabilitiesKHR,
    window_extent: (u32, u32),
) -> vk::Extent2D {
    if capabilities.current_extent.width == u32::MAX {
        let min_extent = capabilities.min_image_extent;
        let max_extent = capabilities.max_image_extent;

        vk::Extent2D {
            width: window_extent.0.clamp(min_extent.width, max_extent.width),
            height: window_extent.1.clamp(min_extent.height, max_extent.height),
        }
    } else {
        capabilities.current_extent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ash::vk;

    fn format(f: vk::Format, cs: vk::ColorSpaceKHR) -> vk::SurfaceFormatKHR {
        vk::SurfaceFormatKHR {
            format: f,
            color_space: cs,
        }
    }

    // ── choose_surface_format ───────────────────────────────────────────────

    #[test]
    fn srgb_bgra_is_preferred_when_offered() {
        let available = [
            format(
                vk::Format::R8G8B8A8_UNORM,
                vk::ColorSpaceKHR::SRGB_NONLINEAR,
            ),
            format(vk::Format::B8G8R8A8_SRGB, vk::ColorSpaceKHR::SRGB_NONLINEAR),
        ];
        let chosen = choose_surface_format(&available);
        assert_eq!(chosen.format, vk::Format::B8G8R8A8_SRGB);
        assert_eq!(chosen.color_space, vk::ColorSpaceKHR::SRGB_NONLINEAR);
    }

    #[test]
    fn first_offered_format_is_the_fallback() {
        let available = [
            format(
                vk::Format::R16G16B16A16_SFLOAT,
                vk::ColorSpaceKHR::EXTENDED_SRGB_LINEAR_EXT,
            ),
            format(
                vk::Format::R8G8B8A8_UNORM,
                vk::ColorSpaceKHR::SRGB_NONLINEAR,
            ),
        ];
        assert_eq!(choose_surface_format(&available), available[0]);
    }

    // ── choose_present_mode ─────────────────────────────────────────────────

    #[test]
    fn mailbox_is_preferred_when_offered() {
        let available = [
            vk::PresentModeKHR::FIFO,
            vk::PresentModeKHR::MAILBOX,
            vk::PresentModeKHR::IMMEDIATE,
        ];
        assert_eq!(choose_present_mode(&available), vk::PresentModeKHR::MAILBOX);
    }

    #[test]
    fn fifo_is_the_fallback() {
        // FIFO is the only mode the spec guarantees.
        let available = [vk::PresentModeKHR::FIFO, vk::PresentModeKHR::IMMEDIATE];
        assert_eq!(choose_present_mode(&available), vk::PresentModeKHR::FIFO);
    }

    // ── choose_image_count ──────────────────────────────────────────────────

    #[test]
    fn one_more_than_the_minimum_is_requested() {
        let caps = vk::SurfaceCapabilitiesKHR {
            min_image_count: 2,
            max_image_count: 8,
            ..Default::default()
        };
        assert_eq!(choose_image_count(&caps), 3);
    }

    #[test]
    fn the_maximum_caps_the_request() {
        let caps = vk::SurfaceCapabilitiesKHR {
            min_image_count: 3,
            max_image_count: 3,
            ..Default::default()
        };
        assert_eq!(choose_image_count(&caps), 3);
    }

    #[test]
    fn zero_maximum_means_unbounded() {
        let caps = vk::SurfaceCapabilitiesKHR {
            min_image_count: 4,
            max_image_count: 0,
            ..Default::default()
        };
        assert_eq!(choose_image_count(&caps), 5);
    }

    // ── choose_extent ───────────────────────────────────────────────────────

    #[test]
    fn a_fixed_current_extent_is_taken_verbatim() {
        let caps = vk::SurfaceCapabilitiesKHR {
            current_extent: vk::Extent2D {
                width: 800,
                height: 600,
            },
            ..Default::default()
        };
        let extent = choose_extent(&caps, (1024, 768));
        assert_eq!((extent.width, extent.height), (800, 600));
    }

    #[test]
    fn a_flexible_surface_clamps_the_window_size_into_bounds() {
        // current_extent of u32::MAX means the surface follows the swapchain.
        let caps = vk::SurfaceCapabilitiesKHR {
            current_extent: vk::Extent2D {
                width: u32::MAX,
                height: u32::MAX,
            },
            min_image_extent: vk::Extent2D {
                width: 200,
                height: 200,
            },
            max_image_extent: vk::Extent2D {
                width: 4096,
                height: 4096,
            },
            ..Default::default()
        };
        let extent = choose_extent(&caps, (100, 8000));
        assert_eq!((extent.width, extent.height), (200, 4096));
    }
}
