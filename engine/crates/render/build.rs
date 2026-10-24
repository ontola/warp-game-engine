use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=NVIDIA_DLSS_SDK");
    println!("cargo:rerun-if-env-changed=VULKAN_HEADERS");
    println!("cargo:rerun-if-changed=native/dlss_bridge.cpp");
    if env::var_os("CARGO_FEATURE_DLSS").is_none() || env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") { return; }
    assert_eq!(env::var("CARGO_CFG_TARGET_ENV").as_deref(), Ok("msvc"), "DLSS requires the Windows MSVC toolchain");
    let sdk = PathBuf::from(env::var_os("NVIDIA_DLSS_SDK").expect("dlss feature: set NVIDIA_DLSS_SDK to the pinned official NVIDIA/DLSS checkout; see native/DLSS.md"));
    let headers = PathBuf::from(env::var_os("VULKAN_HEADERS").expect("dlss feature: set VULKAN_HEADERS to the pinned KhronosGroup/Vulkan-Headers checkout"));
    assert!(sdk.join("include/nvsdk_ngx_vk.h").is_file(), "NVIDIA_DLSS_SDK does not contain the official headers");
    assert!(headers.join("include/vulkan/vulkan.h").is_file(), "VULKAN_HEADERS does not contain Vulkan headers");
    let static_crt = env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default().split(',').any(|s| s == "crt-static");
    let library = if static_crt { "nvsdk_ngx_s" } else { "nvsdk_ngx_d" };
    let libdir = sdk.join("lib/Windows_x86_64/x64");
    assert!(libdir.join(format!("{library}.lib")).is_file(), "NGX SDK import library is missing");
    cc::Build::new().cpp(true).file("native/dlss_bridge.cpp")
        .include(sdk.join("include")).include(headers.join("include"))
        .define("NOMINMAX", None).define("WIN32_LEAN_AND_MEAN", None)
        .flag_if_supported("/std:c++17").flag_if_supported("/EHsc")
        .static_crt(static_crt).compile("kern_dlss_bridge");
    println!("cargo:rustc-link-search=native={}", libdir.display());
    println!("cargo:rustc-link-lib=static={library}");
    for system in ["advapi32", "shlwapi", "version", "winmm"] { println!("cargo:rustc-link-lib={system}"); }
}
