// Kern's C ABI around the unmodified official NVIDIA NGX Vulkan SDK.
// SDK inline helpers are covered by NVIDIA's license, not the engine's MIT license.
// This software contains source code provided by NVIDIA Corporation.
#include <windows.h>
#include <vulkan/vulkan.h>
#include <nvsdk_ngx_helpers.h>
#include <nvsdk_ngx_helpers_vk.h>
#include <cstdint>
#include <cstddef>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <new>
#include <string>

namespace {
std::recursive_mutex ngx_mutex;
constexpr const char* project_id = "7d8e9c51-2461-4a6b-bc30-cdba9d2e91fc";
constexpr const char* engine_version = "Kern-0.1";
struct Context { VkDevice device{}; NVSDK_NGX_Parameter* caps{}; };
struct Feature { NVSDK_NGX_Handle* handle{}; NVSDK_NGX_Parameter* params{}; };
void error(char* out, uint32_t cap, const char* op, NVSDK_NGX_Result result) {
    if (out && cap) std::snprintf(out, cap, "%s: NGX result 0x%08x", op, static_cast<unsigned>(result));
}
bool success(NVSDK_NGX_Result result, char* out, uint32_t cap, const char* op) {
    if (NVSDK_NGX_SUCCEED(result)) return true;
    error(out, cap, op, result); return false;
}
NVSDK_NGX_FeatureCommonInfo common(const wchar_t* runtime, const wchar_t** paths) {
    NVSDK_NGX_FeatureCommonInfo info{};
    paths[0] = runtime;
    if (runtime && *runtime) { info.PathListInfo.Path = paths; info.PathListInfo.Length = 1; }
    return info;
}
NVSDK_NGX_FeatureDiscoveryInfo discovery(const wchar_t* logs, NVSDK_NGX_FeatureCommonInfo* info) {
    NVSDK_NGX_FeatureDiscoveryInfo d{};
    d.SDKVersion = NVSDK_NGX_Version_API;
    d.FeatureID = NVSDK_NGX_Feature_SuperSampling;
    d.Identifier.IdentifierType = NVSDK_NGX_Application_Identifier_Type_Project_Id;
    d.Identifier.v.ProjectDesc = {project_id, NVSDK_NGX_ENGINE_TYPE_CUSTOM, engine_version};
    d.ApplicationDataPath = logs;
    d.FeatureInfo = info;
    return d;
}
}

extern "C" {
// Plain fixed-size C records: no C++ class layout crosses the Rust boundary.
struct KernDlssImage {
    uint64_t image, view;
    uint32_t format, width, height;
};
struct KernDlssEval {
    KernDlssImage color, depth, motion, output, bias, exposure;
    float jitter_x, jitter_y, frame_ms;
    uint32_t reset;
};
static_assert(sizeof(KernDlssImage)==32, "Rust/C++ image ABI mismatch");
static_assert(sizeof(KernDlssEval)==208 && offsetof(KernDlssEval,jitter_x)==192, "Rust/C++ eval ABI mismatch");

int kern_dlss_extensions(uint64_t instance, uint64_t physical, const wchar_t* runtime,
    const wchar_t* logs, char* names, uint32_t slots, uint32_t* count, char* out, uint32_t cap) {
    std::lock_guard<std::recursive_mutex> guard(ngx_mutex);
    const wchar_t* paths[1]{}; auto info = common(runtime, paths); auto d = discovery(logs, &info);
    VkExtensionProperties* properties = nullptr;
    auto result = physical ? NVSDK_NGX_VULKAN_GetFeatureDeviceExtensionRequirements(
        reinterpret_cast<VkInstance>(instance), reinterpret_cast<VkPhysicalDevice>(physical), &d, count, &properties)
        : NVSDK_NGX_VULKAN_GetFeatureInstanceExtensionRequirements(&d, count, &properties);
    if (!success(result, out, cap, "NGX extension query")) return 0;
    if (*count > slots) { if(out && cap) std::snprintf(out,cap,"NGX requires %u extensions; bounded capacity is %u",*count,slots); return 0; }
    for(uint32_t i=0;i<*count;++i) { std::memcpy(names+i*VK_MAX_EXTENSION_NAME_SIZE,properties[i].extensionName,VK_MAX_EXTENSION_NAME_SIZE); }
    return 1;
}

void* kern_dlss_init(uint64_t instance, uint64_t physical, uint64_t device,
    void* gipa, void* gdpa, const wchar_t* runtime, const wchar_t* logs, char* out, uint32_t cap) {
    std::lock_guard<std::recursive_mutex> guard(ngx_mutex);
    const wchar_t* paths[1]{}; auto info = common(runtime, paths);
    auto vk_device = reinterpret_cast<VkDevice>(device);
    auto result = NVSDK_NGX_VULKAN_Init_with_ProjectID(project_id, NVSDK_NGX_ENGINE_TYPE_CUSTOM,
        engine_version, logs, reinterpret_cast<VkInstance>(instance), reinterpret_cast<VkPhysicalDevice>(physical),
        vk_device, reinterpret_cast<PFN_vkGetInstanceProcAddr>(gipa), reinterpret_cast<PFN_vkGetDeviceProcAddr>(gdpa), &info);
    if (!success(result,out,cap,"NVSDK_NGX_VULKAN_Init_with_ProjectID")) return nullptr;
    auto context = new(std::nothrow) Context{vk_device,nullptr};
    if (!context) { if(out && cap) std::snprintf(out,cap,"NGX context allocation failed"); NVSDK_NGX_VULKAN_Shutdown1(vk_device); return nullptr; }
    result = NVSDK_NGX_VULKAN_GetCapabilityParameters(&context->caps);
    if (!success(result,out,cap,"NGX capability parameters")) { NVSDK_NGX_VULKAN_Shutdown1(vk_device); delete context; return nullptr; }
    int available = 0; NVSDK_NGX_Parameter_GetI(context->caps,NVSDK_NGX_Parameter_SuperSampling_Available,&available);
    if (!available) {
        int outdated = 0; NVSDK_NGX_Parameter_GetI(context->caps,NVSDK_NGX_Parameter_SuperSampling_NeedsUpdatedDriver,&outdated);
        if(out && cap) std::snprintf(out,cap,"DLSS Super Resolution is unavailable (driver update required=%d)",outdated);
        NVSDK_NGX_VULKAN_DestroyParameters(context->caps); NVSDK_NGX_VULKAN_Shutdown1(vk_device); delete context; return nullptr;
    }
    return context;
}
void kern_dlss_shutdown(void* pointer) {
    std::lock_guard<std::recursive_mutex> guard(ngx_mutex);
    auto c = static_cast<Context*>(pointer); if(!c) return;
    NVSDK_NGX_VULKAN_DestroyParameters(c->caps); NVSDK_NGX_VULKAN_Shutdown1(c->device); delete c;
}
int kern_dlss_optimal(void* pointer,uint32_t quality,uint32_t width,uint32_t height,
    uint32_t* sizes,char* out,uint32_t cap) {
    std::lock_guard<std::recursive_mutex> guard(ngx_mutex);
    auto c=static_cast<Context*>(pointer); float sharpness=0;
    return success(NGX_DLSS_GET_OPTIMAL_SETTINGS(c->caps,width,height,static_cast<NVSDK_NGX_PerfQuality_Value>(quality),
        &sizes[0],&sizes[1],&sizes[2],&sizes[3],&sizes[4],&sizes[5],&sharpness),out,cap,"NGX optimal render size") ? 1 : 0;
}
void* kern_dlss_create(void* pointer,uint64_t command,uint32_t quality,
    uint32_t width,uint32_t height,uint32_t out_width,uint32_t out_height,char* out,uint32_t cap) {
    std::lock_guard<std::recursive_mutex> guard(ngx_mutex);
    auto c=static_cast<Context*>(pointer); auto f=new(std::nothrow) Feature{};
    if(!f) { if(out && cap) std::snprintf(out,cap,"NGX feature allocation failed"); return nullptr; }
    if(!success(NVSDK_NGX_VULKAN_AllocateParameters(&f->params),out,cap,"NGX allocate feature parameters")) {delete f;return nullptr;}
    NVSDK_NGX_DLSS_Create_Params p{};
    p.Feature.InWidth=width; p.Feature.InHeight=height;
    p.Feature.InTargetWidth=out_width; p.Feature.InTargetHeight=out_height;
    p.Feature.InPerfQualityValue=static_cast<NVSDK_NGX_PerfQuality_Value>(quality);
    p.InFeatureCreateFlags=NVSDK_NGX_DLSS_Feature_Flags_IsHDR|NVSDK_NGX_DLSS_Feature_Flags_MVLowRes|NVSDK_NGX_DLSS_Feature_Flags_DepthInverted;
    auto result=NGX_VULKAN_CREATE_DLSS_EXT1(c->device,reinterpret_cast<VkCommandBuffer>(command),1,1,&f->handle,f->params,&p);
    if(!success(result,out,cap,"NGX create DLSS Super Resolution")) {NVSDK_NGX_VULKAN_DestroyParameters(f->params);delete f;return nullptr;}
    return f;
}
void kern_dlss_release(void* pointer) {
    std::lock_guard<std::recursive_mutex> guard(ngx_mutex);
    auto f=static_cast<Feature*>(pointer); if(!f) return;
    NVSDK_NGX_VULKAN_ReleaseFeature(f->handle); NVSDK_NGX_VULKAN_DestroyParameters(f->params); delete f;
}
int kern_dlss_evaluate(void* pointer,uint64_t command,const KernDlssEval* inputs,char* out,uint32_t cap) {
    std::lock_guard<std::recursive_mutex> guard(ngx_mutex);
    auto f=static_cast<Feature*>(pointer);
    auto resource=[](const KernDlssImage& i,bool write) {
        VkImageSubresourceRange range{VK_IMAGE_ASPECT_COLOR_BIT,0,1,0,1};
        return NVSDK_NGX_Create_ImageView_Resource_VK(reinterpret_cast<VkImageView>(i.view),reinterpret_cast<VkImage>(i.image),
            range,static_cast<VkFormat>(i.format),i.width,i.height,write);
    };
    auto color=resource(inputs->color,false),depth=resource(inputs->depth,false),motion=resource(inputs->motion,false);
    auto output=resource(inputs->output,true),bias=resource(inputs->bias,false),exposure=resource(inputs->exposure,false);
    NVSDK_NGX_VK_DLSS_Eval_Params p{};
    p.Feature.pInColor=&color; p.Feature.pInOutput=&output;
    p.pInDepth=&depth; p.pInMotionVectors=&motion;
    p.pInBiasCurrentColorMask=inputs->bias.image ? &bias : nullptr;
    p.pInExposureTexture=&exposure;
    p.InJitterOffsetX=inputs->jitter_x; p.InJitterOffsetY=inputs->jitter_y;
    p.InRenderSubrectDimensions={inputs->color.width,inputs->color.height};
    p.InReset=inputs->reset; p.InMVScaleX=1.0f; p.InMVScaleY=1.0f;
    p.InPreExposure=1.0f; p.InExposureScale=1.0f; p.InFrameTimeDeltaInMsec=inputs->frame_ms;
    return success(NGX_VULKAN_EVALUATE_DLSS_EXT(reinterpret_cast<VkCommandBuffer>(command),f->handle,f->params,&p),
        out,cap,"NGX evaluate DLSS Super Resolution") ? 1 : 0;
}
}
