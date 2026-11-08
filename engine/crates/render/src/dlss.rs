//! NVIDIA DLSS Super Resolution through the official NGX Vulkan SDK.
//!
//! Native commands are recorded in raw-only command buffers between ordinary
//! wgpu command buffers, with explicit tracked resource transitions. There is no alternate reconstruction hidden behind
//! this API: an unavailable SDK, device, feature or evaluation is an error.

use crate::{gpu::Gpu, upscale::UpscaleMode};

pub struct Inputs<'a> {
    pub color: &'a wgpu::Texture,
    /// Reversed hardware depth (zero at infinity), merged for visible translucency.
    pub depth: &'a wgpu::Texture,
    /// Low-resolution previous-minus-current pixel displacement, excluding jitter.
    pub motion: &'a wgpu::Texture,
    pub bias: Option<&'a wgpu::Texture>,
    pub jitter: [f32; 2],
    pub reset: bool,
    pub exposure: f32,
    pub frame_time_ms: f32,
}

fn quality(mode: UpscaleMode) -> Result<u32, String> {
    match mode {
        UpscaleMode::DlssQuality => Ok(2),
        UpscaleMode::DlssBalanced => Ok(1),
        UpscaleMode::DlssPerformance => Ok(0),
        _ => Err(format!("{} is not an NGX reconstruction mode", mode.as_str())),
    }
}

fn validate_parameters(input: (u32, u32), output: (u32, u32), jitter: [f32; 2], exposure: f32, frame_ms: f32) -> Result<(), String> {
    if input.0 == 0 || input.1 == 0 || output.0 == 0 || output.1 == 0 || input.0 > output.0 || input.1 > output.1 {
        return Err(format!("invalid DLSS input/output dimensions {input:?} -> {output:?}"));
    }
    if !jitter.iter().all(|x| x.is_finite()) || !exposure.is_finite() || exposure <= 0.0 || !frame_ms.is_finite() || frame_ms <= 0.0 {
        return Err("DLSS requires finite jitter, positive exposure and positive frame_time_ms".into());
    }
    Ok(())
}

#[cfg(all(feature = "dlss", target_os = "windows"))]
mod native {
    use super::*;
    use ash::{vk, vk::Handle};
    use std::{collections::HashMap, ffi::{c_char, c_void, CStr, CString}, os::windows::ffi::OsStrExt,
        path::PathBuf, ptr::NonNull, sync::{Arc, Condvar, Mutex, OnceLock, Weak}};

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct Image { image: u64, view: u64, format: u32, width: u32, height: u32 }
    #[repr(C)]
    struct Eval { color: Image, depth: Image, motion: Image, output: Image, bias: Image, exposure: Image,
        jitter_x: f32, jitter_y: f32, frame_ms: f32, reset: u32 }
    unsafe extern "C" {
        fn kern_dlss_extensions(instance: u64, physical: u64, runtime: *const u16, logs: *const u16,
            names: *mut c_char, slots: u32, count: *mut u32, error: *mut c_char, cap: u32) -> i32;
        fn kern_dlss_init(instance: u64, physical: u64, device: u64, gipa: *const c_void, gdpa: *const c_void,
            runtime: *const u16, logs: *const u16, error: *mut c_char, cap: u32) -> *mut c_void;
        fn kern_dlss_shutdown(context: *mut c_void);
        fn kern_dlss_optimal(context: *mut c_void, quality: u32, width: u32, height: u32,
            sizes: *mut u32, error: *mut c_char, cap: u32) -> i32;
        fn kern_dlss_create(context: *mut c_void, command: u64, quality: u32, width: u32, height: u32,
            out_width: u32, out_height: u32, error: *mut c_char, cap: u32) -> *mut c_void;
        fn kern_dlss_release(feature: *mut c_void);
        fn kern_dlss_evaluate(feature: *mut c_void, command: u64, inputs: *const Eval, error: *mut c_char, cap: u32) -> i32;
    }
    const ERROR_CAP: usize = 1024;
    fn error_text(bytes: &[c_char; ERROR_CAP]) -> String {
        // The bridge always receives a zero-filled, bounded buffer.
        unsafe { CStr::from_ptr(bytes.as_ptr()) }.to_string_lossy().into_owned()
    }
    struct Paths { runtime: Vec<u16>, logs: Vec<u16> }
    impl Paths {
        fn new() -> Result<Self, String> {
            let runtime = std::env::var_os("KERN_DLSS_RUNTIME_DIR").map(PathBuf::from)
                .or_else(|| std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from)))
                .ok_or("cannot locate the DLSS runtime directory")?;
            if !runtime.join("nvngx_dlss.dll").is_file() {
                return Err(format!("DLSS release runtime missing at {}; set KERN_DLSS_RUNTIME_DIR (see render/native/DLSS.md)", runtime.display()));
            }
            let logs = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Kern/NGX");
            std::fs::create_dir_all(&logs).map_err(|e| format!("NGX log directory: {e}"))?;
            fn wide(path: PathBuf) -> Result<Vec<u16>, String> {
                let path = path.canonicalize().map_err(|e| format!("NGX path: {e}"))?;
                let mut w: Vec<u16> = path.as_os_str().encode_wide().collect();
                if w.contains(&0) { return Err("NGX path contains NUL".into()); }
                // Some Windows DLL loaders reject extended-path prefixes for discovery.
                if w.starts_with(&[92,92,63,92]) { w.drain(..4); }
                w.push(0); Ok(w)
            }
            Ok(Self { runtime: wide(runtime)?, logs: wide(logs)? })
        }
    }
    fn extension_names(instance: u64, physical: u64, paths: &Paths) -> Result<Vec<CString>, String> {
        let mut names = [0_i8; 32 * 256]; let mut count = 0; let mut error = [0; ERROR_CAP];
        let ok = unsafe { kern_dlss_extensions(instance, physical, paths.runtime.as_ptr(), paths.logs.as_ptr(),
            names.as_mut_ptr(), 32, &mut count, error.as_mut_ptr(), ERROR_CAP as u32) };
        if ok == 0 { return Err(error_text(&error)); }
        (0..count as usize).map(|i| {
            let slot = &names[i*256..(i+1)*256];
            let end = slot.iter().position(|b| *b == 0).ok_or("unterminated NGX extension name")?;
            CString::new(slot[..end].iter().map(|b| *b as u8).collect::<Vec<_>>()).map_err(|e| e.to_string())
        }).collect()
    }
    fn check_instance(shared: &wgpu::hal::vulkan::InstanceShared, required: &[CString]) -> Result<(), String> {
        for extension in required {
            let core11 = shared.instance_api_version() >= vk::API_VERSION_1_1 && matches!(extension.to_bytes(),
                b"VK_KHR_get_physical_device_properties2" | b"VK_KHR_external_memory_capabilities" | b"VK_KHR_external_semaphore_capabilities" | b"VK_KHR_external_fence_capabilities");
            if !core11 && !shared.extensions().contains(&extension.as_c_str()) {
                return Err(format!("DLSS requires Vulkan instance extension {} before instance creation", extension.to_string_lossy()));
            }
        }
        Ok(())
    }
    /// wgpu HAL requires extension names to outlive the device. Interning bounds
    /// permanent allocations to the small SDK extension vocabulary, not frames.
    fn intern_extension(name: CString) -> &'static CStr {
        static NAMES: OnceLock<Mutex<HashMap<Vec<u8>, &'static CStr>>> = OnceLock::new();
        let mut names = NAMES.get_or_init(Default::default).lock().unwrap();
        *names.entry(name.to_bytes().to_vec()).or_insert_with(|| Box::leak(name.into_boxed_c_str()))
    }
    pub(crate) fn request_device(adapter: &wgpu::Adapter, desc: &wgpu::DeviceDescriptor<'_>) -> Option<Result<(wgpu::Device, wgpu::Queue), String>> {
        if adapter.get_info().backend != wgpu::Backend::Vulkan || adapter.get_info().vendor != 0x10de { return None; }
        // A feature-enabled native executable remains usable without an SDK DLL.
        // Selecting DLSS later returns the explicit missing-runtime error.
        let paths = Paths::new().ok()?;
        Some((|| {
            let hal = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }.ok_or("missing Vulkan adapter")?;
            let shared = hal.shared_instance();
            check_instance(shared, &extension_names(0, 0, &paths)?)?;
            let required = extension_names(shared.raw_instance().handle().as_raw(), hal.raw_physical_device().as_raw(), &paths)?;
            let supported = unsafe { shared.raw_instance().enumerate_device_extension_properties(hal.raw_physical_device()) }.map_err(|e| format!("Vulkan extensions: {e}"))?;
            for required in &required {
                if !supported.iter().any(|p| unsafe { CStr::from_ptr(p.extension_name.as_ptr()) } == required.as_c_str()) {
                    return Err(format!("DLSS device extension unavailable: {}", required.to_string_lossy()));
                }
            }
            let required: Vec<_> = required.into_iter().map(intern_extension).collect();
            let open = unsafe { hal.open_with_callback(desc.required_features, &desc.required_limits, &desc.memory_hints,
                Some(Box::new(move |args| { for name in required { if !args.extensions.contains(&name) { args.extensions.push(name); } } }))) }
                .map_err(|e| format!("Vulkan device with DLSS extensions: {e}"))?;
            drop(hal);
            unsafe { adapter.create_device_from_hal(open, desc) }.map_err(|e| format!("wgpu DLSS Vulkan device: {e}"))
        })())
    }

    type Registry = (Mutex<HashMap<u64, Weak<NativeContext>>>, Condvar);
    fn registry() -> &'static Registry {
        static CONTEXTS: OnceLock<Registry> = OnceLock::new();
        CONTEXTS.get_or_init(Default::default)
    }
    struct NativeContext { pointer: NonNull<c_void>, device: wgpu::Device, key: u64 }
    // All NGX calls are serialized by the bridge's mutex. The device is retained
    // through the final feature and submission callback, including resize.
    unsafe impl Send for NativeContext {}
    unsafe impl Sync for NativeContext {}
    impl Drop for NativeContext { fn drop(&mut self) {
        // A dead Weak can exist while its destructor is running. New contexts
        // wait for removal rather than initializing before this Shutdown1.
        let (table, changed) = registry();
        let mut table = table.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { kern_dlss_shutdown(self.pointer.as_ptr()) };
        table.remove(&self.key); changed.notify_all();
    } }
    struct Feature { pointer: NonNull<c_void>, _context: Arc<NativeContext> }
    unsafe impl Send for Feature {}
    unsafe impl Sync for Feature {}
    impl Drop for Feature { fn drop(&mut self) { unsafe { kern_dlss_release(self.pointer.as_ptr()) }; } }
    #[derive(Clone)]
    struct BoundImage { texture: wgpu::Texture, view: wgpu::TextureView, raw: Image }
    impl BoundImage {
        fn new(texture: &wgpu::Texture) -> Result<Self, String> {
            let view = texture.create_view(&Default::default());
            let image = unsafe { texture.as_hal::<wgpu::hal::api::Vulkan>() }.ok_or("DLSS texture is not Vulkan")?;
            let image_view = unsafe { view.as_hal::<wgpu::hal::api::Vulkan>() }.ok_or("DLSS image view is not Vulkan")?;
            let format = match texture.format() {
                wgpu::TextureFormat::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
                wgpu::TextureFormat::Rg16Float => vk::Format::R16G16_SFLOAT,
                wgpu::TextureFormat::R32Float => vk::Format::R32_SFLOAT,
                other => return Err(format!("unsupported NGX texture format {other:?}")),
            };
            let raw = Image { image: unsafe { image.raw_handle() }.as_raw(), view: unsafe { image_view.raw_handle() }.as_raw(),
                format: format.as_raw() as u32, width: texture.width(), height: texture.height() };
            drop(image_view); drop(image);
            Ok(Self { texture: texture.clone(), view, raw })
        }
    }
    struct View {
        key: (u32, (u32,u32), (u32,u32)), feature: Option<Arc<Feature>>, output: BoundImage, exposure: BoundImage,
        inputs: Vec<BoundImage>, first: bool,
    }
    pub struct DlssContext { native: Arc<NativeContext>, views: Vec<Option<View>> }
    impl DlssContext {
        pub fn new(gpu: &Gpu) -> Result<Self, String> {
            if gpu.info.backend != wgpu::Backend::Vulkan { return Err("DLSS backend requires Vulkan; select WGPU_BACKEND=vulkan before GPU creation".into()); }
            if gpu.info.vendor != 0x10de { return Err("DLSS Super Resolution requires a supported NVIDIA RTX device".into()); }
            let paths = Paths::new()?;
            let hal = unsafe { gpu.device.as_hal::<wgpu::hal::api::Vulkan>() }.ok_or("missing Vulkan device")?;
            let raw_device = hal.raw_device().handle().as_raw();
            let (table, changed) = registry();
            let mut contexts = table.lock().map_err(|_| "NGX context registry poisoned")?;
            while let Some(previous) = contexts.get(&raw_device) {
                if let Some(native) = previous.upgrade() { return Ok(Self { native, views: Vec::new() }); }
                contexts = changed.wait(contexts).map_err(|_| "NGX context registry poisoned")?;
            }
            let shared = hal.shared_instance(); let instance = shared.raw_instance().handle().as_raw();
            check_instance(shared, &extension_names(0, 0, &paths)?)?;
            for extension in extension_names(instance, hal.raw_physical_device().as_raw(), &paths)? {
                if !hal.enabled_device_extensions().contains(&extension.as_c_str()) {
                    return Err(format!("Vulkan device was created without required DLSS extension {}; recreate through Gpu", extension.to_string_lossy()));
                }
            }
            let mut error = [0; ERROR_CAP];
            let pointer = unsafe { kern_dlss_init(instance, hal.raw_physical_device().as_raw(), raw_device,
                shared.entry().static_fn().get_instance_proc_addr as *const c_void,
                shared.raw_instance().fp_v1_0().get_device_proc_addr as *const c_void,
                paths.runtime.as_ptr(), paths.logs.as_ptr(), error.as_mut_ptr(), ERROR_CAP as u32) };
            let pointer = NonNull::new(pointer).ok_or_else(|| error_text(&error))?;
            let native = Arc::new(NativeContext { pointer, device: gpu.device.clone(), key: raw_device });
            contexts.insert(raw_device, Arc::downgrade(&native));
            Ok(Self { native, views: Vec::new() })
        }
        pub fn optimal_size(&self, mode: UpscaleMode, output: (u32,u32)) -> Result<(u32,u32), String> {
            let quality = quality(mode)?;
            if output.0 == 0 || output.1 == 0 { return Err("zero DLSS output dimension".into()); }
            let mut sizes = [0_u32;6]; let mut error = [0;ERROR_CAP];
            if unsafe { kern_dlss_optimal(self.native.pointer.as_ptr(), quality, output.0, output.1, sizes.as_mut_ptr(), error.as_mut_ptr(), ERROR_CAP as u32) } == 0 { return Err(error_text(&error)); }
            validate_parameters((sizes[0],sizes[1]), output, [0.0;2], 1.0, 16.0)?;
            Ok((sizes[0],sizes[1]))
        }
        /// Record exactly once per view in a submitted frame. Append `encoder.finish()`
        /// to `segments` and submit all segments together, in order, after the frame.
        /// On failure discard every segment and the encoder; its error is not a frame.
        pub fn evaluate(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, segments: &mut Vec<wgpu::CommandBuffer>, view_index: usize, mode: UpscaleMode,
            input: (u32,u32), output: (u32,u32), inputs: Inputs<'_>) -> Result<wgpu::Texture, String> {
            validate_parameters(input, output, inputs.jitter, inputs.exposure, inputs.frame_time_ms)?;
            let quality = quality(mode)?;
            if view_index >= 4 { return Err("DLSS supports at most four independently tracked views".into()); }
            if gpu.device != self.native.device { return Err("DLSS context belongs to a different GPU device".into()); }
            fn check(t: &wgpu::Texture, dims: (u32,u32), format: wgpu::TextureFormat, name: &str) -> Result<(),String> {
                if (t.width(),t.height()) != dims || t.format() != format || t.dimension() != wgpu::TextureDimension::D2 || t.depth_or_array_layers() != 1 || t.sample_count() != 1 || t.mip_level_count() != 1 || !t.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING) {
                    return Err(format!("DLSS {name} must be a sampled single-layer, single-mip {format:?} texture of size {dims:?}"));
                }
                Ok(())
            }
            check(inputs.color,input,wgpu::TextureFormat::Rgba16Float,"color")?;
            check(inputs.depth,input,wgpu::TextureFormat::R32Float,"depth")?;
            check(inputs.motion,input,wgpu::TextureFormat::Rg16Float,"motion")?;
            if let Some(bias)=inputs.bias { check(bias,input,wgpu::TextureFormat::R32Float,"bias-current-color")?; }
            while self.views.len() <= view_index { self.views.push(None); }
            let key = (quality,input,output);
            if self.views[view_index].as_ref().is_none_or(|v| v.key != key) {
                let make = |label, dims: (u32,u32), format, usage| gpu.device.create_texture(&wgpu::TextureDescriptor {
                    label:Some(label),size:wgpu::Extent3d {width:dims.0,height:dims.1,depth_or_array_layers:1},mip_level_count:1,sample_count:1,
                    dimension:wgpu::TextureDimension::D2,format,usage,view_formats:&[] });
                let output_image = BoundImage::new(&make("DLSS output",output,wgpu::TextureFormat::Rgba16Float,
                    wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::STORAGE_BINDING|wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC))?;
                let exposure = BoundImage::new(&make("DLSS explicit exposure",(1,1),wgpu::TextureFormat::R32Float,
                    wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::COPY_DST))?;
                // A native write cannot update wgpu's lazy-initialization tracker.
                // Initialize once through wgpu so subsequent sampling never clears it.
                { let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label:Some("DLSS output initialization"), color_attachments:&[Some(wgpu::RenderPassColorAttachment {
                        view:&output_image.view,resolve_target:None,depth_slice:None,ops:wgpu::Operations {load:wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),store:wgpu::StoreOp::Store} })],
                    depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None,multiview_mask:None }); }
                self.views[view_index] = Some(View {key,feature:None,output:output_image,exposure,inputs:Vec::new(),first:true});
            }
            let view = self.views[view_index].as_mut().unwrap();
            let mut textures = vec![inputs.color,inputs.depth,inputs.motion];
            if let Some(bias)=inputs.bias { textures.push(bias); }
            if view.inputs.len() != textures.len() || view.inputs.iter().zip(&textures).any(|(old,new)| &old.texture != *new) {
                view.inputs = textures.iter().map(|t| BoundImage::new(t)).collect::<Result<_,_>>()?;
            }
            gpu.queue.write_texture(wgpu::TexelCopyTextureInfo { texture:&view.exposure.texture,mip_level:0,origin:wgpu::Origin3d::ZERO,aspect:wgpu::TextureAspect::All },
                &inputs.exposure.to_ne_bytes(),wgpu::TexelCopyBufferLayout {offset:0,bytes_per_row:Some(4),rows_per_image:Some(1)},wgpu::Extent3d {width:1,height:1,depth_or_array_layers:1});
            encoder.transition_resources(std::iter::empty(), view.inputs.iter().chain(std::iter::once(&view.exposure)).map(|i| wgpu::TextureTransition {
                texture:&i.texture,selector:None,state:wgpu::TextureUses::RESOURCE }).chain(std::iter::once(wgpu::TextureTransition {
                    texture:&view.output.texture,selector:None,state:wgpu::TextureUses::STORAGE_READ_WRITE })));
            // wgpu 30 rejects mixing ordinary commands and raw HAL commands on
            // one encoder. Finish the tracked prefix, insert a raw-only segment,
            // and continue the caller in a fresh tracked encoder. All segments
            // are later submitted once and in order, without a CPU/GPU wait.
            let prefix = std::mem::replace(encoder, gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {label:Some("after DLSS")}));
            segments.push(prefix.finish());
            let mut raw_encoder = gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {label:Some("NGX DLSS native commands")});
            if view.feature.is_none() {
                let mut error = [0;ERROR_CAP];
                let pointer = unsafe { raw_encoder.as_hal_mut::<wgpu::hal::api::Vulkan,_,_>(|command| {
                    command.map(|command| kern_dlss_create(self.native.pointer.as_ptr(), command.raw_handle().as_raw(), quality,
                        input.0,input.1,output.0,output.1,error.as_mut_ptr(),ERROR_CAP as u32)).unwrap_or(std::ptr::null_mut())
                }) };
                let feature = Arc::new(Feature {pointer:NonNull::new(pointer).ok_or_else(||error_text(&error))?,_context:self.native.clone()});
                let keep_created_feature = feature.clone();
                raw_encoder.on_submitted_work_done(move || drop(keep_created_feature));
                view.feature=Some(feature);
            }
            let eval = Eval {color:view.inputs[0].raw,depth:view.inputs[1].raw,motion:view.inputs[2].raw,output:view.output.raw,
                bias:view.inputs.get(3).map(|i|i.raw).unwrap_or_default(),exposure:view.exposure.raw,jitter_x:inputs.jitter[0],jitter_y:inputs.jitter[1],
                frame_ms:inputs.frame_time_ms,reset:u32::from(inputs.reset||view.first)};
            // Native handles are outside wgpu's resource lifetime graph. Keep all
            // of them through submission completion, even when resizing next frame.
            let feature = view.feature.as_ref().unwrap().clone();
            let keep_feature = feature.clone();
            let keep_images: Vec<_> = view.inputs.iter().cloned().chain([view.output.clone(),view.exposure.clone()]).collect();
            raw_encoder.on_submitted_work_done(move || { drop(keep_images); drop(keep_feature); });
            let mut error = [0;ERROR_CAP];
            let ok = unsafe { raw_encoder.as_hal_mut::<wgpu::hal::api::Vulkan,_,_>(|command| command.map(|command|
                kern_dlss_evaluate(feature.pointer.as_ptr(),command.raw_handle().as_raw(),&eval,error.as_mut_ptr(),ERROR_CAP as u32)).unwrap_or(0)) };
            if ok == 0 { view.first=true; view.feature=None; return Err(error_text(&error)); }
            segments.push(raw_encoder.finish());
            encoder.transition_resources(std::iter::empty(), std::iter::once(wgpu::TextureTransition {
                texture:&view.output.texture,selector:None,state:wgpu::TextureUses::RESOURCE }));
            view.first=false;
            Ok(view.output.texture.clone())
        }
    }
    impl Drop for DlssContext {
        fn drop(&mut self) {
            // Submitted work must complete before freeing feature handles. An
            // unsubmitted encoder still owns its retained feature Arc and device.
            let _ = self.native.device.poll(wgpu::PollType::wait_indefinitely());
            self.views.clear();
        }
    }
    #[cfg(test)] mod tests {
        use super::*;
        #[test] fn native_abi_matches_fixed_cpp_records() {
            assert_eq!(std::mem::size_of::<Image>(),32);
            assert_eq!(std::mem::size_of::<Eval>(),208);
            assert_eq!(std::mem::offset_of!(Eval,jitter_x),192);
        }
    }
}

#[cfg(all(feature = "dlss", target_os = "windows"))]
pub use native::DlssContext;
#[cfg(all(feature = "dlss", target_os = "windows"))]
pub(crate) use native::request_device;

#[cfg(not(all(feature = "dlss", target_os = "windows")))]
mod unavailable {
    use super::*;
    pub struct DlssContext;
    impl DlssContext {
        pub fn new(_: &Gpu) -> Result<Self,String> { Err("DLSS is not compiled in; use a Windows build with render/dlss and the official NVIDIA SDK (see render/native/DLSS.md)".into()) }
        pub fn optimal_size(&self,_:UpscaleMode,_:(u32,u32))->Result<(u32,u32),String> { Err("DLSS unavailable".into()) }
        pub fn evaluate(&mut self,_:&Gpu,_:&mut wgpu::CommandEncoder,_:&mut Vec<wgpu::CommandBuffer>,_:usize,_:UpscaleMode,_:(u32,u32),_:(u32,u32),_:Inputs<'_>)->Result<wgpu::Texture,String> { Err("DLSS unavailable".into()) }
    }
}
#[cfg(not(all(feature = "dlss", target_os = "windows")))]
pub use unavailable::DlssContext;

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn parameter_errors_are_explicit_before_native_commands() {
        assert!(quality(UpscaleMode::Native).is_err());
        assert!(validate_parameters((0,720),(1920,1080),[0.;2],1.,16.).is_err());
        assert!(validate_parameters((1921,720),(1920,1080),[0.;2],1.,16.).is_err());
        assert!(validate_parameters((1280,720),(1920,1080),[f32::NAN,0.],1.,16.).is_err());
        assert!(validate_parameters((1280,720),(1920,1080),[0.;2],0.,16.).is_err());
        assert!(validate_parameters((1280,720),(1920,1080),[0.;2],1.,f32::INFINITY).is_err());
        assert!(validate_parameters((1276,716),(1916,1076),[-0.25,0.25],1.,8.333).is_ok());
    }
}
