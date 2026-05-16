use crate::player::{Player, PlayerEvent};
use crate::resolve::{spawn_resolve, ResolveEvent};
use crate::search::{spawn_search, SearchEvent};
use crate::types::{Focus, LoopMode, Track};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

const RESULTS_PER_QUERY: usize = 30;

pub struct App {
    pub focus: Focus,
    pub query: String,
    pub results: Vec<Track>,
    pub playlist: Vec<Track>,
    pub results_sel: usize,
    pub playlist_sel: usize,
    pub current: Option<usize>,
    pub paused: bool,
    pub loading: bool,
    pub time_pos: f64,
    pub duration: f64,
    pub volume: f64,
    pub loop_mode: LoopMode,
    pub show_help: bool,
    pub status: Option<(String, Instant)>,
    pub searching: bool,
    pub search_req: u64,
    pub last_query: String,
    pub results_limit: usize,
    pub jump_to_on_results: Option<usize>,
    pub play_req: u64,
    pub pending_g: bool,
    pub pending_d: bool,
    pub error_retries: u32,
    pub search_tx: Sender<SearchEvent>,
    pub resolve_tx: Sender<ResolveEvent>,
    pub player: Player,
    pub should_quit: bool,
    pub pending_count: Option<u32>,
}

impl App {
    pub fn new(
        player: Player,
        search_tx: Sender<SearchEvent>,
        resolve_tx: Sender<ResolveEvent>,
    ) -> Self {
        let mut app = Self {
            focus: Focus::Playlist,
            query: String::new(),
            results: Vec::new(),
            playlist: Vec::new(),
            results_sel: 0,
            playlist_sel: 0,
            current: None,
            paused: false,
            loading: false,
            time_pos: f64::NAN,
            duration: f64::NAN,
            volume: 75.0,
            loop_mode: LoopMode::Off,
            show_help: false,
            status: None,
            searching: false,
            search_req: 0,
            last_query: String::new(),
            results_limit: RESULTS_PER_QUERY,
            jump_to_on_results: None,
            play_req: 0,
            pending_g: false,
            pending_d: false,
            error_retries: 0,
            search_tx,
            resolve_tx,
            player,
            should_quit: false,
            pending_count: None,
        };
        if let Some(saved) = load_playlist() {
            if !saved.is_empty() {
                app.set_status(format!("loaded {} saved tracks", saved.len()));
            }
            app.playlist = saved;
        }
        app
    }

    pub fn set_status(&mut self, s: impl Into<String>) {
        self.status = Some((s.into(), Instant::now()));
    }

    pub fn status_text(&self) -> Option<&str> {
        match &self.status {
            Some((s, t)) if t.elapsed() < Duration::from_secs(4) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        // Windows crossterm emits Press + Repeat + Release; we want held-key
        // repeat to work, but ignore Release.
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return;
        }

        // Esc always exits the app, regardless of focus / overlay.
        if key.code == KeyCode::Esc {
            self.should_quit = true;
            return;
        }

        if self.show_help {
            if matches!(key.code, KeyCode::Char('?') | KeyCode::Char('q')) {
                self.show_help = false;
            }
            return;
        }

        if self.focus == Focus::Search {
            self.on_key_search(key);
            return;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Pending multi-key sequences (gg, dd) — only valid for current focus.
        if self.pending_g {
            self.pending_g = false;
            if let KeyCode::Char('g') = key.code {
                self.go_top();
                return;
            }
        }
        if self.pending_d {
            self.pending_d = false;
            if let KeyCode::Char('d') = key.code {
                if self.focus == Focus::Playlist {
                    let n = self.take_pending_count(1);
                    for _ in 0..n {
                        self.delete_playlist_current();
                    }
                }
                return;
            }
        } else if let KeyCode::Char('d') = key.code {
            if self.focus == Focus::Playlist {
                self.pending_d = true;
            }
            return;
        }

        if let KeyCode::Char(c @ '0'..='9') = key.code {
            let d = c as u32 - '0' as u32;
            if d > 0 || self.pending_count.is_some() {
                self.pending_count = Some(
                    self.pending_count
                        .unwrap_or(0)
                        .saturating_mul(10)
                        .saturating_add(d),
                );
                return;
            }
        }

        // keys that use pending_count
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                let n = self.take_pending_count(1);
                self.move_sel(n)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if ctrl {
                    self.focus = Focus::Search
                } else {
                    let n = self.take_pending_count(1);
                    self.move_sel(-n)
                }
            }
            KeyCode::PageDown => {
                let n = self.take_pending_count(1);
                self.move_sel(10 * n)
            }
            KeyCode::PageUp => {
                let n = self.take_pending_count(1);
                self.move_sel(-10 * n)
            }
            KeyCode::Char('d') if ctrl => {
                let n = self.take_pending_count(1);
                self.move_sel(10 * n)
            }
            KeyCode::Char('u') if ctrl => {
                let n = self.take_pending_count(1);
                self.move_sel(-10 * n)
            }
            KeyCode::Char('n') => {
                let n = self.take_pending_count(1) as usize;
                self.next_track(n, false)
            }
            KeyCode::Char('N') => {
                let n = self.take_pending_count(1) as usize;
                self.prev_track(n);
            }
            KeyCode::Char('>') => {
                let n = self.take_pending_count(1) as f64;
                self.seek(if key.kind == KeyEventKind::Repeat {
                    60.0
                } else {
                    10.0 * n
                })
            }
            KeyCode::Char('<') => {
                let n = self.take_pending_count(1) as f64;
                self.seek(if key.kind == KeyEventKind::Repeat {
                    -60.0
                } else {
                    -10.0 * n
                })
            }
            _ => {}
        }

        self.pending_count = None;
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('/') => {
                self.focus = Focus::Search;
                self.query.clear();
            }
            KeyCode::Tab => {
                self.focus = match self.focus {
                    Focus::Results => Focus::Playlist,
                    Focus::Playlist => Focus::Results,
                    Focus::Search => Focus::Results,
                };
            }
            KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Results => Focus::Playlist,
                    Focus::Playlist => Focus::Results,
                    Focus::Search => Focus::Results,
                };
            }
            KeyCode::Char('g') => self.pending_g = true,
            KeyCode::Char('G') => self.go_bottom(),
            KeyCode::Char('C') if self.focus == Focus::Playlist => {
                self.playlist.clear();
                self.playlist_sel = 0;
                self.clear_now_playing();
                self.set_status("playlist cleared");
            }
            KeyCode::Enter => self.activate(),
            KeyCode::Char('a') if self.focus == Focus::Results => self.add_selected_to_playlist(),
            KeyCode::Char('A') if self.focus == Focus::Results => self.add_all_to_playlist(),
            KeyCode::Char('m') if self.focus == Focus::Results => self.load_more_results(),
            KeyCode::Char(' ') | KeyCode::Char('p') => self.toggle_pause(),
            KeyCode::Char('h') => {
                if ctrl && self.focus == Focus::Playlist {
                    self.focus = Focus::Results
                }
            }
            KeyCode::Char('l') => {
                if ctrl && self.focus == Focus::Results {
                    self.focus = Focus::Playlist;
                } else {
                    self.cycle_loop();
                }
            }
            KeyCode::Char('r') => {
                // Quick toggle: loop-one on/off.
                self.loop_mode = match self.loop_mode {
                    LoopMode::One => LoopMode::Off,
                    _ => LoopMode::One,
                };
                self.set_status(format!("loop: {}", self.loop_mode.label()));
            }
            KeyCode::Char('+') | KeyCode::Char('=') => self.vol_delta(5.0),
            KeyCode::Char('-') => self.vol_delta(-5.0),
            _ => {}
        }
    }

    fn take_pending_count(&mut self, default: i32) -> i32 {
        self.pending_count
            .take()
            .map(|n| n as i32)
            .unwrap_or(default)
    }

    fn on_key_search(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Tab => {
                self.focus = Focus::Results;
            }
            KeyCode::Enter => {
                let q = self.query.trim().to_string();
                if !q.is_empty() {
                    self.last_query = q.clone();
                    self.results_limit = RESULTS_PER_QUERY;
                    self.jump_to_on_results = None;
                    self.search_req = self.search_req.wrapping_add(1);
                    self.searching = true;
                    spawn_search(
                        q,
                        self.results_limit,
                        self.search_req,
                        self.search_tx.clone(),
                    );
                    self.set_status("searching…");
                }
            }
            KeyCode::Backspace => {
                self.query.pop();
            }
            KeyCode::Char('j') if ctrl => self.focus = Focus::Results,
            KeyCode::Char('l') if ctrl => self.focus = Focus::Playlist,
            KeyCode::Char(c) => {
                self.query.push(c);
            }
            _ => {}
        }
    }

    fn list_len(&self) -> usize {
        match self.focus {
            Focus::Results => self.results.len(),
            Focus::Playlist => self.playlist.len(),
            Focus::Search => 0,
        }
    }

    fn sel_mut(&mut self) -> &mut usize {
        match self.focus {
            Focus::Results => &mut self.results_sel,
            Focus::Playlist => &mut self.playlist_sel,
            Focus::Search => &mut self.results_sel,
        }
    }

    fn move_sel(&mut self, delta: i32) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        let sel = self.sel_mut();
        let cur = *sel as i32;
        let mut next = cur + delta;
        if next < 0 {
            next = 0;
        }
        if next as usize >= len {
            next = (len - 1) as i32;
        }
        *sel = next as usize;
    }

    fn go_top(&mut self) {
        *self.sel_mut() = 0;
    }
    fn go_bottom(&mut self) {
        let len = self.list_len();
        if len > 0 {
            *self.sel_mut() = len - 1;
        }
    }

    fn activate(&mut self) {
        match self.focus {
            Focus::Results => {
                if let Some(t) = self.results.get(self.results_sel).cloned() {
                    self.playlist.push(t);
                    let idx = self.playlist.len() - 1;
                    self.playlist_sel = idx;
                    self.play_index(idx);
                }
            }
            Focus::Playlist => {
                if self.playlist_sel < self.playlist.len() {
                    self.play_index(self.playlist_sel);
                }
            }
            Focus::Search => {}
        }
    }

    fn add_selected_to_playlist(&mut self) {
        if let Some(t) = self.results.get(self.results_sel).cloned() {
            self.playlist.push(t.clone());
            self.set_status(format!("added: {}", trunc(&t.title, 60)));
        }
    }

    fn add_all_to_playlist(&mut self) {
        let n = self.results.len();
        self.playlist.extend(self.results.iter().cloned());
        self.set_status(format!("added {n} tracks"));
    }

    fn load_more_results(&mut self) {
        if self.last_query.is_empty() || self.searching {
            return;
        }
        let old_count = self.results.len();
        self.results_limit = self.results_limit.saturating_add(RESULTS_PER_QUERY);
        self.search_req = self.search_req.wrapping_add(1);
        self.searching = true;
        // After results come back, jump the cursor to the first new item so
        // the user sees what was added without having to scroll.
        self.jump_to_on_results = Some(old_count);
        spawn_search(
            self.last_query.clone(),
            self.results_limit,
            self.search_req,
            self.search_tx.clone(),
        );
        self.set_status(format!("loading up to {} results…", self.results_limit));
    }

    fn delete_playlist_current(&mut self) {
        if self.playlist_sel >= self.playlist.len() {
            return;
        }
        let removed = self.playlist.remove(self.playlist_sel);
        match self.current {
            Some(c) if c == self.playlist_sel => self.clear_now_playing(),
            Some(c) if c > self.playlist_sel => self.current = Some(c - 1),
            _ => {}
        }
        if self.playlist_sel >= self.playlist.len() && self.playlist_sel > 0 {
            self.playlist_sel -= 1;
        }
        self.set_status(format!("removed: {}", trunc(&removed.title, 60)));
    }

    fn clear_now_playing(&mut self) {
        self.current = None;
        self.time_pos = f64::NAN;
        self.duration = f64::NAN;
        self.paused = false;
        self.loading = false;
        // Invalidate any in-flight resolve so a late completion can't
        // resurrect playback after we cleared (e.g. dd on current track
        // while yt-dlp is still resolving its URL).
        self.play_req = self.play_req.wrapping_add(1);
        let _ = self.player.stop();
    }

    fn play_index(&mut self, idx: usize) {
        let Some(t) = self.playlist.get(idx).cloned() else {
            return;
        };
        self.current = Some(idx);
        self.time_pos = f64::NAN;
        self.duration = f64::NAN;
        self.paused = false;
        self.loading = true;
        self.error_retries = 0;
        self.play_req = self.play_req.wrapping_add(1);
        spawn_resolve(self.play_req, t.url(), self.resolve_tx.clone());
        self.set_status(format!("resolving: {}", trunc(&t.title, 60)));
    }

    pub fn on_resolve_event(&mut self, ev: ResolveEvent) {
        match ev {
            ResolveEvent::Resolved { req, stream_url } => {
                if req != self.play_req {
                    return;
                }
                if let Err(e) = self.player.loadfile(&stream_url) {
                    self.loading = false;
                    self.set_status(format!("mpv error: {e}"));
                    return;
                }
                if let Some(t) = self.current.and_then(|i| self.playlist.get(i)) {
                    self.set_status(format!("playing: {}", trunc(&t.title, 60)));
                }
            }
            ResolveEvent::Failed { req, err } => {
                if req != self.play_req {
                    return;
                }
                self.loading = false;
                self.set_status(format!("resolve failed: {}", trunc(&err, 120)));
            }
        }
    }

    fn toggle_pause(&mut self) {
        if self.current.is_none() {
            return;
        }
        let _ = self.player.toggle_pause();
    }

    fn next_track(&mut self, inc: usize, auto: bool) {
        let len = self.playlist.len();
        if len == 0 {
            return;
        }
        let Some(cur) = self.current else {
            // Nothing playing yet — start from the top.
            self.playlist_sel = 0;
            self.play_index(0);
            return;
        };
        let next = cur.saturating_add(inc);
        if next < len {
            self.playlist_sel = next;
            self.play_index(next);
        } else if matches!(self.loop_mode, LoopMode::Playlist) {
            self.playlist_sel = 0;
            self.play_index(0);
        } else if !auto {
            self.set_status("end of playlist");
        } else {
            self.clear_now_playing();
        }
    }

    fn prev_track(&mut self, dec: usize) {
        let len = self.playlist.len();
        if len == 0 {
            return;
        }
        let Some(cur) = self.current else {
            self.playlist_sel = 0;
            self.play_index(0);
            return;
        };
        let prev = if cur >= dec {
            cur - dec
        } else if matches!(self.loop_mode, LoopMode::Playlist) {
            len - 1
        } else {
            0
        };
        self.playlist_sel = prev;
        self.play_index(prev);
    }

    fn cycle_loop(&mut self) {
        self.loop_mode = match self.loop_mode {
            LoopMode::Off => LoopMode::Playlist,
            LoopMode::Playlist => LoopMode::One,
            LoopMode::One => LoopMode::Off,
        };
        self.set_status(format!("loop: {}", self.loop_mode.label()));
    }

    fn vol_delta(&mut self, delta: f64) {
        let _ = self.player.add_volume(delta);
    }

    fn seek(&mut self, secs: f64) {
        if self.current.is_some() {
            let _ = self.player.seek(secs);
        }
    }

    pub fn on_search_event(&mut self, ev: SearchEvent) {
        match ev {
            SearchEvent::Results(id, tracks) => {
                if id != self.search_req {
                    return;
                }
                self.searching = false;
                let n = tracks.len();
                self.results = tracks;
                self.results_sel = self
                    .jump_to_on_results
                    .take()
                    .filter(|&j| j < self.results.len())
                    .unwrap_or(0);
                if self.focus == Focus::Search {
                    self.focus = Focus::Results;
                }
                self.set_status(format!("{n} results"));
            }
            SearchEvent::Error(id, e) => {
                if id != self.search_req {
                    return;
                }
                self.searching = false;
                self.set_status(format!("search error: {}", trunc(&e, 80)));
            }
        }
    }

    pub fn on_player_event(&mut self, ev: PlayerEvent) {
        match ev {
            PlayerEvent::TimePos(t) => {
                self.time_pos = t;
                if t.is_finite() {
                    self.loading = false;
                }
            }
            PlayerEvent::Duration(d) => self.duration = d,
            PlayerEvent::Paused(p) => self.paused = p,
            PlayerEvent::Volume(v) => self.volume = v,
            PlayerEvent::StartFile => {
                self.paused = false;
                self.loading = true;
                self.error_retries = 0;
            }
            PlayerEvent::EndFile { reason } => match reason.as_str() {
                "eof" => match self.loop_mode {
                    LoopMode::One => {
                        if let Some(i) = self.current {
                            self.play_index(i);
                        }
                    }
                    LoopMode::Playlist | LoopMode::Off => self.next_track(1, true),
                },
                "error" => {
                    if self.error_retries < 3 {
                        if let Some(t) = self.current.and_then(|i| self.playlist.get(i)).cloned() {
                            self.error_retries += 1;
                            self.set_status(format!(
                                "playback error, retrying… ({}/3)",
                                self.error_retries
                            ));
                            self.play_req = self.play_req.wrapping_add(1);
                            self.loading = true;
                            spawn_resolve(self.play_req, t.url(), self.resolve_tx.clone());
                        } else {
                            self.loading = false;
                        }
                    } else {
                        self.loading = false;
                        self.set_status("mpv: playback error (gave up after 3 retries)");
                    }
                }
                _ => {}
            },
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Save the playlist on any exit path — clean shutdown or panic
        // unwind. Player::Drop handles the mpv-quit side.
        save_playlist(&self.playlist);
    }
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn config_dir() -> Option<PathBuf> {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let mut p = PathBuf::from(appdata);
        p.push(env!("CARGO_PKG_NAME"));
        return Some(p);
    }
    if let Ok(home) = std::env::var("HOME") {
        let mut p = PathBuf::from(home);
        p.push(".config");
        p.push(env!("CARGO_PKG_NAME"));
        return Some(p);
    }
    None
}

fn playlist_path() -> Option<PathBuf> {
    let mut p = config_dir()?;
    let _ = fs::create_dir_all(&p);
    p.push("playlist.json");
    Some(p)
}

fn load_playlist() -> Option<Vec<Track>> {
    let p = playlist_path()?;
    let bytes = fs::read(&p).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn save_playlist(tracks: &[Track]) {
    if let Some(p) = playlist_path() {
        if let Ok(s) = serde_json::to_string_pretty(tracks) {
            let _ = fs::write(p, s);
        }
    }
}
