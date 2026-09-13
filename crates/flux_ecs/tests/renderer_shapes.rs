//! Transcriptions of the renderer's system signatures: every shape the
//! renderer uses must be expressible and runnable. Types are stand-ins; the
//! signatures are faithful (parameter kinds, counts, return types).

use flux_ecs::{
    Commands, Component, Query, Schedule, Single, System, SystemLabel, SystemParam, World,
};
use std::rc::Rc;

// ---- stand-ins for the renderer's resource types ----

macro_rules! marker_resource {
    ($($name:ident),+) => {
        $(
            #[derive(Component, Default, PartialEq, Debug)]
            struct $name(#[allow(dead_code)] u32);
        )+
    };
}
marker_resource!(
    VulkanInstance,
    PhysicalDevice,
    Device,
    VulkanSurface,
    Swapchain,
    Pipeline,
    DepthBuffers,
    CommandPools,
    CommandBuffers,
    Descriptors,
    UniformBuffers,
    VertexBuffer,
    IndexBuffer,
    RendererSettings,
    DeviceRequirements
);

#[derive(Component, Default)]
struct SyncObjects {
    frames_rendered: u32,
}

#[derive(Component, Default)]
struct FrameData {
    frame_index: usize,
}

/// The window handle: not thread-safe, like the real one.
#[derive(Component)]
#[component(non_send)]
struct SurfaceProviderResource(#[allow(dead_code)] Rc<u8>);

#[derive(Component, PartialEq, Debug)]
struct VulkanMesh(#[allow(dead_code)] u32);

#[derive(Debug)]
struct MockVkError;

// ---- the signatures, transcribed ----

fn create_instance(
    settings: Single<&RendererSettings>,
    surface_provider: Single<&SurfaceProviderResource>,
    mut commands: Commands,
) -> Result<(), MockVkError> {
    let _ = (settings.0, &surface_provider);
    commands.insert_singleton(VulkanInstance(1));
    Ok(())
}

fn create_swapchain(
    instance: Single<&VulkanInstance>,
    physical_device: Single<&PhysicalDevice>,
    device: Single<&Device>,
    surface: Single<&VulkanSurface>,
    surface_provider: Single<&SurfaceProviderResource>,
    mut commands: Commands,
) -> Result<(), MockVkError> {
    let _ = (
        instance.0,
        physical_device.0,
        device.0,
        surface.0,
        &*surface_provider,
    );
    commands.insert_singleton(Swapchain(1));
    Ok(())
}

/// The widest signature in the renderer: eleven parameters, fallible.
#[allow(clippy::too_many_arguments)]
fn render(
    instance: Single<&VulkanInstance>,
    device: Single<&Device>,
    swapchain: Single<&Swapchain>,
    command_buffers: Single<&CommandBuffers>,
    depth_buffers: Single<&DepthBuffers>,
    pipeline: Single<&Pipeline>,
    meshes: Query<&VulkanMesh>,
    descriptors: Single<&Descriptors>,
    mut sync_objects: Single<&mut SyncObjects>,
    mut frame_data: Single<&mut FrameData>,
    mut commands: Commands,
) -> Result<(), MockVkError> {
    let _ = (
        instance.0,
        device.0,
        swapchain.0,
        command_buffers.0,
        depth_buffers.0,
        pipeline.0,
        descriptors.0,
    );
    let mut mesh_count = 0;
    meshes.for_each(|_| mesh_count += 1);
    sync_objects.frames_rendered += 1;
    frame_data.frame_index = (frame_data.frame_index + 1) % 2;
    commands.spawn((VulkanMesh(mesh_count),));
    Ok(())
}

fn wait_device_idle(device: Single<&Device>) {
    let _ = device.0;
}

fn destroy_swapchain(swapchain: Single<&Swapchain>, mut commands: Commands) {
    let _ = swapchain.0;
    commands.remove_singleton::<Swapchain>();
}

fn failing_system(_device: Single<&Device>) -> Result<(), MockVkError> {
    Err(MockVkError)
}

// ---- the schedules, wired like the renderer's init/render/destroy ----

#[test]
fn renderer_shaped_schedules_run() {
    const INSTANCE: SystemLabel = SystemLabel("instance");
    const SWAPCHAIN: SystemLabel = SystemLabel("swapchain");

    let mut world = World::new();
    world.insert_singleton(RendererSettings(0));
    world.insert_singleton(SurfaceProviderResource(Rc::new(0)));
    // resources the transcribed subset does not create itself
    world.insert_singleton(PhysicalDevice(0));
    world.insert_singleton(Device(0));
    world.insert_singleton(VulkanSurface(0));
    world.insert_singleton(CommandBuffers(0));
    world.insert_singleton(DepthBuffers(0));
    world.insert_singleton(Pipeline(0));
    world.insert_singleton(Descriptors(0));
    world.insert_singleton(SyncObjects::default());
    world.insert_singleton(FrameData::default());

    let mut init = Schedule::new();
    init.add(create_instance).label(INSTANCE);
    init.add(create_swapchain).label(SWAPCHAIN).after(INSTANCE);
    init.run(&mut world);
    assert!(
        world.singleton::<VulkanInstance>().is_some(),
        "deferred insert applied"
    );
    assert!(
        world.singleton::<Swapchain>().is_some(),
        "ordering held: swapchain saw the instance"
    );

    let mut main = Schedule::new();
    main.add(render);
    main.run(&mut world);
    main.run(&mut world);
    assert_eq!(world.singleton::<SyncObjects>().unwrap().frames_rendered, 2);
    assert_eq!(
        world.singleton::<FrameData>().unwrap().frame_index,
        0,
        "wrapped around"
    );

    let mut shutdown = Schedule::new();
    shutdown.add(wait_device_idle);
    shutdown.add(destroy_swapchain);
    shutdown.run(&mut world);
    assert!(
        world.singleton::<Swapchain>().is_none(),
        "deferred removal applied"
    );
}

/// The same shape as `render`, with related requests grouped into tuples.
#[allow(clippy::type_complexity)]
fn render_grouped(
    gpu: (
        Single<&VulkanInstance>,
        Single<&Device>,
        Single<&Swapchain>,
        Single<&Pipeline>,
        Single<&Descriptors>,
    ),
    targets: (Single<&CommandBuffers>, Single<&DepthBuffers>),
    meshes: Query<&VulkanMesh>,
    frame: (Single<&mut SyncObjects>, Single<&mut FrameData>),
    mut commands: Commands,
) -> Result<(), MockVkError> {
    let (instance, device, swapchain, pipeline, descriptors) = gpu;
    let (command_buffers, depth_buffers) = targets;
    let (mut sync_objects, mut frame_data) = frame;
    let _ = (
        instance.0,
        device.0,
        swapchain.0,
        command_buffers.0,
        depth_buffers.0,
        pipeline.0,
        descriptors.0,
    );
    let mut mesh_count = 0;
    meshes.for_each(|_| mesh_count += 1);
    sync_objects.frames_rendered += 1;
    frame_data.frame_index = (frame_data.frame_index + 1) % 2;
    commands.spawn((VulkanMesh(mesh_count),));
    Ok(())
}

/// The grouped shape again, with named fields instead of tuple positions.
#[derive(SystemParam)]
struct GpuContext<'w> {
    instance: Single<'w, &'w VulkanInstance>,
    device: Single<'w, &'w Device>,
    swapchain: Single<'w, &'w Swapchain>,
    pipeline: Single<'w, &'w Pipeline>,
    descriptors: Single<'w, &'w Descriptors>,
}

#[derive(SystemParam)]
struct FrameContext<'w, 's> {
    sync_objects: Single<'w, &'w mut SyncObjects>,
    frame_data: Single<'w, &'w mut FrameData>,
    commands: Commands<'s>,
}

fn render_derived(
    gpu: GpuContext,
    meshes: Query<&VulkanMesh>,
    mut frame: FrameContext,
) -> Result<(), MockVkError> {
    let _ = (
        gpu.instance.0,
        gpu.device.0,
        gpu.swapchain.0,
        gpu.pipeline.0,
        gpu.descriptors.0,
    );
    let mut mesh_count = 0;
    meshes.for_each(|_| mesh_count += 1);
    frame.sync_objects.frames_rendered += 1;
    frame.frame_data.frame_index = (frame.frame_data.frame_index + 1) % 2;
    frame.commands.spawn((VulkanMesh(mesh_count),));
    Ok(())
}

#[test]
fn derived_parameter_structs_behave_like_flat_ones() {
    let mut world = World::new();
    world.insert_singleton(VulkanInstance(0));
    world.insert_singleton(Device(0));
    world.insert_singleton(Swapchain(0));
    world.insert_singleton(Pipeline(0));
    world.insert_singleton(Descriptors(0));
    world.insert_singleton(SyncObjects::default());
    world.insert_singleton(FrameData::default());
    let mut schedule = Schedule::new();
    schedule.add(render_derived);
    schedule.run(&mut world);
    schedule.run(&mut world);
    assert_eq!(world.singleton::<SyncObjects>().unwrap().frames_rendered, 2);
    let mut meshes = flux_ecs::QueryState::<&VulkanMesh>::new();
    let n: usize = world.query(&mut meshes).chunks().map(|m| m.len()).sum();
    assert_eq!(
        n, 2,
        "deferred spawns from the derived Commands field applied"
    );
}

#[test]
fn grouped_parameters_behave_like_flat_ones() {
    let mut world = World::new();
    world.insert_singleton(VulkanInstance(0));
    world.insert_singleton(Device(0));
    world.insert_singleton(Swapchain(0));
    world.insert_singleton(Pipeline(0));
    world.insert_singleton(Descriptors(0));
    world.insert_singleton(CommandBuffers(0));
    world.insert_singleton(DepthBuffers(0));
    world.insert_singleton(SyncObjects::default());
    world.insert_singleton(FrameData::default());
    let mut schedule = Schedule::new();
    schedule.add(render_grouped);
    schedule.run(&mut world);
    schedule.run(&mut world);
    assert_eq!(world.singleton::<SyncObjects>().unwrap().frames_rendered, 2);
}

#[test]
fn commands_apply_after_the_system_not_during() {
    fn creates(mut commands: Commands) {
        commands.insert_singleton(Device(7));
    }
    fn observes(device: Single<&Device>) {
        assert_eq!(device.0, 7);
    }
    let mut world = World::new();
    let mut schedule = Schedule::new();
    const C: SystemLabel = SystemLabel("creates");
    schedule.add(creates).label(C);
    schedule.add(observes).after(C);
    schedule.run(&mut world);
    assert!(world.singleton::<Device>().is_some());
}

#[test]
fn command_spawns_become_visible_to_later_systems() {
    fn spawner(mut commands: Commands) {
        commands.spawn((VulkanMesh(1),));
        commands.spawn((VulkanMesh(2),));
    }
    fn counter(meshes: Query<&VulkanMesh>, mut total: Single<&mut Device>) {
        let mut n = 0;
        meshes.for_each(|_| n += 1);
        total.0 = n;
    }
    let mut world = World::new();
    world.insert_singleton(Device(0));
    const S: SystemLabel = SystemLabel("spawner");
    let mut schedule = Schedule::new();
    schedule.add(spawner).label(S);
    schedule.add(counter).after(S);
    schedule.run(&mut world);
    assert_eq!(world.singleton::<Device>().unwrap().0, 2);
    schedule.run(&mut world);
    assert_eq!(
        world.singleton::<Device>().unwrap().0,
        4,
        "queues drain per run: two new spawns, not replayed old ones"
    );
}

#[test]
fn command_spawn_ids_are_usable_immediately() {
    #[derive(Component, Default)]
    struct Probe(Option<flux_ecs::Entity>);

    fn spawner(mut commands: Commands, mut probe: Single<&mut Probe>) {
        let e = commands.spawn((VulkanMesh(1),));
        commands.insert(e, Device(42)); // command against the reserved id
        probe.0 = Some(e);
    }
    let mut world = World::new();
    world.insert_singleton(Probe::default());
    let mut schedule = Schedule::new();
    schedule.add(spawner);
    schedule.run(&mut world);

    let e = world.singleton::<Probe>().unwrap().0.expect("recorded");
    assert!(world.is_alive(e), "alive once commands applied");
    assert_eq!(
        world.get::<Device>(e),
        Some(&Device(42)),
        "follow-up command hit the reserved id"
    );
    assert!(world.get::<VulkanMesh>(e).is_some());
}

#[test]
fn reserved_spawns_survive_interleaved_frees() {
    #[derive(Component, Default)]
    struct Probe(Option<flux_ecs::Entity>);

    // remove_singleton frees an entity slot mid-queue; the reserved spawn
    // after it must still land on its predicted id.
    fn churn(mut commands: Commands, mut probe: Single<&mut Probe>) {
        commands.remove_singleton::<Swapchain>();
        let e = commands.spawn((VulkanMesh(9),));
        probe.0 = Some(e);
    }
    let mut world = World::new();
    world.insert_singleton(Swapchain(0));
    world.insert_singleton(Probe::default());
    let mut schedule = Schedule::new();
    schedule.add(churn);
    schedule.run(&mut world);

    let e = world.singleton::<Probe>().unwrap().0.expect("recorded");
    assert!(world.is_alive(e));
    assert_eq!(world.get::<VulkanMesh>(e), Some(&VulkanMesh(9)));
    assert!(world.singleton::<Swapchain>().is_none());
}

#[test]
fn a_failed_systems_commands_are_discarded() {
    fn tries(mut commands: Commands) -> Result<(), MockVkError> {
        commands.insert_singleton(Device(1));
        commands.spawn((VulkanMesh(1),));
        Err(MockVkError)
    }
    let mut world = World::new();
    let mut system = flux_ecs::IntoSystem::into_system(tries);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        system.run(&mut world);
    }));
    assert!(panicked.is_err(), "the failure still panics");
    assert!(
        world.singleton::<Device>().is_none(),
        "no effects from the failed run"
    );
    assert_eq!(world.len(), 0, "no spawns from the failed run");

    // the queue is empty: a later successful run applies only its own work
    fn succeeds(mut commands: Commands) {
        commands.insert_singleton(Device(2));
    }
    let mut ok = flux_ecs::IntoSystem::into_system(succeeds);
    ok.run(&mut world);
    assert_eq!(world.singleton::<Device>(), Some(&Device(2)));
    assert_eq!(world.len(), 1, "exactly the successful system's effects");
}

#[test]
#[should_panic(expected = "failing_system")]
fn a_failing_system_panics_naming_itself() {
    let mut world = World::new();
    world.insert_singleton(Device(0));
    let mut schedule = Schedule::new();
    schedule.add(failing_system);
    schedule.run(&mut world);
}
