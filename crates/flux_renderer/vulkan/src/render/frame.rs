use crate::device::logical::RenderDevice;
use crate::error::RendererError;
use crate::memory::gpu_memory::GpuMemory;
use crate::pipeline::descriptors::Descriptors;
use crate::pipeline::graphics::Pipeline;
use crate::present::presenter::{Acquire, Frame, Presenter};
use crate::render::commands::CommandBuffers;
use crate::render::mesh::VulkanMesh;
use crate::resource::depth::DepthBuffer;
use crate::resource::upload::StagingRing;
use ash::vk;
use ash::vk::IndexType;
use flux_ecs::Query;
use flux_ecs::Single;
use flux_ecs::World;

/// True while the presentation no longer matches the surface; the recreation
/// systems in the render schedule run while set.
#[derive(flux_ecs::Component, Default)]
pub struct PresentationOutdated(pub bool);

pub fn presentation_outdated(world: &World) -> bool {
    world
        .singleton::<PresentationOutdated>()
        .is_some_and(|flag| flag.0)
}

/// True once the presenter has been rebuilt for an outdated presentation.
/// Its dependents are recreated only then, so a minimized window, whose
/// rebuild is skipped, does not churn them every frame.
pub fn presentation_rebuilt(world: &World) -> bool {
    presentation_outdated(world)
        && world
            .singleton::<Presenter>()
            .is_some_and(|presenter| !presenter.needs_rebuild())
}

pub fn clear_presentation_outdated(mut flag: Single<&mut PresentationOutdated>) {
    flag.0 = false;
}

/// Renders and presents one frame. While the presenter is out of date the
/// frame is skipped and the recreation systems take over.
pub fn render(
    device: Single<&RenderDevice>,
    mut presenter: Single<&mut Presenter>,
    command_buffers: Single<&CommandBuffers>,
    depth_buffer: Single<&DepthBuffer>,
    pipeline: Single<&Pipeline>,
    meshes: Query<&VulkanMesh>,
    descriptors: Single<&Descriptors>,
    mut outdated: Single<&mut PresentationOutdated>,
    mut memory: Single<&mut GpuMemory>,
    mut staging: Single<&mut StagingRing>,
) -> Result<(), RendererError> {
    if presenter.needs_rebuild() {
        outdated.0 = true;
        return Ok(());
    }

    let frame = match presenter.acquire()? {
        Acquire::Frame(frame) => frame,
        Acquire::OutOfDate => {
            outdated.0 = true;
            return Ok(());
        }
    };
    // The acquire waited on this slot's fence, so everything the slot last
    // used is provably done.
    memory.advance(frame.slot);
    staging.advance(frame.slot);

    let command_buffer = command_buffers.command_buffers[frame.image_index as usize];
    record_command_buffer(
        &device,
        command_buffer,
        &presenter,
        &frame,
        &depth_buffer,
        &pipeline,
        meshes,
        &descriptors,
    )?;

    // The acquired image must be ready before color output; rendering
    // completion releases both the presentation wait and the slot's fence.
    let wait_semaphores = [frame.image_available];
    let wait_stages = [vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT];
    let submit_command_buffers = [command_buffer];
    let signal_semaphores = [frame.render_finished];
    let submit_info = vk::SubmitInfo::default()
        .wait_semaphores(&wait_semaphores)
        .wait_dst_stage_mask(&wait_stages)
        .command_buffers(&submit_command_buffers)
        .signal_semaphores(&signal_semaphores);

    unsafe { device.queue_submit(device.graphics.raw, &[submit_info], frame.in_flight)? };

    presenter.present(&device, &frame)?;
    if presenter.needs_rebuild() {
        outdated.0 = true;
    }

    Ok(())
}

pub fn wait_device_idle(device: Single<&RenderDevice>) {
    device.wait_idle();
}

#[allow(clippy::too_many_arguments)]
fn record_command_buffer(
    device: &RenderDevice,
    command_buffer: vk::CommandBuffer,
    presenter: &Presenter,
    frame: &Frame,
    depth_buffer: &DepthBuffer,
    pipeline: &Pipeline,
    meshes: Query<&VulkanMesh>,
    descriptors: &Descriptors,
) -> Result<(), vk::Result> {
    let image = presenter.image(frame.image_index);
    unsafe {
        let inheritance = vk::CommandBufferInheritanceInfo::default();

        let info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::empty())
            .inheritance_info(&inheritance);

        device.begin_command_buffer(command_buffer, &info)?;
        let pass_label = device
            .debug_utils
            .cmd_label_scope(command_buffer, "forward pass");

        let image_barrier_to_render = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::empty()) // No need to wait for previous operations
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE) // We are preparing to write
            .old_layout(vk::ImageLayout::UNDEFINED) // We don't care about the previous layout/contents
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL) // Layout needed for rendering
            .image(image)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .level_count(1)
                    .layer_count(1),
            );

        device.cmd_pipeline_barrier(
            command_buffer,
            vk::PipelineStageFlags::TOP_OF_PIPE, // Source stage
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT, // Destination stage
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[image_barrier_to_render],
        );

        let render_area = vk::Rect2D::default()
            .offset(vk::Offset2D::default())
            .extent(presenter.extent);

        let color_clear_value = vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 1.0],
            },
        };

        let depth_clear_value = vk::ClearValue {
            depth_stencil: vk::ClearDepthStencilValue {
                depth: 1.0,
                stencil: 0,
            },
        };

        let color_attachment_info = vk::RenderingAttachmentInfo::default()
            .image_view(presenter.view(frame.image_index))
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(color_clear_value);

        let depth_attachment_info = vk::RenderingAttachmentInfo::default()
            .image_view(depth_buffer.image.view())
            .image_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(depth_clear_value);

        let color_attachments = [color_attachment_info];
        let rendering_info = vk::RenderingInfo::default()
            .render_area(render_area)
            .layer_count(1)
            .color_attachments(&color_attachments)
            .depth_attachment(&depth_attachment_info);

        device.cmd_begin_rendering(command_buffer, &rendering_info);
        device.cmd_bind_pipeline(command_buffer, vk::PipelineBindPoint::GRAPHICS, **pipeline);

        device.cmd_bind_descriptor_sets(
            command_buffer,
            vk::PipelineBindPoint::GRAPHICS,
            pipeline.pipeline_layout,
            0,
            &[descriptors.descriptor_sets[frame.image_index as usize]],
            &[],
        );

        meshes.for_each(|mesh| {
            device.cmd_bind_vertex_buffers(command_buffer, 0, &[mesh.vertex_buffer.raw()], &[0]);
            match &mesh.indices {
                Some(indices) => {
                    device.cmd_bind_index_buffer(
                        command_buffer,
                        indices.buffer.raw(),
                        0,
                        IndexType::UINT32,
                    );
                    device.cmd_draw_indexed(command_buffer, indices.count, 1, 0, 0, 0);
                }
                None => device.cmd_draw(command_buffer, mesh.vertex_count, 1, 0, 0),
            }
        });

        device.cmd_end_rendering(command_buffer);

        let image_barrier_to_present = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::empty())
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .image(image)
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .level_count(1)
                    .layer_count(1),
            );

        device.cmd_pipeline_barrier(
            command_buffer,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[image_barrier_to_present],
        );

        drop(pass_label);
        device.end_command_buffer(command_buffer)?;

        Ok(())
    }
}
