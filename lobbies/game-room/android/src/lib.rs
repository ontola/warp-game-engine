//! Splitscreen Club hosting natively on an Android phone: the Game Room
//! and its games in a NativeActivity with Vulkan, instead of the browser
//! host in a WebView or Chrome. It is the browser host itself
//! (examples/browser: the room, cloud room, seats, game switching),
//! compiled for Android, where app::web is the Android shell
//! (engine/crates/app/src/android.rs). Built and packed by
//! engine/tools/build-android-host.py; see engine/docs/android-host.md.

#[cfg(target_os = "android")]
#[path = "../../examples/browser/main.rs"]
mod browser;

/// The NativeActivity's entry (android-activity calls it on its own thread).
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(activity: android_activity::AndroidApp) {
    app::android::main(activity, browser::page::main);
}
