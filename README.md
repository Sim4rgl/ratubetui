# ratubetui

A vim-style YouTube music player for the terminal. Audio-only, best available
quality (typically 160 kbps Opus), with a persistent playlist and a small,
keyboard-driven TUI built on [ratatui](https://github.com/ratatui-org/ratatui).

```
┌─ Search ───────────────────────────────────────────────────┐
│ ▶ lofi hip hop                                              │
├─ Results (30) ──────────────────────────┬─ Playlist (2/3) ──┤
│ ▶  1. lofi hip hop radio — Lofi Girl     │ ♪  1. Foo  3:12   │
│    2. relaxing study beats — XYZ         │    2. Bar  2:55   │
│    …                                     │    3. Baz  5:01   │
├──────────────────────────────────────────┴───────────────────┤
│ Now Playing                                                   │
│ ▶ Foo    Foo Music                                            │
│ [██████░░░░░░░░░░░░░░░] 1:23 / 3:12                           │
│ vol 75%    loop list    playing                              │
├──────────────────────────────────────────────────────────────┤
│ /:search  Tab:focus  j/k:move  Enter:play  a:add  m:more  …  │
└──────────────────────────────────────────────────────────────┘
```

## Features

- Search YouTube from the TUI (powered by `yt-dlp`)
- Vim-style navigation: `j`/`k`, `gg`/`G`, `Ctrl-d`/`Ctrl-u`, `Tab`
- Numeric prefix for motion keys: `3j`, `10k`, etc.
- Build a playlist: `a`, `A`, `Enter`, `dd`, `C`
- Play / pause, next / prev, loop (off → list → one), volume
- Seek forward/backward with `>`/`<` (hold for 60s jumps)
- **Audio only** — no video stream is downloaded
- **Best quality** — `bestaudio` format selection (Opus 160 kbps when available)
- "Load more results" with `m` to extend a search by another 30 hits
- Auto-retry on playback error (e.g. expired stream URL) — up to 3 attempts
- Playlist auto-saves to `%APPDATA%\ratubetui\playlist.json` and reloads on next start
- Terminal is always restored on exit, even on panic

## Dependencies

External tools — must be on `PATH`:

| tool        | what it does                                |
|-------------|---------------------------------------------|
| **mpv**     | audio backend, controlled via its JSON-IPC  |
| **yt-dlp**  | YouTube search + stream-URL resolution      |

To build from source:

- **Rust 1.75+** (2021 edition) — get it from [rustup.rs](https://rustup.rs)
- **Windows** with the MSVC toolchain (Visual Studio Build Tools or VS)

> **Platform support.** Only Windows is currently supported. The mpv IPC
> layer uses Windows named pipes (`\\.\pipe\…`) and the reply-drain uses the
> `windows-sys` `PeekNamedPipe`/`ReadFile` calls. Linux/macOS would need a
> separate Unix-socket path; the `#[cfg(not(windows))]` placeholder in the
> code is a stub, not a working backend.

## Installing the runtime tools

### Chocolatey

```powershell
choco install mpv yt-dlp
```

### Scoop

```powershell
scoop install mpv yt-dlp
```

Verify both resolve:

```powershell
mpv --version
yt-dlp --version
```

## Building

```powershell
git clone https://github.com/<you>/ratubetui.git
cd ratubetui
cargo build --release
```

> **Build from PowerShell, not Git Bash.** Git's `/usr/bin/link` shadows
> MSVC's `link.exe`, and cargo will fail with strange linker errors. From a
> real PowerShell or `cmd` window everything works.

The binary is at `target\release\ratubetui.exe`. Drop it somewhere on your `PATH`
to launch it as just `ratubetui`.

## Running

```powershell
.\target\release\ratubetui.exe
```

or once it's on `PATH`:

```powershell
ratubetui
```

## Keys

The screen has four zones. The focused pane has a **cyan** border.

### Search

| key                | action                              |
|--------------------|-------------------------------------|
| `/`                | focus the search box (clears query) |
| (type) / Backspace | edit the query                      |
| `Enter`            | run the search (up to 30 results)   |
| `Tab`              | leave the box, focus Results        |

### Movement

| key                  | action                              |
|----------------------|-------------------------------------|
| `j` / `k` / arrows  | down / up                           |
| `[n]j` / `[n]k`     | move n lines down / up              |
| `gg` / `G`           | jump to top / bottom                |
| `Ctrl-d` / `Ctrl-u`  | half-page down / up                 |
| `Tab`                | toggle focus Results ↔ Playlist     |
| `Ctrl-k`             | move focus up to Search             |
| `Ctrl-j`             | move focus down (Search → Results)  |
| `Ctrl-l`             | move focus right (Results → Playlist, Search → Playlist) |
| `Ctrl-h`             | move focus left (Playlist → Results)|

### Playlist editing

| key     | where     | action                                       |
|---------|-----------|----------------------------------------------|
| `Enter` | Results   | add highlighted result **and** play it       |
| `Enter` | Playlist  | play highlighted track                       |
| `a`     | Results   | add highlighted result (no play)             |
| `A`     | Results   | add **all** current results                  |
| `m`     | Results   | load 30 more results for the same query      |
| `dd`    | Playlist  | remove highlighted track                     |
| `[n]dd` | Playlist  | remove n tracks from highlighted position    |
| `C`     | Playlist  | clear the entire playlist                    |

### Playback

| key             | action                                       |
|-----------------|----------------------------------------------|
| `Space` or `p`  | play / pause                                 |
| `n` / `N`       | next / previous track                        |
| `l`             | cycle loop mode (off → list → one)           |
| `r`             | quick toggle repeat-one                      |
| `>` / `<`       | seek +10s / -10s (hold: +60s / -60s)        |
| `+` / `-`       | volume ±5                                    |

### App-wide

| key            | action                                       |
|----------------|----------------------------------------------|
| `?`            | toggle help overlay                          |
| `q` or `Esc`   | quit                                         |

### Now Playing glyphs

| glyph | meaning                                                  |
|-------|----------------------------------------------------------|
| `■`   | idle (no track selected)                                 |
| `…`   | loading (yt-dlp resolving / mpv buffering)               |
| `▶`   | playing                                                  |
| `❚❚`  | paused                                                   |

## Persistence

- **Playlist** is saved to `%APPDATA%\ratubetui\playlist.json` on any exit (clean
  shutdown or panic) and auto-loaded on the next launch.
- To wipe it, delete that file, or press `C` in the Playlist pane.

## How it works

- **mpv** is launched with `--idle --no-video --audio-display=no
  --ytdl-format=bestaudio[ext=m4a]/bestaudio/best` and controlled over its
  JSON IPC.
- **Two pipe connections** are opened to mpv — one purely for reading events
  (subscribed to `time-pos`, `duration`, `pause`, `volume`) and one purely
  for writing commands. Two are needed because Windows synchronous file
  handles serialize I/O at the kernel: a blocked `ReadFile` on one
  cloned handle blocks `WriteFile` on another. Two separate client
  connections give each thread its own independent pipe instance.
- **yt-dlp pre-resolves** the audio stream URL with `yt-dlp -g -f
  bestaudio/best`, and ratubetui hands the direct `googlevideo.com` URL to mpv.
  This skips mpv's `ytdl_hook`, which can stall silently on Windows.
- **Auto-retry on error** — if mpv reports a playback error (e.g. an expired
  stream URL), ratubetui automatically re-resolves a fresh URL and retries up
  to 3 times before giving up.
- **Reply buffer drain** — every command we send produces a small reply on
  the writer pipe. ratubetui calls `PeekNamedPipe` + `ReadFile` (via
  `windows-sys`) before each write to clear those replies, so the kernel
  buffer never fills.
- **Always non-blocking UI** — search and resolve run on per-call worker
  threads, the mpv IPC reader and writer each have their own thread, and
  the main thread only handles input + render at ~20 Hz.

## Troubleshooting

| symptom                                 | likely cause / fix                                                  |
|-----------------------------------------|---------------------------------------------------------------------|
| "failed to spawn mpv (is it on PATH?)"  | install mpv, or open a fresh terminal so it picks up `PATH`          |
| `resolve failed: …`                     | yt-dlp couldn't fetch the video — check the URL, check network, run `yt-dlp -U` |
| `mpv: playback error …`                 | retried 3 times and gave up — check network or try the track again  |
| stuck on `loading…`                     | open an issue with the last few lines of the status bar              |
| linker errors when building             | you're in Git Bash — rebuild from PowerShell                         |

