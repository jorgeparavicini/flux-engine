//! Device tests: run against any installed Vulkan implementation, including
//! lavapipe in CI. No window system is required.

use flux_renderer_api::settings::{AppInfo, RendererSettings};
use flux_renderer_vulkan::ash::vk;
use flux_renderer_vulkan::{RenderDevice, take_validation_errors};

#[test]
fn headless_device_creation_is_validation_clean() {
    let _ = pretty_env_logger::try_init();
    let app = AppInfo {
        name: "flux device tests".to_owned(),
        ..AppInfo::default()
    };
    let (instance, device) =
        RenderDevice::new_headless(&app, &RendererSettings::default()).expect("headless device");

    let capabilities = &device.capabilities;
    assert_ne!(capabilities.name, "");
    assert!(
        capabilities.api_version >= vk::make_api_version(0, 1, 3, 0),
        "selection enforces the version floor"
    );
    assert!(
        capabilities.features.dynamic_rendering,
        "selection enforces the feature floor"
    );
    assert_eq!(device.graphics.family, capabilities.queues.graphics);

    device.wait_idle();
    drop(device);
    drop(instance);

    let errors = take_validation_errors();
    assert!(errors.is_empty(), "validation errors recorded: {errors:#?}");
}
