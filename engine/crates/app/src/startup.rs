//! Desktop launch policy shared by native games and the Luau runner.
//! Call `init` before loading assets; use the Windows GUI subsystem on binaries.
use std::sync::{OnceLock, atomic::{AtomicBool, Ordering}};
use winit::window::Icon;
static ICON: OnceLock<Icon> = OnceLock::new();
static READY: AtomicBool = AtomicBool::new(false);
static TITLE: OnceLock<String> = OnceLock::new();

/// Why the process is ending, on stderr and appended to
/// `<temp>/warp-exit.log` (a GUI-subsystem binary has no console, so a
/// window close, an Esc quit, a frame error or a panic would otherwise
/// leave no trace).
pub fn note_exit(why: &str) {
    note(&format!("exit: {why}"));
}

/// One line on stderr and in `<temp>/warp-exit.log`, for events around a
/// game's end (a lost host connection) worth finding after the fact.
pub fn note(why: &str) {
    let title = TITLE.get().map(String::as_str).unwrap_or("warp");
    eprintln!("{title}: {why}");
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let line = format!("{secs:.3} {title} pid {}: {}\n", std::process::id(), why.replace('\n', " | "));
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("warp-exit.log")) {
        use std::io::Write;
        let _ = f.write_all(line.as_bytes());
    }
}

pub fn interactive(args: &[String]) -> bool {
    if args.iter().any(|a| matches!(a.as_str(), "--shot"|"--bench"|"--profile"|"--profile-frames"|"--headless"|"--help"|"--version"|"--script"|"--dump"|"--smoke-frames")) {return false;}
    args.first().is_none_or(|a| a.starts_with('-') || matches!(a.as_str(),"play"|"run") || std::path::Path::new(a).exists())
}
pub fn fullscreen(args: &[String]) -> bool { !args.iter().any(|a|a=="--windowed") }

pub fn init(title: &str, png: Option<&[u8]>) {
    let _ = TITLE.set(title.to_owned());
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        note_exit(&format!("panic: {info}"));
        prev(info);
    }));
    #[cfg(windows)] windows::console(title);
    let rgba=png.and_then(|p|image::load_from_memory(p).ok()).map(|p|p.thumbnail(64,64).to_rgba8())
        .unwrap_or_else(||image::RgbaImage::from_fn(32,32,|x,y| {
            let mark=y>7 && y<25 && ((x>5 && x<10)||(x>22 && x<27)||(y>17 && x>12 && x<20));
            image::Rgba(if mark {[121,230,212,255]}else{[19,25,42,255]})
        }));
    let _=ICON.set(Icon::from_rgba(rgba.clone().into_raw(),rgba.width(),rgba.height()).expect("RGBA icon"));
    #[cfg(windows)] {
        let args:Vec<_>=std::env::args().skip(1).collect();
        // Warm-host launches must remain invisible and must never take focus.
        let hosted=std::env::var_os("GAMENIGHT").is_some() || std::env::var_os("GAMENIGHT_ADDR").is_some();
        if interactive(&args) && !hosted {
            let title=title.to_owned();
            std::thread::spawn(move||windows::loading(&title,rgba));
        }
    }
    #[cfg(not(windows))] let _=title;
}
pub fn icon() -> Option<Icon> { ICON.get().cloned() }
/// Called only after a correctly sized game frame is actually visible.
pub fn ready() { READY.store(true,Ordering::Release); }

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{ffi::c_void, os::windows::io::AsRawHandle, ptr::null_mut};
    type Handle=*mut c_void;
    #[repr(C)] struct Class {style:u32,proc:unsafe extern "system" fn(Handle,u32,usize,isize)->isize,extra:i32,window_extra:i32,instance:Handle,icon:Handle,cursor:Handle,background:Handle,menu:*const u16,name:*const u16}
    #[repr(C)] struct Message {window:Handle,message:u32,wparam:usize,lparam:isize,time:u32,point:[i32;2],private:u32}
    #[link(name="user32")] unsafe extern "system" {
        fn RegisterClassW(class:*const Class)->u16;
        fn CreateWindowExW(ex:u32,class:*const u16,title:*const u16,style:u32,x:i32,y:i32,w:i32,h:i32,parent:Handle,menu:Handle,instance:Handle,param:Handle)->Handle;
        fn DefWindowProcW(w:Handle,m:u32,p:usize,l:isize)->isize;
        fn ShowWindow(w:Handle,show:i32)->i32;
        fn DestroyWindow(w:Handle)->i32;
        fn CreateIcon(i:Handle,w:i32,h:i32,planes:u8,bits:u8,and:*const u8,xor:*const u8)->Handle;
        fn DestroyIcon(i:Handle)->i32;
        fn SendMessageW(window:Handle,msg:u32,w:usize,l:isize)->isize;
        fn PeekMessageW(m:*mut Message,w:Handle,min:u32,max:u32,remove:u32)->i32;
        fn TranslateMessage(m:*const Message)->i32;
        fn DispatchMessageW(m:*const Message)->isize;
    }
    #[link(name="kernel32")] unsafe extern "system" {
        fn GetModuleHandleW(name:*const u16)->Handle;
        fn AttachConsole(pid:u32)->i32;
        fn SetStdHandle(kind:u32,handle:Handle)->i32;
    }
    fn wide(s:&str)->Vec<u16> {s.encode_utf16().chain([0]).collect()}
    pub fn console(title:&str) {
        // Keep CLI tools/pipes usable without creating a console on double click.
        // Existing redirected handles must not be replaced by parent attachment.
        #[link(name="kernel32")] unsafe extern "system" {fn GetStdHandle(kind:u32)->Handle;}
        unsafe {
            let out=GetStdHandle(-11i32 as u32);
            if !out.is_null() && out as isize != -1 {return;}
            if AttachConsole(u32::MAX)!=0 {return;}
        }
        let dir=std::env::temp_dir().join("warp-logs");let _=std::fs::create_dir_all(&dir);
        let name:String=title.chars().map(|c|if c.is_ascii_alphanumeric(){c}else{'-'}).collect();
        if let Ok(file)=std::fs::File::create(dir.join(format!("{name}-{}.log",std::process::id()))) {
            unsafe {SetStdHandle(-11i32 as u32,file.as_raw_handle());SetStdHandle(-12i32 as u32,file.as_raw_handle());}
            // Handles must live for the process, including panic output.
            std::mem::forget(file);
        }
    }
    unsafe extern "system" fn procedure(w:Handle,m:u32,p:usize,l:isize)->isize {
        // This is a taskbar loading marker, not an empty restorable game window.
        if m==0x112 && p&0xfff0==0xf120 {return 0;}
        unsafe {DefWindowProcW(w,m,p,l)}
    }
    pub fn loading(title:&str,rgba:image::RgbaImage) {unsafe {
        let instance=GetModuleHandleW(std::ptr::null());
        let name=wide("WarpLoadingTaskbar");let title=wide(title);
        let w=rgba.width();let h=rgba.height();let mut pixels=rgba.into_raw();
        for p in pixels.chunks_exact_mut(4) {p.swap(0,2);}
        let mask=vec![0u8;(w.div_ceil(32)*4*h) as usize];
        let icon=CreateIcon(instance,w as i32,h as i32,1,32,mask.as_ptr(),pixels.as_ptr());
        let class=Class{style:0,proc:procedure,extra:0,window_extra:0,instance,icon,cursor:null_mut(),background:null_mut(),menu:std::ptr::null(),name:name.as_ptr()};
        if RegisterClassW(&class)==0 {DestroyIcon(icon);return;}
        let window=CreateWindowExW(0x08040000,name.as_ptr(),title.as_ptr(),0x00cf0000,0,0,160,90,null_mut(),null_mut(),instance,null_mut());
        if window.is_null() {DestroyIcon(icon);return;}
        SendMessageW(window,0x80,1,icon as isize);
        SendMessageW(window,0x80,0,icon as isize);
        if !READY.load(Ordering::Acquire) {ShowWindow(window,7);}
        while !READY.load(Ordering::Acquire) {
            let mut message:Message=std::mem::zeroed();
            while PeekMessageW(&mut message,null_mut(),0,0,1)!=0 {TranslateMessage(&message);DispatchMessageW(&message);}
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        DestroyWindow(window);DestroyIcon(icon);
    }}

    #[cfg(test)] mod tests {
        use super::*;
        #[link(name="user32")] unsafe extern "system" {
            fn FindWindowW(class:*const u16,title:*const u16)->Handle;
            fn IsIconic(window:Handle)->i32;
            fn GetForegroundWindow()->Handle;
            fn SendMessageW(window:Handle,msg:u32,w:usize,l:isize)->isize;
        }
        #[test]
        fn loading_marker_is_minimized_has_icon_and_does_not_take_focus() {
            READY.store(false,Ordering::Release);
            let name=wide("Warp startup test");
            let foreground=unsafe {GetForegroundWindow()};
            let worker=std::thread::spawn(||loading("Warp startup test",image::RgbaImage::from_pixel(32,32,image::Rgba([90,230,190,255]))));
            let mut found=null_mut();
            for _ in 0..100 {
                found=unsafe {FindWindowW(std::ptr::null(),name.as_ptr())};
                if !found.is_null() && unsafe {IsIconic(found)}!=0 {break;}
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let minimized=!found.is_null() && unsafe {IsIconic(found)}!=0;
            // WM_GETICON: the taskbar/Alt-Tab large icon is explicitly installed.
            let has_icon=!found.is_null() && unsafe {SendMessageW(found,0x7f,1,0)}!=0;
            let unchanged=unsafe {GetForegroundWindow()}==foreground;
            ready();worker.join().unwrap();
            assert!(minimized);assert!(has_icon);assert!(unchanged);
            assert!(unsafe {FindWindowW(std::ptr::null(),name.as_ptr())}.is_null());
        }
    }
}

#[cfg(test)] mod tests {
    use super::*;
    fn args(a:&[&str])->Vec<String>{a.iter().map(|s|s.to_string()).collect()}
    #[test] fn launch_modes_preserve_diagnostics_and_explicit_windowed() {
        for a in [&[][..], &["play"][..], &["--players","2"][..]] {assert!(interactive(&args(a)));assert!(fullscreen(&args(a)));}
        for a in [&["--shot","out.png"][..], &["play","--profile-frames","10"][..], &["check","game"][..], &["serve","game"][..], &["script","game"][..]] {assert!(!interactive(&args(a)));}
        assert!(!fullscreen(&args(&["play","--windowed"])));
    }
}
