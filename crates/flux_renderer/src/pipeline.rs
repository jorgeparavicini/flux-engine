use crate::device::Device;
use crate::swapchain::Swapchain;
use ash::vk;
use flux_ecs::commands::Commands;
use flux_ecs::resource::{Res, Resource};
use std::{io, slice};
// TODO: Error handling is just a placeholder, needs to be improved

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct Vertex {
    pos: [f32; 3],
    color: [f32; 3],
    tex_coords: [f32; 2],
}

pub struct Pipeline {
    pub pipeline: vk::Pipeline,
    // TODO: Not sure if this belongs here
    pub descriptor_set_layout: vk::DescriptorSetLayout,
    pub pipeline_layout: vk::PipelineLayout,
}

impl Resource for Pipeline {}

pub fn create_pipeline(
    device: Res<Device>,
    swapchain: Res<Swapchain>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    let vertex_shader_module =
        create_shader_module(&device, &include_bytes!("../shaders/vert.spv")[..])?;
    let frag_shader_module =
        create_shader_module(&device, &include_bytes!("../shaders/frag.spv")[..])?;

    let vert_stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::VERTEX)
        .module(vertex_shader_module)
        .name(c"main");

    let frag_stage = vk::PipelineShaderStageCreateInfo::default()
        .stage(vk::ShaderStageFlags::FRAGMENT)
        .module(frag_shader_module)
        .name(c"main");

    let vertex_binding_descriptions = [vk::VertexInputBindingDescription::default()
        .binding(0)
        .stride(size_of::<Vertex>() as u32)
        .input_rate(vk::VertexInputRate::VERTEX)];

    let vertex_attribute_descriptions = [
        // Position attribute
        vk::VertexInputAttributeDescription::default()
            .binding(0)
            .location(0)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(0),
        // Color attribute
        vk::VertexInputAttributeDescription::default()
            .binding(0)
            .location(1)
            .format(vk::Format::R32G32B32_SFLOAT)
            .offset(size_of::<[f32; 3]>() as u32),
        // Texture coordinates attribute
        vk::VertexInputAttributeDescription::default()
            .binding(0)
            .location(2)
            .format(vk::Format::R32G32_SFLOAT)
            .offset((size_of::<[f32; 3]>() + size_of::<[f32; 3]>()) as u32),
    ];

    let vertex_input_info = vk::PipelineVertexInputStateCreateInfo::default()
        .vertex_binding_descriptions(&vertex_binding_descriptions)
        .vertex_attribute_descriptions(&vertex_attribute_descriptions);

    let input_assembly_info = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
        .primitive_restart_enable(false);

    let viewport = vk::Viewport::default()
        .x(0.0)
        .y(0.0)
        .width(swapchain.extent.width as f32)
        .height(swapchain.extent.height as f32)
        .min_depth(0.0)
        .max_depth(1.0);

    let scissor = vk::Rect2D::default()
        .offset(vk::Offset2D::default())
        .extent(swapchain.extent);

    let viewports = &[viewport];
    let scissors = &[scissor];
    let viewport_state_info = vk::PipelineViewportStateCreateInfo::default()
        .viewports(viewports)
        .scissors(scissors);

    let rasterization_state = vk::PipelineRasterizationStateCreateInfo::default()
        .depth_clamp_enable(false)
        .rasterizer_discard_enable(false)
        .polygon_mode(vk::PolygonMode::FILL)
        .line_width(1.0)
        .cull_mode(vk::CullModeFlags::BACK)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .depth_bias_enable(false);

    let multisample_state = vk::PipelineMultisampleStateCreateInfo::default()
        .sample_shading_enable(false)
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);

    let depth_stencil_state = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(true)
        .depth_compare_op(vk::CompareOp::LESS)
        .depth_bounds_test_enable(false)
        .min_depth_bounds(0.0)
        .max_depth_bounds(1.0)
        .stencil_test_enable(false);

    let attachment = vk::PipelineColorBlendAttachmentState::default()
        .color_write_mask(vk::ColorComponentFlags::RGBA)
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ZERO)
        .alpha_blend_op(vk::BlendOp::ADD);

    let attachments = &[attachment];
    let color_blend_state = vk::PipelineColorBlendStateCreateInfo::default()
        .logic_op_enable(false)
        .logic_op(vk::LogicOp::COPY)
        .attachments(attachments)
        .blend_constants([0.0, 0.0, 0.0, 0.0]);

    let ubo_binding = vk::DescriptorSetLayoutBinding::default()
        .binding(0)
        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::VERTEX);

    let sampler_binding = vk::DescriptorSetLayoutBinding::default()
        .binding(1)
        .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
        .descriptor_count(1)
        .stage_flags(vk::ShaderStageFlags::FRAGMENT);

    let bindings = &[ubo_binding, sampler_binding];
    let descriptor_set_layout_create_info =
        vk::DescriptorSetLayoutCreateInfo::default().bindings(bindings);

    let descriptor_set_layout =
        unsafe { device.create_descriptor_set_layout(&descriptor_set_layout_create_info, None) }?;

    let descriptor_set_layouts = &[descriptor_set_layout];
    let layout_create_info = vk::PipelineLayoutCreateInfo::default()
        .set_layouts(descriptor_set_layouts)
        .push_constant_ranges(&[]);

    let pipeline_layout = unsafe { device.create_pipeline_layout(&layout_create_info, None) }?;

    let stages = &[vert_stage, frag_stage];

    let color_attachment_formats = &[swapchain.format.format];
    let mut rendering_info = vk::PipelineRenderingCreateInfo::default()
        .color_attachment_formats(color_attachment_formats)
        .depth_attachment_format(vk::Format::D32_SFLOAT_S8_UINT)
        .stencil_attachment_format(vk::Format::D32_SFLOAT_S8_UINT);

    let info = vk::GraphicsPipelineCreateInfo::default()
        .stages(stages)
        .vertex_input_state(&vertex_input_info)
        .input_assembly_state(&input_assembly_info)
        .viewport_state(&viewport_state_info)
        .rasterization_state(&rasterization_state)
        .multisample_state(&multisample_state)
        .depth_stencil_state(&depth_stencil_state)
        .color_blend_state(&color_blend_state)
        .layout(pipeline_layout)
        .push_next(&mut rendering_info);

    let pipelines =
        unsafe { device.create_graphics_pipelines(vk::PipelineCache::null(), &[info], None) }
            // TODO: This is just to get it to compile, needs proper error handling
            .map_err(|e| e.1)?;

    unsafe {
        device.destroy_shader_module(vertex_shader_module, None);
        device.destroy_shader_module(frag_shader_module, None);
    }

    let pipeline = Pipeline {
        pipeline: pipelines[0],
        descriptor_set_layout,
        pipeline_layout,
    };

    commands.insert_resource(pipeline);

    Ok(())
}

// TODO: Use Rust-GPU

fn create_shader_module(device: &Device, code: &[u8]) -> Result<vk::ShaderModule, vk::Result> {
    let code = read_spv(&mut io::Cursor::new(code))
        .map_err(|_| vk::Result::ERROR_INITIALIZATION_FAILED)?;

    let create_info = vk::ShaderModuleCreateInfo::default().code(&code);
    unsafe { device.create_shader_module(&create_info, None) }
}

fn read_spv<R: io::Read + io::Seek>(x: &mut R) -> io::Result<Vec<u32>> {
    let size = x.seek(io::SeekFrom::End(0))?;
    x.rewind()?;
    if !size.is_multiple_of(4) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "SPIR-V size is not a multiple of 4",
        ));
    }

    if size > usize::MAX as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "SPIR-V size exceeds usize::MAX",
        ));
    }

    let words = (size / 4) as usize;
    let mut result = vec![0u32; words];
    x.read_exact(unsafe {
        slice::from_raw_parts_mut(result.as_mut_ptr().cast::<u8>(), words * 4)
    })?;

    const MAGIC_NUMBER: u32 = 0x0723_0203;
    if !result.is_empty() && result[0] == MAGIC_NUMBER.swap_bytes() {
        for word in &mut result {
            *word = word.swap_bytes();
        }
    }

    if result.is_empty() || result[0] != MAGIC_NUMBER {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid SPIR-V magic number",
        ));
    }

    Ok(result)
}

pub fn destroy_pipeline(
    device: Res<Device>,
    pipeline: Res<Pipeline>,
    mut commands: Commands,
) -> Result<(), vk::Result> {
    unsafe {
        device.destroy_pipeline(pipeline.pipeline, None);
        device.destroy_descriptor_set_layout(pipeline.descriptor_set_layout, None);
        device.destroy_pipeline_layout(pipeline.pipeline_layout, None);
    }

    commands.remove_resource::<Pipeline>();

    Ok(())
}
