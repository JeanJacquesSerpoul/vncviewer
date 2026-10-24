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
- Security:
  - none, VNC authentication (password, DES) or Apple authentication (macOS
    Screen Sharing, user name and password);
  - **VeNCrypt with TLS** (X509None, X509Vnc, X509Plain): a certificate signed by
    a known authority is accepted directly; a self-signed certificate is shown
    with its SHA-256 fingerprint and remembered once trusted;
  - built-in **SSH tunnel** (`user@gateway[:port]`): host key checked against
    `~/.ssh/known_hosts` (added once trusted), authentication with the SSH agent,
    the keys in `~/.ssh` or a password;
  - the padlock in the toolbar tells whether the session is encrypted.
- The password is only asked for when the server requires one; it can be saved
  in the system keychain (never in the settings).
- Encodings: Tight (JPEG, palette, gradient), ZRLE, Hextile, RRE, CopyRect, Raw;
  three quality levels (lossless, good, low bandwidth); cursor, desktop resize
  (including ExtendedDesktopSize) and LastRect pseudo-encodings.
- Full keyboard support (X11 keysyms, numeric keypad, AltGr), 3-button mouse and
  wheel; "Keys" menu: Ctrl+Alt+Del, Alt+Tab, Windows key, Print Screen, F1–F12…
- Two-way clipboard, in UTF-8 with servers that support the extended clipboard
  (Latin-1 otherwise): text copied on the server lands in the local clipboard;
  the local clipboard is sent when the view regains focus (or with its button).
- Display: fit to the window or zoom from 50 to 200 %, optional smoothing, remote
  resolution following the window (when the server allows it).
- Full screen with an auto-hidden toolbar: it shows at the top edge or with F8;
  Ctrl+Alt+Enter toggles full screen.
- View-only mode, PNG screenshot, frames per second and bandwidth in the toolbar.
- Automatic reconnection when the connection drops; cancellable connection.
- Connection history and favourites; several windows (one session each).
- Light or dark theme following the system; window size and position remembered.
- French / English interface: system language by default, FR/EN switch on the
  connection screen and in the toolbar, choice remembered.
- VNC address notations: `host`, `host:1` (display → port 5901), `host::5901`, `[::1]:0`.

## Architecture

```
src/              Leptos UI (WebAssembly, built by Trunk)
  app.rs          state, connection life cycle, reconnection
  connect.rs      connection screen, history, credential prompts
  toolbar.rs      remote view, toolbar and menus
  viewer.rs       canvas rendering, keyboard/mouse input queue
  keysym.rs       KeyboardEvent → X11 keysym
  i18n.rs         FR/EN texts and error translation
  prefs.rs        settings and history (localStorage)
  wire.rs         decoding of the backend's binary messages
src-tauri/        Tauri application
  src/rfb/        RFB client: handshake, security (VNC, ARD, VeNCrypt/TLS),
                  decoders (including Tight), extended clipboard, framebuffer
  src/ssh.rs      SSH tunnel
  src/session.rs  connection, read/write tasks, flow control
  src/lib.rs      Tauri commands (one session per window)
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
(authentication, VeNCrypt/TLS, every encoding, cursor, classic and extended
clipboard, resize).

An extra test, ignored by default, connects to a real server; the parameters are
those of the `connect` command, as JSON:

```bash
VNC_LIVE='{"host":"localhost","port":5901,"password":"secret","shared":true}' \
  cargo test -p vncviewer live -- --ignored --nocapture
```

## Limitations

- Anonymous TLS (VeNCrypt TLSNone / TLSVnc / TLSPlain subtypes) is not supported:
  when the server offers nothing else, the connection falls back to the other
  (unencrypted) security types. Set up an X.509 certificate on the server, or use
  the SSH tunnel.
- RealVNC proprietary security types (RA2) are not supported.
- SSH tunnel: keys protected by a passphrase go through the SSH agent; SSH host
  certificates are not accepted.
