//! Tee stdout and stderr into the `logs` tool (Unix): each stream is
//! redirected into a pipe whose reader thread writes everything on to the
//! original stream and keeps the lines. Dropping the guard restores the
//! streams and drains what is left. `WARP_LIVE_CONSOLE=0` turns it off.
//! Elsewhere only lines the game and server log themselves are kept.
use crate::Logs;

pub struct Capture {
    #[cfg(unix)]
    streams: Vec<(i32, i32, std::sync::mpsc::Receiver<()>)>,
}

#[cfg(unix)]
pub fn capture(logs: Logs) -> Option<Capture> {
    use std::io::{Read, Write};
    use std::os::fd::FromRawFd;
    if std::env::var("WARP_LIVE_CONSOLE").is_ok_and(|v| v == "0") { return None; }
    let mut streams = Vec::new();
    for fd in [1, 2] {
        // SAFETY: plain descriptor calls; every new descriptor is owned below.
        let (read, original) = unsafe {
            let mut p = [0i32; 2];
            if libc::pipe(p.as_mut_ptr()) != 0 { break; }
            let original = libc::dup(fd);
            if original < 0 || libc::dup2(p[1], fd) < 0 { libc::close(p[0]); libc::close(p[1]); if original >= 0 { libc::close(original); } break; }
            libc::close(p[1]);
            (p[0], original)
        };
        let logs = logs.clone();
        let (done_tx, done) = std::sync::mpsc::channel();
        let out_fd = unsafe { libc::dup(original) };
        let handle = std::thread::Builder::new().name("live-console".into()).spawn(move || {
            // SAFETY: both descriptors were created above for this thread alone.
            let mut pipe = unsafe { std::fs::File::from_raw_fd(read) };
            let mut out = unsafe { std::fs::File::from_raw_fd(out_fd) };
            let mut buf = [0u8; 4096];
            let mut line = Vec::new();
            loop {
                let n = match pipe.read(&mut buf) { Ok(0) | Err(_) => break, Ok(n) => n };
                let _ = out.write_all(&buf[..n]);
                for &b in &buf[..n] {
                    if b == b'\n' { logs.push(String::from_utf8_lossy(&line).to_string()); line.clear(); } else if line.len() < 4096 { line.push(b); }
                }
            }
            if !line.is_empty() { logs.push(String::from_utf8_lossy(&line).to_string()); }
            let _ = done_tx.send(());
        }).ok()?;
        drop(handle);
        streams.push((fd, original, done));
    }
    Some(Capture { streams })
}

#[cfg(not(unix))]
pub fn capture(_logs: Logs) -> Option<Capture> { None }

impl Drop for Capture {
    fn drop(&mut self) {
        #[cfg(unix)]
        for (fd, original, done) in self.streams.drain(..) {
            use std::io::Write;
            if fd == 1 { let _ = std::io::stdout().flush(); }
            // SAFETY: restores the stream; the pipe's last writer closes, so
            // the reader sees end of file after draining.
            unsafe { libc::dup2(original, fd); libc::close(original); }
            // A child process holding the pipe would keep it open: bounded wait.
            let _ = done.recv_timeout(std::time::Duration::from_millis(500));
        }
    }
}
