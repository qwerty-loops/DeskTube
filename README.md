# DeskTube

A Windows music player for YouTube Music, built with Tauri, Rust, and Svelte. Audio plays through a native Rust backend rather than an Electron app.

**Status:** early development. Windows x64 is the current target. Internet access is required. DeskTube is not affiliated with Google or YouTube.

## What it does

- Search for songs, artists, and albums; play music without signing in.
- Connect Google to access liked music and playlists.
- Discover tracks in Explore, based on recent listening, likes, and YouTube radio mixes.
- Use radio, shuffle, Play next, crossfade, and saved playback sessions.
- View lyrics and an expanded Now Playing screen.
- Control playback with Windows media controls and keyboard shortcuts.
- Choose an audio output device, set a sleep timer, or control playback from a phone.

## Install

If a published release is available, download its Windows installer from this repository's **Releases** section. Install it and open DeskTube.

If no installer is available, use the source instructions below. The release workflow creates draft releases; an installer is not available to users until the maintainer publishes one.

Builds are currently unsigned. Windows may warn or block them. Do not disable Windows security protections to install the app.

## Google sign-in (optional)

Search and playback work without an account. Liked music and account playlists require Google sign-in.

**Sign-in is not configured out of the box yet.** For now:

1. Create a project in Google Cloud Console and enable **YouTube Data API v3**.
2. Configure the OAuth consent screen. If the project is in Testing, add your Google account as a test user.
3. Create an OAuth client with the type **TVs and Limited Input devices**, then download its JSON file.
4. Press `Win + R`, enter `%APPDATA%\DeskTube`, and create the folder if needed.
5. Save the downloaded file there as `google-oauth.json`.
6. Restart DeskTube and click **Connect Google**. Follow the browser instructions and enter the code shown in the app.

Keep this JSON private and out of Git. Saved sign-in tokens use Windows Credential Manager. Google may require additional consent or verification depending on the project's configuration.

## Phone remote

1. Keep DeskTube running on your PC.
2. Connect your phone to the same local network.
3. Click the phone icon at the top right and scan the QR code.
4. Allow DeskTube through Windows Firewall on your trusted private network if prompted.

The phone controls playback; **audio stays on the PC**. To use a Bluetooth speaker, pair it with the PC and select the appropriate audio output.

The pairing link contains a temporary access token. Do not share it or expose the remote server to the internet. Scan again after restarting DeskTube.

## Keyboard shortcuts

These apply while the app is focused and you are not typing in an input.

| Key | Action |
| --- | --- |
| Space | Play / pause |
| Left / Right | Seek backward / forward 10 seconds |
| Ctrl + Left / Right | Previous / next track |
| Up / Down | Adjust volume |
| M | Mute / unmute |
| / | Focus search |
| Esc | Close popovers or exit expanded view |

## Run from source

### Requirements

- Windows x64 with Microsoft Edge WebView2 Runtime.
- Node.js (a supported LTS version) and pnpm 9.
- Rust stable with the MSVC toolchain.
- Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK.
- The Windows x64 `yt-dlp.exe` from the official yt-dlp GitHub releases.

### Setup

Clone this repository, open PowerShell in its folder, then run:

```powershell
pnpm.cmd install --frozen-lockfile
New-Item -ItemType Directory -Force src-tauri/binaries
```

Place the downloaded `yt-dlp.exe` in that folder and rename it to:

```text
src-tauri/binaries/yt-dlp-x86_64-pc-windows-msvc.exe
```

Start the desktop app:

```powershell
pnpm.cmd tauri dev
```

Keep the development terminal open while using it. `pnpm.cmd dev` alone starts only the frontend, not the desktop backend.

### Build installers

```powershell
pnpm.cmd tauri build
```

Build output is under `src-tauri/target/release/bundle/`. The VBS/PowerShell launchers in this repository are local development helpers, not the installation method for other users.

### Check changes

Run from the repository root:

```powershell
pnpm.cmd check
pnpm.cmd build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

## Known limitations

- YouTube changes can break search, sign-in, lyrics, or playback. Availability is not guaranteed.
- Like/unlike actions use YouTube's rating API; they may not match YouTube Music's liked-library state immediately or exactly.
- Phone remote is a controller, not audio casting.
- Linux, macOS, and Raspberry Pi are not currently supported targets.
- This project uses unofficial YouTube Music endpoints. Review YouTube's terms and applicable rules before using or distributing it.

## Report a problem

Open an issue with what happened, the steps to reproduce it, your Windows version, and the DeskTube version or commit you used. Remove OAuth files, tokens, and phone pairing links from logs and screenshots before sharing them.
