use crate::device::instance::Instance;
use crate::device::logical::RenderDevice;
use crate::error::RendererError;
use crate::present::policy;
use crate::present::surface::VulkanSurface;
use crate::resource::image::create_image_view;
use ash::vk::SwapchainCreateInfoKHR;
use ash::{khr, vk};
use flux_ecs::{Commands, Single};
use flux_renderer_api::surface::PresentTarget;
use log::info;

/// How many frames the CPU may record ahead of the GPU.
pub const FRAMES_IN_FLIGHT: usize = 2;

/// One frame-in-flight's synchronization objects.
struct FrameSlot {
    image_available: vk::Semaphore,
    in_flight: vk::Fence,
}

/// A frame the renderer may record into and present.
pub struct Frame {
    /// Which swapchain image was acquired.
    pub image_index: u32,
    /// Signaled when the image is ready to be rendered to; wait on it before
    /// color output.
    pub image_available: vk::Semaphore,
    /// Signal this when rendering finishes; presentation waits on it.
    pub render_finished: vk::Semaphore,
    /// Signal this at submit; the slot's next acquire waits on it.
    pub in_flight: vk::Fence,
}

/// The outcome of asking for a frame.
pub enum Acquire {
    /// An image is ready to be rendered.
    Frame(Frame),
    /// The swapchain no longer matches the surface; rebuild and retry next
    /// frame.
    OutOfDate,
}

/// Owns the surface, swapchain, and frame pacing. Created once, rebuilt in
/// place whenever the surface changes, torn down as one unit on drop.
#[derive(flux_ecs::Component)]
pub struct Presenter {
    // Drop needs these; the presenter owns everything its destruction uses.
    device: ash::Device,
    surface_loader: khr::surface::Instance,
    swapchain_loader: khr::swapchain::Device,

    surface: vk::SurfaceKHR,
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    pub format: vk::SurfaceFormatKHR,
    pub extent: vk::Extent2D,

    slots: Vec<FrameSlot>,
    /// Per swapchain image: presentation waits on the one for the acquired
    /// image.
    render_finished: Vec<vk::Semaphore>,
    /// Per swapchain image: the in-flight fence of the slot that last used
    /// it, or null.
    images_in_flight: Vec<vk::Fence>,
    current_slot: usize,
    needs_rebuild: bool,
}

impl Presenter {
    /// Creates the presenter, taking ownership of the surface. The swapchain
    /// is negotiated against the surface's current state.
    pub fn new(
        instance: &Instance,
        device: &RenderDevice,
        target: &PresentTarget,
        surface: vk::SurfaceKHR,
    ) -> Result<Self, RendererError> {
        let surface_loader = khr::surface::Instance::new(instance.entry(), instance);
        let swapchain_loader = khr::swapchain::Device::new(instance, device);

        let bundle = SwapchainBundle::new(
            device,
            target,
            &surface_loader,
            &swapchain_loader,
            surface,
            vk::SwapchainKHR::null(),
        )?;

        let fence_info = vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED);
        let semaphore_info = vk::SemaphoreCreateInfo::default();
        let mut slots = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for i in 0..FRAMES_IN_FLIGHT {
            let image_available = unsafe { device.create_semaphore(&semaphore_info, None)? };
            device
                .debug_utils
                .set_name(image_available, &format!("image available semaphore {i}"));
            let in_flight = unsafe { device.create_fence(&fence_info, None)? };
            device
                .debug_utils
                .set_name(in_flight, &format!("in flight fence {i}"));
            slots.push(FrameSlot {
                image_available,
                in_flight,
            });
        }

        Ok(Self {
            device: (**device).clone(),
            surface_loader,
            swapchain_loader,
            surface,
            swapchain: bundle.swapchain,
            images: bundle.images,
            views: bundle.views,
            format: bundle.format,
            extent: bundle.extent,
            slots,
            render_finished: bundle.render_finished,
            images_in_flight: bundle.images_in_flight,
            current_slot: 0,
            needs_rebuild: false,
        })
    }

    /// Rebuilds the swapchain for the surface's current state, re-negotiating
    /// format, mode, and extent. Waits for the device to go idle, accepted as
    /// a deliberate hitch on resize. Frame pacing and the surface survive.
    pub fn rebuild(
        &mut self,
        device: &RenderDevice,
        target: &PresentTarget,
    ) -> Result<(), RendererError> {
        device.wait_idle();

        unsafe {
            for &view in &self.views {
                self.device.destroy_image_view(view, None);
            }
            for &semaphore in &self.render_finished {
                self.device.destroy_semaphore(semaphore, None);
            }
        }

        let bundle = SwapchainBundle::new(
            device,
            target,
            &self.surface_loader,
            &self.swapchain_loader,
            self.surface,
            self.swapchain,
        )?;

        unsafe {
            self.swapchain_loader
                .destroy_swapchain(self.swapchain, None);
        }

        self.swapchain = bundle.swapchain;
        self.views = bundle.views;
        self.render_finished = bundle.render_finished;
        self.format = bundle.format;
        self.extent = bundle.extent;
        self.images_in_flight = bundle.images_in_flight;
        self.images = bundle.images;
        self.needs_rebuild = false;

        Ok(())
    }

    /// Acquires the next image, blocking until the GPU frees the current
    /// in-flight slot.
    ///
    /// Every returned [`Frame`] must be submitted with its `in_flight` fence
    /// as the submit fence: the fence is handed out unsignaled, and the
    /// slot's next acquire waits on it.
    pub fn acquire(&mut self) -> Result<Acquire, RendererError> {
        let image_available = self.slots[self.current_slot].image_available;
        let in_flight = self.slots[self.current_slot].in_flight;

        unsafe { self.device.wait_for_fences(&[in_flight], true, u64::MAX)? };

        let acquired = unsafe {
            self.swapchain_loader.acquire_next_image(
                self.swapchain,
                u64::MAX,
                image_available,
                vk::Fence::null(),
            )
        };

        let (image_index, suboptimal) = match acquired {
            Ok(pair) => pair,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.needs_rebuild = true;
                return Ok(Acquire::OutOfDate);
            }
            Err(e) => return Err(e.into()),
        };

        if suboptimal {
            self.needs_rebuild = true;
        }

        let image_fence = self.images_in_flight[image_index as usize];
        if image_fence != vk::Fence::null() {
            unsafe {
                self.device
                    .wait_for_fences(&[image_fence], true, u64::MAX)?;
            }
        }

        self.images_in_flight[image_index as usize] = in_flight;

        unsafe { self.device.reset_fences(&[in_flight])? };

        Ok(Acquire::Frame(Frame {
            image_index,
            image_available,
            render_finished: self.render_finished[image_index as usize],
            in_flight,
        }))
    }

    /// Presents a frame and advances the slot. Out-of-date or suboptimal
    /// surfaces set `needs_rebuild` instead of failing.
    pub fn present(&mut self, device: &RenderDevice, frame: &Frame) -> Result<(), RendererError> {
        let wait_semaphore = [frame.render_finished];
        let swapchains = [self.swapchain];
        let image_indices = [frame.image_index];

        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&wait_semaphore)
            .swapchains(&swapchains)
            .image_indices(&image_indices);

        let result = unsafe {
            self.swapchain_loader
                .queue_present(device.present.raw, &present_info)
        };

        match result {
            Ok(false) => {}
            Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => self.needs_rebuild = true,
            Err(e) => return Err(e.into()),
        }

        self.current_slot = next_slot(self.current_slot);

        Ok(())
    }

    /// True when the swapchain no longer matches the surface and
    /// [`Self::rebuild`] should run before the next frame.
    #[must_use]
    pub const fn needs_rebuild(&self) -> bool {
        self.needs_rebuild
    }

    #[must_use]
    pub const fn image_count(&self) -> usize {
        self.images.len()
    }

    #[must_use]
    pub fn view(&self, image_index: u32) -> vk::ImageView {
        self.views[image_index as usize]
    }

    #[must_use]
    pub fn image(&self, image_index: u32) -> vk::Image {
        self.images[image_index as usize]
    }
}

impl Drop for Presenter {
    fn drop(&mut self) {
        info!("Destroying presenter");
        unsafe {
            for slot in &self.slots {
                self.device.destroy_semaphore(slot.image_available, None);
                self.device.destroy_fence(slot.in_flight, None);
            }
            for &semaphore in &self.render_finished {
                self.device.destroy_semaphore(semaphore, None);
            }
            for &view in &self.views {
                self.device.destroy_image_view(view, None);
            }
            self.swapchain_loader
                .destroy_swapchain(self.swapchain, None);
            self.surface_loader.destroy_surface(self.surface, None);
        }
    }
}

struct SwapchainBundle {
    swapchain: vk::SwapchainKHR,
    images: Vec<vk::Image>,
    views: Vec<vk::ImageView>,
    format: vk::SurfaceFormatKHR,
    extent: vk::Extent2D,
    render_finished: Vec<vk::Semaphore>,
    images_in_flight: Vec<vk::Fence>,
}

impl SwapchainBundle {
    fn new(
        device: &RenderDevice,
        target: &PresentTarget,
        surface_loader: &khr::surface::Instance,
        swapchain_loader: &khr::swapchain::Device,
        surface: vk::SurfaceKHR,
        old_swapchain: vk::SwapchainKHR,
    ) -> Result<Self, RendererError> {
        let capabilities = unsafe {
            surface_loader.get_physical_device_surface_capabilities(device.physical, surface)?
        };
        let formats = unsafe {
            surface_loader.get_physical_device_surface_formats(device.physical, surface)?
        };
        let present_modes = unsafe {
            surface_loader.get_physical_device_surface_present_modes(device.physical, surface)?
        };

        let format = policy::choose_surface_format(&formats);
        let present_mode = policy::choose_present_mode(&present_modes);
        let image_count = policy::choose_image_count(&capabilities);
        let extent = policy::choose_extent(&capabilities, target.extent());

        let mut queue_family_indices = vec![];
        let image_sharing_mode = if device.graphics.family == device.present.family {
            vk::SharingMode::EXCLUSIVE
        } else {
            queue_family_indices.push(device.graphics.family);
            queue_family_indices.push(device.present.family);
            vk::SharingMode::CONCURRENT
        };

        let swapchain_create_info = SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(image_sharing_mode)
            .queue_family_indices(&queue_family_indices)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
            .present_mode(present_mode)
            .clipped(true)
            .old_swapchain(old_swapchain);

        let swapchain = unsafe { swapchain_loader.create_swapchain(&swapchain_create_info, None)? };
        let images = unsafe { swapchain_loader.get_swapchain_images(swapchain)? };

        let views = images
            .iter()
            .map(|&image| {
                create_image_view(device, image, format.format, vk::ImageAspectFlags::COLOR)
            })
            .collect::<Result<Vec<_>, _>>()?;

        device.debug_utils.set_name(swapchain, "main swapchain");
        for (i, image) in images.iter().enumerate() {
            device
                .debug_utils
                .set_name(*image, &format!("swapchain image {i}"));
        }
        for (i, view) in views.iter().enumerate() {
            device
                .debug_utils
                .set_name(*view, &format!("swapchain image view {i}"));
        }

        let semaphore_info = vk::SemaphoreCreateInfo::default();

        let mut render_finished = Vec::with_capacity(images.len());
        for i in 0..images.len() {
            let semaphore = unsafe { device.create_semaphore(&semaphore_info, None)? };
            device
                .debug_utils
                .set_name(semaphore, &format!("render finished semaphore {i}"));
            render_finished.push(semaphore);
        }
        let images_in_flight = vec![vk::Fence::null(); images.len()];

        Ok(Self {
            swapchain,
            images,
            views,
            format,
            extent,
            render_finished,
            images_in_flight,
        })
    }
}

/// Creates the [`Presenter`] singleton, taking the surface over from the
/// [`VulkanSurface`] singleton, whose job ends here.
pub fn create_presenter(
    instance: Single<&Instance>,
    device: Single<&RenderDevice>,
    target: Single<&PresentTarget>,
    surface: Single<&VulkanSurface>,
    mut commands: Commands,
) -> Result<(), RendererError> {
    let presenter = Presenter::new(&instance, &device, &target, **surface)?;
    commands.insert_singleton(presenter);
    commands.remove_singleton::<VulkanSurface>();
    Ok(())
}

/// Rebuilds the presenter for the surface's current state. A zero-sized
/// surface (minimized window) is skipped; the outdated flag stays set and
/// rendering stays paused until the window has an extent again.
pub fn rebuild_presenter(
    device: Single<&RenderDevice>,
    target: Single<&PresentTarget>,
    mut presenter: Single<&mut Presenter>,
) -> Result<(), RendererError> {
    let (width, height) = target.extent();
    if width == 0 || height == 0 {
        return Ok(());
    }
    presenter.rebuild(&device, &target)
}

/// Removes the [`Presenter`] singleton; destruction happens in its `Drop`.
pub fn destroy_presenter(mut commands: Commands) {
    commands.remove_singleton::<Presenter>();
}

/// The slot that follows `current`, wrapping at `FRAMES_IN_FLIGHT`.
const fn next_slot(current: usize) -> usize {
    (current + 1) % FRAMES_IN_FLIGHT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_rotate_and_wrap() {
        let mut slot = 0;
        let mut seen = Vec::new();
        for _ in 0..(FRAMES_IN_FLIGHT * 2) {
            seen.push(slot);
            slot = next_slot(slot);
        }
        assert_eq!(slot, 0, "rotation wraps back to the first slot");
        assert!(seen.iter().all(|&s| s < FRAMES_IN_FLIGHT));
        assert_eq!(
            seen[..FRAMES_IN_FLIGHT],
            seen[FRAMES_IN_FLIGHT..],
            "every slot is visited once per cycle"
        );
    }
}
