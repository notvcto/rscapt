<div align="center">

<img src="rscapt.png" width="140" alt="rscapt" />

# rscapt

**OBS replay buffer → 1440p clip processor for Windows**

[![License: MIT](https://img.shields.io/badge/License-MIT-5c6bc0?style=flat-square)](LICENSE)
[![GitHub Stars](https://img.shields.io/github/stars/notvcto/rscapt?style=flat-square&color=ffd700&label=Stars)](https://github.com/notvcto/rscapt)
[![Windows](https://img.shields.io/badge/Windows-x64-0078d4?style=flat-square&logo=windows&logoColor=white)](#getting-started)
[![Rust](https://img.shields.io/badge/Rust-2024_edition-f74c00?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![OBS](https://img.shields.io/badge/OBS-WebSocket_v5-302e31?style=flat-square&logo=obsstudio&logoColor=white)](https://obsproject.com)
[![0x0.st](https://img.shields.io/badge/share-0x0.st-4caf50?style=flat-square)](#sharing)

<br>

*rscapt watches your OBS replay buffer and automatically upscales every saved clip to 1440p with Lanczos. From the built-in TUI you can post-process (motion interpolation, motion blur, colour grading), compress with NVENC / x265 / AV1, and share to [0x0.st](https://0x0.st) — all without touching a timeline editor.*

</div>

---

## Features

| Area | What it does |
|---|---|
| **Auto-upscale** | Every replay save → Lanczos upscale to 1440p, non-blocking background job |
| **Post-process** | Per-clip: motion interpolation (up to 120 fps), motion blur (shutter angle), saturation, sharpen |
| **Compress** | H.264 NVENC · HEVC NVENC · x265 · AV1 × High / Med / Low quality presets; optional trim |
| **Share** | Upload to [0x0.st](https://0x0.st), auto-copy URL to clipboard, one-key delete |
| **Clip library** | Browse and manage every clip from the TUI; size, retention days, and share status at a glance |
| **OBS management** | Optionally downloads portable OBS, writes a dedicated profile & replay buffer config, launches it silently |
| **First-run wizard** | Ratatui TUI wizard on first launch — no config file editing required |
| **Silent autostart** | Daemon starts on login via a VBS launcher (zero console flash) |
| **Mouse support** | Click to select, scroll to navigate — full mouse support in both TUIs |

---

## Prerequisites

**ffmpeg** must be installed and available in your PATH. The easiest way:

```powershell
winget install Gyan.FFmpeg
```

Then open a new terminal and verify with `ffmpeg -version`. That's it — no manual PATH editing needed.

---

## Getting Started

### Download

Grab the latest `rscapt-vX.Y.Z-windows-x64.exe` from the [Releases](https://github.com/notvcto/rscapt/releases) page.

> **Windows SmartScreen warning:** Because rscapt is not code-signed, Windows will show a "Windows protected your PC" warning on first run. Click **More info → Run anyway** to proceed. This is normal for open-source tools without a paid signing certificate — the exe is built transparently from this repo via GitHub Actions.

### First run

Double-click the exe. The setup wizard opens and walks you through:

1. **OBS** — download portable OBS automatically, point to your existing install, or skip
2. **Output folder** — where upscaled clips are saved (default: `Videos\Captures`)
3. **Buffer duration** — how many seconds OBS keeps in the replay buffer (15–600 s)
4. **Capture source** — Game Capture (recommended) or Display Capture
5. **Autostart** — register the daemon to start silently on login

After the wizard completes, run `rscapt daemon` to start processing immediately (or log out and back in if you enabled autostart).

> **Existing OBS users:** rscapt never touches your scenes or settings. It only listens for replay buffer saves. Make sure the replay buffer is enabled in OBS under **Settings → Output → Replay Buffer**, and that it's running before you try to save a clip. The downloaded/managed OBS instance has this configured and started automatically.

### Open the TUI

```
rscapt tui
```

Or launch **rscapt** from the Start Menu.

---

## TUI reference

### Main view

| Key | Action |
|---|---|
| `Tab` | Switch focus between Jobs and Clips panels |
| `j` / `↓` | Select next item |
| `k` / `↑` | Select previous item |
| `q` / `Esc` | Quit |

**Jobs panel** (left):

| Key | Action |
|---|---|
| `c` | Cancel the selected job |

**Clips panel** (right):

| Key | Action |
|---|---|
| `p` | Open post-process modal |
| `x` | Open compress modal |
| `s` | Share clip (upload to 0x0.st) |
| `d` | Delete share link for the selected clip |

### Post-process modal

| Key | Action |
|---|---|
| `j` / `↑` `k` / `↓` | Navigate effects |
| `Space` | Toggle effect on/off |
| `l` / `+` / `→` | Increase value |
| `h` / `-` / `←` | Decrease value |
| `Enter` | Confirm and queue job |
| `Esc` | Cancel |

### Compress modal

| Key | Action |
|---|---|
| `Tab` / `↓` | Next field |
| `↑` | Previous field |
| `←` / `→` | Cycle codec or quality |
| `Enter` | Confirm and queue job |
| `Esc` | Cancel |

Mouse clicks and scroll work throughout both TUIs.

---

## Sharing

rscapt uploads clips to [0x0.st](https://0x0.st), a no-signup file host. After a Share job completes the URL is automatically copied to your clipboard.

Retention is calculated by 0x0.st based on file size: smaller clips are kept longer (up to ~365 days; 512 MB files expire after 30 days). The rscapt TUI shows the estimated retention next to each clip.

To remove a shared clip from 0x0.st: select it in the Clips panel and press `d` (or open the Share modal and press `d`).

---

## Configuration

Config is stored at `%APPDATA%\rscapt\config.json`. All fields are set by the wizard; edit manually to tune.

| Field | Default | Description |
|---|---|---|
| `obs_host` | `127.0.0.1` | OBS WebSocket host |
| `obs_port` | `4455` | OBS WebSocket port |
| `obs_password` | `""` | OBS WebSocket password |
| `output_dir` | `Videos\Captures` | Where upscaled clips land |
| `encoder` | `h264_nvenc` | ffmpeg encoder for the upscale pass |
| `ipc_port` | `19874` | Localhost port for daemon ↔ TUI IPC |
| `obs_exe_path` | `""` | Full path to `obs64.exe` (if managed) |
| `obs_managed` | `false` | Whether rscapt launches/manages OBS |
| `replay_buffer_seconds` | `30` | Replay buffer duration written to the OBS profile |
| `capture_source` | `"game"` | `"game"` or `"display"` for the OBS scene source type |

---

## Commands

```
rscapt               First-run wizard (if no config) or start daemon
rscapt daemon        Start the background clip processor
rscapt tui           Open the clip manager TUI
rscapt tray          Start as a system tray icon (autostart entry point)
rscapt setup         Re-run the setup wizard
rscapt install       Create Start Menu shortcut + register autostart
rscapt uninstall     Remove shortcuts, autostart, and PATH entry
rscapt update        Check for updates and install if a newer version is available
```

---

## Building from source

Requires Rust (2024 edition), [Zig 0.14](https://ziglang.org/download/), and [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild).

```bash
# Native (Linux — for development/testing)
cargo build --release

# Windows x64 (cross-compile from Linux)
cargo zigbuild --release --target x86_64-pc-windows-gnu
```

The GitHub Actions workflow builds the Windows exe automatically on every tagged release.

---

## License

MIT — see [LICENSE](LICENSE).
