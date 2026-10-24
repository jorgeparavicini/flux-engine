//! Device tests: run against any installed Vulkan implementation, including
//! lavapipe in CI. No window system is required.

use flux_renderer_api::settings::{AppInfo, RendererSettings};
use flux_renderer_vulkan::ash::vk;
use flux_renderer_vulkan::{
    GpuBuffer, GpuMemory, MemoryUsage, RenderDevice, StagingRing, StagingSlice, Uploader,
    take_validation_errors,
};

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

#[test]
fn staged_bytes_round_trip_through_the_gpu() {
    let _ = pretty_env_logger::try_init();
    let app = AppInfo {
        name: "flux upload tests".to_owned(),
        ..AppInfo::default()
    };
    let (instance, device) =
        RenderDevice::new_headless(&app, &RendererSettings::default()).expect("headless device");

    {
        let mut memory = GpuMemory::new(&device);
        let mut staging = StagingRing::new(&device, &mut memory, 1024).expect("staging ring");
        let mut uploader = Uploader::new(&device).expect("uploader");

        let payload: Vec<u8> = (0..=255).collect();
        let gpu_only = GpuBuffer::new(
            &device,
            &mut memory,
            payload.len() as u64,
            vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::TRANSFER_SRC,
            MemoryUsage::GpuOnly,
            "round trip target",
        )
        .expect("gpu-only buffer");
        let readback = GpuBuffer::new(
            &device,
            &mut memory,
            payload.len() as u64,
            vk::BufferUsageFlags::TRANSFER_DST,
            MemoryUsage::Readback,
            "round trip readback",
        )
        .expect("readback buffer");

        let slice = staging.stage(0, &payload).expect("staged payload");

        uploader.begin().expect("begin");
        uploader.copy_buffer(slice, gpu_only.raw(), 0);
        uploader.submit_and_wait().expect("upload");

        uploader.begin().expect("begin readback");
        uploader.copy_buffer(
            StagingSlice {
                buffer: gpu_only.raw(),
                offset: 0,
                size: payload.len() as u64,
            },
            readback.raw(),
            0,
        );
        uploader.submit_and_wait().expect("copy back");

        let read = readback.read_all();
        assert_eq!(read, payload, "bytes must survive the round trip");

        drop(gpu_only);
        drop(readback);
        drop(staging);
        device.wait_idle();
        memory.drain();
        assert_eq!(memory.allocator.live(), 0, "everything returned");
    }

    device.wait_idle();
    drop(device);
    drop(instance);
    let errors = take_validation_errors();
    assert!(errors.is_empty(), "validation errors recorded: {errors:#?}");
}
