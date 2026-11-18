//! Storage bindings each GPU forest entry point uses, against the engine rule
//! of 8 storage buffers and 4 storage textures per stage (engine/docs/browser.md).
//! `cargo test --release -p render --lib forest_bindings -- --ignored --nocapture`
//! prints the inventory; the 8-buffer work turns it into an assertion.

fn sources()->Vec<(&'static str,String)> {
    vec![
        ("foliage_refine",crate::foliage_refine::refine_source()),
        ("foliage_select",include_str!("shaders/foliage_select.wgsl").to_owned()),
        ("foliage_residency",include_str!("shaders/foliage_residency.wgsl").to_owned()),
        ("foliage_materialize",include_str!("shaders/foliage_materialize.wgsl").to_owned()),
        ("foliage_pose_dispatch",include_str!("shaders/foliage_pose_dispatch.wgsl").to_owned()),
        ("foliage_publish",include_str!("shaders/foliage_publish.wgsl").to_owned()),
        ("foliage_compact",crate::foliage_compact::shader_source().to_owned()),
        ("foliage_seed_visibility",include_str!("shaders/foliage_seed_visibility.wgsl").to_owned()),
        ("voxel_forest",crate::voxel_forest::shader_source()),
        ("voxel_splat_raster",include_str!("shaders/voxel_splat_raster.wgsl").to_owned()),
        ("foliage_world",crate::foliage_world_render::shader_source().unwrap()),
        ("vegetation_bricks",crate::vegetation_brick_render::shader_source().unwrap()),
    ]
}

#[test]
#[ignore = "prints the forest's storage binding inventory"]
fn inventory() {
    for (file,source) in sources() {
        let module=match naga::front::wgsl::parse_str(&source) {Ok(m)=>m,Err(e)=>{println!("{file}: parse error {}",e.emit_to_string(&source));continue;}};
        let info=match naga::valid::Validator::new(naga::valid::ValidationFlags::all(),naga::valid::Capabilities::all()).validate(&module) {Ok(i)=>i,Err(e)=>{println!("{file}: invalid {}",e.emit_to_string(&source));continue;}};
        for (index,entry) in module.entry_points.iter().enumerate() {
            let ep=info.get_entry_point(index);
            let (mut buffers,mut textures,mut names)=(0,0,Vec::new());
            for (handle,global) in module.global_variables.iter() {
                if ep[handle].is_empty() {continue;}
                let storage_texture=matches!(module.types[global.ty].inner,naga::TypeInner::Image {class:naga::ImageClass::Storage {..},..});
                if matches!(global.space,naga::AddressSpace::Storage {..}) {buffers+=1;names.push(global.name.clone().unwrap_or_default());}
                if storage_texture {textures+=1;}
            }
            let flag=if buffers>8 || textures>4 {" OVER"} else {""};
            println!("{file}\t{}\t{:?}\tbuffers {buffers}\ttextures {textures}{flag}\t{}",entry.name,entry.stage,names.join(","));
        }
    }
}
