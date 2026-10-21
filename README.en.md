# vncviewer

[Français](README.md) · **English**

[![CI](https://github.com/JeanJacquesSerpoul/vncviewer/actions/workflows/ci.yml/badge.svg)](https://github.com/JeanJacquesSerpoul/vncviewer/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/JeanJacquesSerpoul/vncviewer)](https://github.com/JeanJacquesSerpoul/vncviewer/releases/latest)

Remote desktop over VNC — a desktop viewer written in Rust with
[Tauri 2](https://tauri.app) and [Leptos](https://leptos.dev).

## Download

Installers for the latest version are on the
**[Releases](https://github.com/JeanJacquesSerpoul/vncviewer/releases/latest)** page:

| System | File |
|---|---|
| Debian, Ubuntu | `.deb`: `sudo apt install ./VNC.Viewer_*_amd64.deb` |
| Fedora, openSUSE | `.rpm` |
| Other Linux | `.AppImage` (make it executable, then run it) |
| Windows | `-setup.exe` or `.msi` |
| macOS Apple Silicon / Intel | `aarch64.dmg` / `x64.dmg` |

The installers are not signed: Windows (SmartScreen) and macOS ask for confirmation
on first launch; the steps are described on the release page. To build from source,
see below.

## Features

- RFB 3.3 / 3.7 / 3.8 protocol implemented in Rust (no external VNC library).
- Security: none, or VNC authentication (password, DES).
- Encodings: ZRLE, Hextile, RRE, CopyRect, Raw; cursor pseudo-encoding
  (the remote cursor is drawn locally), desktop resize and LastRect.
- Full keyboard support (X11 keysyms, numeric keypad, AltGr),
  3-button mouse and wheel, Ctrl+Alt+Del.
- Two-way clipboard: text copied on the server lands in the local clipboard; the
  local clipboard is sent when the view regains focus (or with the "Clipboard" button).
- "Fit" mode (scaled to the window) or actual size with scrolling, full screen.
- French / English interface: system language by default, FR/EN switch on the
  connection screen and in the toolbar, choice remembered.
- VNC address notations: `host`, `host:1` (display → port 5901), `host::5901`, `[::1]:0`.

## Architecture

```
src/              Leptos UI (WebAssembly, built by Trunk)
  app.rs          connection screen and toolbar
  viewer.rs       canvas rendering, keyboard/mouse input queue
  keysym.rs       KeyboardEvent → X11 keysym
  i18n.rs         FR/EN texts and error translation
  wire.rs         decoding of the backend's binary messages
src-tauri/        Tauri application
  src/rfb/        RFB client: handshake, auth, decoders, framebuffer
  src/session.rs  TCP connection, read/write tasks, flow control
  src/lib.rs      Tauri commands
```

The backend reports errors as language-neutral codes (`{code, detail}`); the UI
translates them when displaying them.

The backend decodes rectangles into an RGBA framebuffer and sends only the changed
areas to the UI through a binary Tauri `Channel`; the UI draws them with
`putImageData` without copying them into wasm memory, then acknowledges the frame
before the backend requests the next one (no latency build-up).

## Requirements

- Stable Rust with the wasm target: `rustup target add wasm32-unknown-unknown`
- [Trunk](https://trunkrs.dev) and the Tauri CLI: `cargo install trunk tauri-cli`
- On Linux, Tauri's system dependencies (WebKitGTK 4.1, etc.), see
  <https://tauri.app/start/prerequisites/>

## Build and install on Linux

```bash
./install.sh --deps
```

The script builds the application in release mode and installs it into `~/.local`:
the `vnc-viewer` binary (named so as not to shadow TigerVNC's `vncviewer`), a menu
entry and icons. `--deps` first installs the build dependencies (apt, dnf or pacman)
and Rust if missing; not needed if they are already present. The script's messages
are in French.

| Option | Effect |
|---|---|
| `--system` | installs for all users into `/usr/local` (sudo) |
| `--prefix DIR` | another installation directory |
| `--skip-build` | reinstalls the already built binary |
| `--uninstall` | uninstalls (with the same `--system` / `--prefix`) |

## Run in development

```bash
cargo tauri dev
```

## Build the packages

```bash
cargo tauri build
```

## Tests

```bash
cargo test --workspace
```

The backend tests play a full session against an in-memory fake VNC server
(authentication, every encoding, cursor, clipboard, resize).

## Limitations

- No encryption (VeNCrypt/TLS) and no proprietary security types
  (Apple ARD, RealVNC): use an SSH tunnel for non-local connections,
  for example `ssh -L 5901:localhost:5901 server`, then connect to `localhost:1`.
- Tight encoding is not supported (ZRLE is used instead).
