use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub enum PlayerEvent {
    TimePos(f64),
    Duration(f64),
    Paused(bool),
    Volume(f64),
    StartFile,
    EndFile { reason: String },
}

pub struct Player {
    child: Child,
    cmd_tx: Sender<Value>,
}

impl Player {
    pub fn spawn(events_tx: Sender<PlayerEvent>) -> Result<Self> {
        let pipe_path = mk_pipe_path();
        let mut cmd = mpv_command(&pipe_path);

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .context("failed to spawn mpv (is it on PATH?)")?;

        // Publish mpv's PID so the console-control handler installed in
        // main can kill it directly if the terminal closes.
        #[cfg(windows)]
        crate::winjob::set_mpv_pid(child.id());

        // If anything in the setup below fails, we still need to take mpv
        // down with us — otherwise the process leaks. Wrap setup so we can
        // catch and clean up. (On Windows the process-wide kill-on-exit job
        // installed in main also covers this, but we still want a clean
        // error path on graceful failure.)
        match setup_pipes(&pipe_path) {
            Ok((reader_pipe, writer_pipe)) => {
                thread::spawn(move || reader_loop(reader_pipe, events_tx));

                let (cmd_tx, cmd_rx) = channel::<Value>();
                thread::spawn(move || writer_loop(writer_pipe, cmd_rx));

                Ok(Self { child, cmd_tx })
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(e)
            }
        }
    }

    fn send(&self, cmd: Value) -> Result<()> {
        self.cmd_tx
            .send(cmd)
            .map_err(|_| anyhow!("mpv writer thread is gone"))
    }

    pub fn loadfile(&self, url: &str) -> Result<()> {
        self.send(json!({"command": ["loadfile", url, "replace"]}))
    }

    pub fn stop(&self) -> Result<()> {
        self.send(json!({"command": ["stop"]}))
    }

    pub fn toggle_pause(&self) -> Result<()> {
        self.send(json!({"command": ["cycle", "pause"]}))
    }

    pub fn seek(&self, secs: f64) -> Result<()> {
        self.send(json!({"command": ["seek", secs, "relative"]}))
    }

    pub fn add_volume(&self, delta: f64) -> Result<()> {
        self.send(json!({"command": ["add", "volume", delta]}))
    }

    pub fn quit(&self) -> Result<()> {
        self.send(json!({"command": ["quit"]}))
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.quit();
        // Give mpv a brief moment to shut down gracefully, then force-kill
        // so we never leak the process on panic / abnormal exit. Force-killing
        // also unblocks the writer thread if it was stuck on a pipe write.
        for _ in 0..20 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn mpv_command(pipe_path: &str) -> Command {
    let mut cmd = Command::new("mpv");
    cmd.args([
        "--idle=yes",
        "--no-video",
        "--no-terminal",
        "--really-quiet",
        "--audio-display=no",
        "--volume=75",
        "--ytdl-format=bestaudio[ext=m4a]/bestaudio/best",
        "--cache=yes",
        "--demuxer-max-bytes=64MiB",
        "--demuxer-max-back-bytes=32MiB",
        &format!("--input-ipc-server={}", pipe_path),
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    cmd
}

fn mk_pipe_path() -> String {
    let name = format!("{}-{}-{}", env!("CARGO_PKG_NAME"), std::process::id(), now_nanos());
    if cfg!(windows) {
        format!(r"\\.\pipe\{}", name)
    } else {
        let mut p = std::env::temp_dir();
        p.push(&name);
        p.to_string_lossy().into_owned()
    }
}

fn reader_loop(pipe: File, tx: Sender<PlayerEvent>) {
    let mut br = BufReader::new(pipe);
    let mut buf = String::new();
    loop {
        buf.clear();
        match br.read_line(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = buf.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(evt) = parse_event(&v) {
            if tx.send(evt).is_err() {
                break;
            }
        }
    }
}

fn writer_loop(mut pipe: File, rx: Receiver<Value>) {
    while let Ok(cmd) = rx.recv() {
        // mpv echoes a small reply for every command. We don't read them on
        // this connection, so they accumulate in the kernel pipe buffer
        // (~12 KB on Windows). After a few hundred commands the buffer fills
        // and the next write_all blocks forever. Drain any pending replies
        // first to keep that buffer empty.
        drain_pipe(&pipe);

        let mut s = cmd.to_string();
        s.push('\n');
        if pipe.write_all(s.as_bytes()).is_err() {
            break;
        }
        let _ = pipe.flush();
    }
}

/// Non-blocking drain of all pending bytes on a Windows named pipe.
/// Uses PeekNamedPipe to check availability, ReadFile to consume.
/// On non-Windows this is a no-op (mpv on Linux/macOS uses a unix socket
/// which would need a different path; ytui currently targets Windows).
#[cfg(windows)]
fn drain_pipe(pipe: &File) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::ReadFile;
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;

    let handle: HANDLE = pipe.as_raw_handle() as HANDLE;
    let mut buf = [0u8; 4096];
    loop {
        let mut available: u32 = 0;
        let peek_ok = unsafe {
            PeekNamedPipe(
                handle,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if peek_ok == 0 || available == 0 {
            return;
        }
        let to_read = available.min(buf.len() as u32);
        let mut read: u32 = 0;
        let ok = unsafe {
            ReadFile(
                handle,
                buf.as_mut_ptr(),
                to_read,
                &mut read,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || read == 0 {
            return;
        }
    }
}

#[cfg(not(windows))]
fn drain_pipe(_pipe: &File) {}

fn setup_pipes(pipe_path: &str) -> Result<(File, File)> {
    // Open TWO independent IPC connections to mpv. One is purely for
    // reading events, the other purely for writing commands.
    //
    // Why two: on Windows, std::fs::File on a named pipe is synchronous.
    // A `ReadFile` blocked in one thread will block a concurrent
    // `WriteFile` on a *cloned* handle in another thread (same kernel pipe
    // instance). Using two separate client connections gives each thread
    // its own pipe instance with independent I/O state. mpv's IPC server
    // accepts unlimited concurrent clients.
    let mut reader_pipe = open_pipe_with_retry(pipe_path, Duration::from_secs(8))
        .context("failed to open mpv reader pipe")?;

    // Subscribe to property changes ON the reader connection. mpv routes
    // `property-change` events back only over the connection that issued the
    // subscription, so observe_property has to be sent here.
    for (id, prop) in [
        (1u64, "time-pos"),
        (2, "duration"),
        (3, "pause"),
        (4, "volume"),
    ] {
        let mut line = json!({"command": ["observe_property", id, prop]}).to_string();
        line.push('\n');
        reader_pipe
            .write_all(line.as_bytes())
            .context("subscribe to mpv property")?;
    }
    let _ = reader_pipe.flush();

    let writer_pipe = open_pipe_with_retry(pipe_path, Duration::from_secs(8))
        .context("failed to open mpv writer pipe")?;
    Ok((reader_pipe, writer_pipe))
}

fn parse_event(v: &Value) -> Option<PlayerEvent> {
    match v.get("event")?.as_str()? {
        "property-change" => {
            let name = v.get("name")?.as_str()?;
            let data = v.get("data");
            match name {
                "time-pos" => Some(PlayerEvent::TimePos(
                    data.and_then(|x| x.as_f64()).unwrap_or(f64::NAN),
                )),
                "duration" => Some(PlayerEvent::Duration(
                    data.and_then(|x| x.as_f64()).unwrap_or(f64::NAN),
                )),
                "pause" => Some(PlayerEvent::Paused(
                    data.and_then(|x| x.as_bool()).unwrap_or(false),
                )),
                "volume" => Some(PlayerEvent::Volume(
                    data.and_then(|x| x.as_f64()).unwrap_or(0.0),
                )),
                _ => None,
            }
        }
        "start-file" => Some(PlayerEvent::StartFile),
        "end-file" => Some(PlayerEvent::EndFile {
            reason: v
                .get("reason")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string(),
        }),
        _ => None,
    }
}

fn open_pipe_with_retry(path: &str, timeout: Duration) -> Result<File> {
    let start = Instant::now();
    let mut last_err: Option<std::io::Error> = None;
    while start.elapsed() < timeout {
        match OpenOptions::new().read(true).write(true).open(path) {
            Ok(f) => return Ok(f),
            Err(e) => {
                last_err = Some(e);
                thread::sleep(Duration::from_millis(80));
            }
        }
    }
    Err(anyhow!(
        "mpv IPC pipe never became available: {}",
        last_err
            .map(|e| e.to_string())
            .unwrap_or_else(|| "timeout".into())
    ))
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}
