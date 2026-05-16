use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

#[derive(Debug)]
pub enum ResolveEvent {
    Resolved { req: u64, stream_url: String },
    Failed { req: u64, err: String },
}

pub fn spawn_resolve(req: u64, video_url: String, tx: Sender<ResolveEvent>) {
    thread::spawn(move || {
        let mut cmd = Command::new("yt-dlp");
        cmd.args([
            "--no-warnings",
            "--no-playlist",
            "-f",
            "bestaudio/best",
            "-g",
            &video_url,
        ]);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let output = match cmd.output() {
            Ok(o) => o,
            Err(e) => {
                let _ = tx.send(ResolveEvent::Failed {
                    req,
                    err: format!("failed to invoke yt-dlp: {e}"),
                });
                return;
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Take the last non-empty stderr line — yt-dlp's error is usually there.
            let msg = stderr
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("yt-dlp failed")
                .trim()
                .to_string();
            let _ = tx.send(ResolveEvent::Failed { req, err: msg });
            return;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let url = stdout
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_string();
        let _ = if url.is_empty() {
            tx.send(ResolveEvent::Failed {
                req,
                err: "yt-dlp returned no URL".into(),
            })
        } else {
            tx.send(ResolveEvent::Resolved {
                req,
                stream_url: url,
            })
        };
    });
}
