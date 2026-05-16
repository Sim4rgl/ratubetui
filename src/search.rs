use crate::types::Track;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

#[derive(Debug)]
pub enum SearchEvent {
    Results(u64, Vec<Track>),
    Error(u64, String),
}

pub fn spawn_search(query: String, limit: usize, req_id: u64, tx: Sender<SearchEvent>) {
    thread::spawn(move || {
        let target = format!("ytsearch{}:{}", limit, query);
        let mut cmd = Command::new("yt-dlp");
        cmd.args([
            "--flat-playlist",
            "--no-warnings",
            "--ignore-errors",
            "-j",
            &target,
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
                let _ = tx.send(SearchEvent::Error(
                    req_id,
                    format!("failed to invoke yt-dlp: {e}"),
                ));
                return;
            }
        };

        if !output.status.success() && output.stdout.is_empty() {
            let err = String::from_utf8_lossy(&output.stderr).to_string();
            let _ = tx.send(SearchEvent::Error(req_id, err));
            return;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut tracks = Vec::new();
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            // Skip non-video entries (channels, playlists). yt-dlp tags
            // videos with ie_key="Youtube" and channels with "YoutubeTab".
            let ie_key = v.get("ie_key").and_then(|x| x.as_str()).unwrap_or("");
            if !ie_key.is_empty() && ie_key != "Youtube" {
                continue;
            }
            let id = v
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            // YouTube video IDs are exactly 11 chars; channel IDs (UC…) are
            // 24. Anything else here isn't a playable video.
            if id.len() != 11 {
                continue;
            }
            let title = v
                .get("title")
                .and_then(|x| x.as_str())
                .unwrap_or("(no title)")
                .to_string();
            let channel = v
                .get("channel")
                .or_else(|| v.get("uploader"))
                .or_else(|| v.get("playlist_uploader"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let duration = v.get("duration").and_then(|x| x.as_f64());
            tracks.push(Track {
                id,
                title,
                channel,
                duration,
            });
        }

        let _ = tx.send(SearchEvent::Results(req_id, tracks));
    });
}
