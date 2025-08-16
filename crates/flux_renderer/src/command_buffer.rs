use crate::buffers::{IndexBuffer, VertexBuffer};
use crate::command_pool::CommandPools;
use crate::depth_buffers::DepthBuffers;
use crate::descriptors::Descriptors;
use crate::device::Device;
use crate::pipeline::Pipeline;
use crate::swapchain::Swapchain;
use ash::vk;
use flux_ecs::resource::Res;
use log::debug;

pub fn create_command_buffer(
    device: Res<Device>,
    command_pools: Res<CommandPools>,
    swapchain: Res<Swapchain>,
    depth_buffers: Res<DepthBuffers>,
    pipeline: Res<Pipeline>,
    vertex_buffer: Res<VertexBuffer>,
    index_buffer: Res<IndexBuffer>,
    descriptors: Res<Descriptors>,
) -> Result<(), vk::Result> {
    debug!("Creating command buffer");

    let allocate_info = vk::CommandBufferAllocateInfo::default()
        .command_pool(command_pools.graphics)
        .level(vk::CommandBufferLevel::PRIMARY)
        .command_buffer_count(1);

    let command_buffer = unsafe { device.allocate_command_buffers(&allocate_info)? };

    for (i, command_buffer) in command_buffer.iter().enumerate() {
        let inheritance = vk::CommandBufferInheritanceInfo::default();

        let info = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::empty())
            .inheritance_info(&inheritance);

        unsafe {
            device.begin_command_buffer(*command_buffer, &info)?;
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

            device.cmd_draw_indexed(*command_buffer, 3, 1, 0, 0, 0);

            device.cmd_end_rendering(*command_buffer);
            device.end_command_buffer(*command_buffer)?;
        }
    }

    Ok(())
}
