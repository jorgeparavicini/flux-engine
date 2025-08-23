use crate::buffers::{IndexBuffer, VertexBuffer};
use crate::command_pool::CommandPools;
use crate::depth_buffers::DepthBuffers;
use crate::device::Device;
use crate::pipeline::Pipeline;
use crate::swapchain::Swapchain;
use ash::vk;
use flux_ecs::commands::Commands;
use flux_ecs::resource::{Res, Resource};
use log::debug;
use std::ops::Deref;
use crate::descriptors::Descriptors;

pub struct CommandBuffers {
    pub command_buffers: Vec<vk::CommandBuffer>,
}

impl Deref for CommandBuffers {
    type Target = Vec<vk::CommandBuffer>;

    fn deref(&self) -> &Self::Target {
        &self.command_buffers
    }
}

impl Resource for CommandBuffers {}

pub fn create_command_buffer(
    device: Res<Device>,
    command_pools: Res<CommandPools>,
    swapchain: Res<Swapchain>,
    depth_buffers: Res<DepthBuffers>,
    pipeline: Res<Pipeline>,
    vertex_buffer: Res<VertexBuffer>,
    index_buffer: Res<IndexBuffer>,
    descriptors: Res<Descriptors>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    debug!("Creating command buffer");

    let allocate_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pools.graphics)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(swapchain.images.len() as u32);

    let command_buffers = unsafe { device.allocate_command_buffers(&allocate_info)? };

    for (i, command_buffer) in command_buffers.iter().enumerate() {
        let inheritance = vk::CommandBufferInheritanceInfo::default();

        let info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::empty())
            .inheritance_info(&inheritance);

        unsafe {
            device.begin_command_buffer(*command_buffer, &info)?;
        }

        let image_barrier_to_render = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::empty()) // No need to wait for previous operations
            .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE) // We are preparing to write
            .old_layout(vk::ImageLayout::UNDEFINED) // We don't care about the previous layout/contents
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL) // Layout needed for rendering
            .image(swapchain.images[i])
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .level_count(1)
                    .layer_count(1),
            );

        unsafe {
            device.cmd_pipeline_barrier(
                *command_buffer,
                vk::PipelineStageFlags::TOP_OF_PIPE, // Source stage
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT, // Destination stage
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[image_barrier_to_render],
            );
        }

        let render_area = vk::Rect2D::default()
            .offset(vk::Offset2D::default())
            .extent(swapchain.extent);

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

        // TODO: Compare values with framebuffer attachments
        let color_attachment_info = vk::RenderingAttachmentInfo::default()
            .image_view(swapchain.image_views[i])
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(color_clear_value);

        let depth_attachment_info = vk::RenderingAttachmentInfo::default()
            .image_view(depth_buffers.depth_image_view)
            .image_layout(vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(depth_clear_value);

        let color_attachments = &[color_attachment_info];
        let rendering_info = vk::RenderingInfo::default()
            .render_area(render_area)
            .layer_count(1)
            .color_attachments(color_attachments)
            .depth_attachment(&depth_attachment_info);

        unsafe {
            device.cmd_begin_rendering(*command_buffer, &rendering_info);
            device.cmd_bind_pipeline(*command_buffer, vk::PipelineBindPoint::GRAPHICS, **pipeline);

            device.cmd_bind_vertex_buffers(*command_buffer, 0, &[vertex_buffer.buffer], &[0]);
            device.cmd_bind_index_buffer(
                *command_buffer,
                index_buffer.buffer,
                0,
                vk::IndexType::UINT32,
            );

            device.cmd_bind_descriptor_sets(
                *command_buffer,
                vk::PipelineBindPoint::GRAPHICS,
                pipeline.pipeline_layout,
                0,
                &[descriptors.descriptor_sets[i]],
                &[],
            );

            device.cmd_draw(*command_buffer, 3, 1, 0, 0);

            device.cmd_end_rendering(*command_buffer);
        }

        let image_barrier_to_present = vk::ImageMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
            .dst_access_mask(vk::AccessFlags::empty())
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .image(swapchain.images[i])
            .subresource_range(
                vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .level_count(1)
                    .layer_count(1),
            );

        unsafe {
            device.cmd_pipeline_barrier(
                *command_buffer,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[image_barrier_to_present],
            );
            device.end_command_buffer(*command_buffer)?;
        }
    }

    commands.insert_resource(CommandBuffers { command_buffers });

    Ok(())
}
