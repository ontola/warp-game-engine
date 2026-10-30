//! OpenXR owns its swapchain images. Warp/wgpu owns the rendering device.
//! The legacy Vulkan extension permits app-created devices; we enable every
//! runtime-required extension and select exactly the physical device it returns.
use anyhow::{Context, Result, ensure};
use ash::{vk, vk::Handle};
use openxr as xr;
use render::Gpu;
use std::ffi::CString;

pub struct Bridge {
    // Keep the wgpu owner alive until after native command objects are destroyed.
    pub gpu: Gpu,
    device: ash::Device,
    queue: vk::Queue,
    pool: vk::CommandPool,
    command: vk::CommandBuffer,
    fence: vk::Fence,
}
fn extensions(names: &str) -> Vec<&'static std::ffi::CStr> {
    // A bounded set allocated once at startup; HAL requires static names.
    names.split_whitespace().map(|s| &*Box::leak(CString::new(s).unwrap().into_boxed_c_str())).collect()
}
impl Bridge {
    pub unsafe fn new(xr: &xr::Instance, system: xr::SystemId) -> Result<(Self, xr::vulkan::SessionCreateInfo)> {
        unsafe {
            let req = xr.graphics_requirements::<xr::Vulkan>(system)?;
            let version = xr::Version::new(1, 1, 0);
            ensure!(version >= req.min_api_version_supported && version <= req.max_api_version_supported,
                "OpenXR runtime does not support Vulkan 1.1");
            let entry = ash::Entry::load()?;
            let flags = wgpu::InstanceFlags::empty();
            let mut enabled = wgpu::hal::vulkan::Instance::desired_extensions(&entry, vk::API_VERSION_1_1, flags)?;
            for ext in extensions(&xr.vulkan_legacy_instance_extensions(system)?) {
                if !enabled.contains(&ext) { enabled.push(ext); }
            }
            let pointers: Vec<_> = enabled.iter().map(|x| x.as_ptr()).collect();
            let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_1);
            let raw = entry.create_instance(&vk::InstanceCreateInfo::default().application_info(&app).enabled_extension_names(&pointers), None)?;
            let physical = vk::PhysicalDevice::from_raw(xr.vulkan_graphics_device(system, raw.handle().as_raw() as _)? as _);
            // HAL uses queue family zero. Reject incompatible runtimes before device creation.
            let families = raw.get_physical_device_queue_family_properties(physical);
            ensure!(families.first().is_some_and(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)), "OpenXR GPU queue 0 lacks graphics/compute");
            let instance_handle = raw.handle();
            let hal = wgpu::hal::vulkan::Instance::from_raw(entry, raw, vk::API_VERSION_1_1, 29, None,
                enabled, flags, Default::default(), false, None)?;
            let exposed = hal.expose_adapter(physical).context("OpenXR GPU is not supported by wgpu")?;
            let instance = wgpu::Instance::from_hal::<wgpu::hal::api::Vulkan>(hal);
            let adapter = instance.create_adapter_from_hal(exposed);
            let al = adapter.limits();
            ensure!(al.max_storage_buffers_per_shader_stage >= 16 && al.max_sampled_textures_per_shader_stage >= 17,
                "GPU limits below Warp requirements: storage buffers {}, sampled textures {}", al.max_storage_buffers_per_shader_stage, al.max_sampled_textures_per_shader_stage);
            let mut limits = wgpu::Limits::default().using_resolution(al.clone());
            limits.max_storage_buffer_binding_size = al.max_storage_buffer_binding_size;
            limits.max_buffer_size = al.max_buffer_size;
            limits.max_storage_buffers_per_shader_stage = 16;
            limits.max_sampled_textures_per_shader_stage = al.max_sampled_textures_per_shader_stage.min(32);
            limits.max_compute_workgroups_per_dimension = al.max_compute_workgroups_per_dimension;
            let features = adapter.features() & (wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS | wgpu::Features::INDIRECT_FIRST_INSTANCE | wgpu::Features::PRIMITIVE_INDEX | wgpu::Features::PIPELINE_CACHE);
            let desc = wgpu::DeviceDescriptor { label: Some("Warp OpenXR"), required_features: features,
                required_limits: limits, memory_hints: wgpu::MemoryHints::MemoryUsage, ..Default::default() };
            let extra = extensions(&xr.vulkan_legacy_device_extensions(system)?);
            let hal_adapter = adapter.as_hal::<wgpu::hal::api::Vulkan>().context("Vulkan adapter missing")?;
            let opened = hal_adapter.open_with_callback(features, &desc.required_limits, &desc.memory_hints,
                Some(Box::new(move |args| { for name in &extra { if !args.extensions.contains(name) { args.extensions.push(name); } } })))?;
            drop(hal_adapter);
            let (device, queue) = adapter.create_device_from_hal(opened, &desc)?;
            let gpu = Gpu::from_device(device, queue, adapter.get_info());
            let hal_device = gpu.device.as_hal::<wgpu::hal::api::Vulkan>().context("Vulkan device missing")?;
            let device = hal_device.raw_device().clone();
            let queue = hal_device.raw_queue();
            let family = hal_device.queue_family_index();
            let info = xr::vulkan::SessionCreateInfo { instance: instance_handle.as_raw() as _,
                physical_device: physical.as_raw() as _, device: device.handle().as_raw() as _, queue_family_index: family, queue_index: 0 };
            drop(hal_device);
            let pool = device.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(family).flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER), None)?;
            let command = device.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(pool).level(vk::CommandBufferLevel::PRIMARY).command_buffer_count(1))?[0];
            let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)?;
            Ok((Self { gpu, device, queue, pool, command, fence }, info))
        }
    }

    /// A single-threaded queue handoff: wgpu transitions the source, then a
    /// native submission copies into runtime-owned array layers. A fence makes
    /// completion explicit before xrReleaseSwapchainImage (no borrowed-image
    /// destruction, no ambiguous wgpu layout tracking, no CPU pixel readback).
    pub fn copy(&self, source: &wgpu::Texture, image: u64, width: u32, height: u32, layers: u32) -> Result<()> {
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        encoder.transition_resources(std::iter::empty(), [wgpu::TextureTransition { texture: source, selector: None, state: wgpu::TextureUses::COPY_SRC }].into_iter());
        self.gpu.queue.submit([encoder.finish()]);
        // Wait before using the raw queue: wgpu may have background submissions.
        self.gpu.wait();
        unsafe {
            let raw = source.as_hal::<wgpu::hal::api::Vulkan>().context("source is not Vulkan")?;
            let source_image = raw.raw_handle();
            let dest = vk::Image::from_raw(image);
            self.device.reset_command_buffer(self.command, vk::CommandBufferResetFlags::empty())?;
            self.device.begin_command_buffer(self.command, &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT))?;
            let range = vk::ImageSubresourceRange::default().aspect_mask(vk::ImageAspectFlags::COLOR).level_count(1).layer_count(layers);
            let barrier = |old, new, src, dst| vk::ImageMemoryBarrier::default().image(dest).subresource_range(range)
                .old_layout(old).new_layout(new).src_access_mask(src).dst_access_mask(dst)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED);
            self.device.cmd_pipeline_barrier(self.command, vk::PipelineStageFlags::TOP_OF_PIPE, vk::PipelineStageFlags::TRANSFER, vk::DependencyFlags::empty(), &[], &[],
                &[barrier(vk::ImageLayout::UNDEFINED, vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::AccessFlags::empty(), vk::AccessFlags::TRANSFER_WRITE)]);
            let regions: Vec<_> = (0..layers).map(|layer| vk::ImageCopy::default()
                .src_subresource(vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).layer_count(1))
                .src_offset(vk::Offset3D { x: (layer*width) as i32, y: 0, z: 0 })
                .dst_subresource(vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).base_array_layer(layer).layer_count(1))
                .extent(vk::Extent3D { width, height, depth: 1 })).collect();
            self.device.cmd_copy_image(self.command, source_image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, dest, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &regions);
            self.device.cmd_pipeline_barrier(self.command, vk::PipelineStageFlags::TRANSFER, vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT, vk::DependencyFlags::empty(), &[], &[],
                &[barrier(vk::ImageLayout::TRANSFER_DST_OPTIMAL, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL, vk::AccessFlags::TRANSFER_WRITE, vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE)]);
            self.device.end_command_buffer(self.command)?;
            self.device.queue_submit(self.queue, &[vk::SubmitInfo::default().command_buffers(&[self.command])], self.fence)?;
            self.device.wait_for_fences(&[self.fence], true, u64::MAX)?;
            self.device.reset_fences(&[self.fence])?;
        }
        Ok(())
    }
}
impl Drop for Bridge {
    fn drop(&mut self) { unsafe { let _ = self.device.device_wait_idle(); self.device.destroy_fence(self.fence,None); self.device.destroy_command_pool(self.pool,None); } }
}
