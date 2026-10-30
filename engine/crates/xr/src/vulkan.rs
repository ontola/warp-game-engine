//! OpenXR owns its swapchain images. Warp/wgpu owns the rendering device.
//! The legacy Vulkan extension permits app-created devices; we enable every
//! runtime-required extension and select exactly the physical device it returns.
use anyhow::{Context, Result, ensure};
use ash::{vk, vk::Handle};
use openxr as xr;
use render::Gpu;
use std::ffi::CString;

pub struct Bridge { pub gpu: Gpu }
/// Declared after its OpenXR swapchain, so wrappers and their GPU work are
/// retired before the runtime frees the images, including on error returns.
pub struct ExternalTargets { pub images:Vec<wgpu::Texture>, device:wgpu::Device }
impl Drop for ExternalTargets {
    fn drop(&mut self) {
        let _=self.device.poll(wgpu::PollType::wait_indefinitely());
        self.images.clear();
        let _=self.device.poll(wgpu::PollType::wait_indefinitely());
    }
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
            let family = hal_device.queue_family_index();
            let info = xr::vulkan::SessionCreateInfo { instance: instance_handle.as_raw() as _,
                physical_device: physical.as_raw() as _, device: device.handle().as_raw() as _, queue_family_index: family, queue_index: 0 };
            drop(hal_device);
            Ok((Self { gpu }, info))
        }
    }

    /// OpenXR guarantees COLOR_ATTACHMENT_OPTIMAL after xrWaitSwapchainImage.
    /// The no-op callback and External memory keep ownership with OpenXR.
    pub unsafe fn import_targets(&self,images:&[u64],size:[u32;2])->Result<ExternalTargets> {
        let desc=wgpu::TextureDescriptor {label:Some("OpenXR eye attachment"),
            size:wgpu::Extent3d{width:size[0],height:size[1],depth_or_array_layers:1},mip_level_count:1,sample_count:1,
            dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8UnormSrgb,
            usage:wgpu::TextureUsages::RENDER_ATTACHMENT,view_formats:&[]};
        let hal_desc=wgpu::hal::TextureDescriptor {label:desc.label,size:desc.size,mip_level_count:1,sample_count:1,
            dimension:desc.dimension,format:desc.format,usage:wgpu::TextureUses::COLOR_TARGET,
            memory_flags:wgpu::hal::MemoryFlags::empty(),view_formats:Vec::new()};
        let mut out=Vec::new();
        for &image in images {unsafe {
            let device=self.gpu.device.as_hal::<wgpu::hal::api::Vulkan>().context("Vulkan device missing")?;
            let texture=device.texture_from_raw(vk::Image::from_raw(image),&hal_desc,Some(Box::new(||{})),wgpu::hal::vulkan::TextureMemory::External);
            drop(device);
            out.push(self.gpu.device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(texture,&desc,wgpu::TextureUses::COLOR_TARGET));
        }}
        Ok(ExternalTargets{images:out,device:self.gpu.device.clone()})
    }
}
