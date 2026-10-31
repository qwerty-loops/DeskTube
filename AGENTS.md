# Repository guidance

## Project

DeskTube is a Windows x64 YouTube Music client built with Tauri v2, Rust, Svelte 5, and TypeScript. Keep audio playback native; do not introduce Electron or a browser-based playback engine.

## Code map

- `src/App.svelte`: main UI, queue, playback events, navigation, recommendations, session persistence, and settings. Follow existing Svelte 5 runes conventions.
- `src-tauri/src/lib.rs`: Tauri commands, shared state, and event wiring.
- `src-tauri/src/player.rs`: rodio/Symphonia playback, audio devices, gapless playback, and crossfade.
- `src-tauri/src/innertube.rs`: unofficial YouTube search, browse, radio, audio lookup, and lyrics.
- `src-tauri/src/auth.rs`: Google device sign-in, Windows Credential Manager tokens, and account library access.
- `src-tauri/src/remote.rs`: LAN phone remote and QR pairing. Audio stays on the PC.
- `src-tauri/src/smtc.rs` and `taskbar.rs`: Windows media integration.
- `.github/workflows/release.yml`: Windows installer builds and draft GitHub releases on version tags.
- `DeskTube.vbs` and `launch.ps1`: local launch helpers, not the public installation path.

## Setup

Use PowerShell commands from the repository root. Use `pnpm.cmd` if execution policy blocks `pnpm.ps1`.

Requirements: Node.js LTS, pnpm 9, Rust stable MSVC, Visual Studio C++ build tools with a Windows SDK, and WebView2.

```powershell
pnpm.cmd install --frozen-lockfile
pnpm.cmd tauri dev
```

Before starting Tauri, place the official Windows x64 yt-dlp executable at `src-tauri/binaries/yt-dlp-x86_64-pc-windows-msvc.exe`. This directory is ignored by Git; CI downloads the sidecar during release builds.

Google sign-in is optional and currently requires a local OAuth client JSON at `%APPDATA%\DeskTube\google-oauth.json`. Do not read or print its credential values. Missing configuration must not prevent signed-out startup, search, or playback.

## Verification

```powershell
pnpm.cmd check
pnpm.cmd build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

The existing Rust probes are ignored by default because they need network access or a signed-in account. A default test run with ignored probes does not verify live services. Run relevant probes explicitly when changing endpoint behavior; do not expose tokens or private response data in output.

Rebuild `dist/` after frontend changes when testing the local launcher. Restart the app after Rust changes. Build installers with `pnpm.cmd tauri build`; output is under `src-tauri/target/release/bundle/`.

## Implementation and security

- Preserve existing user edits and keep changes scoped to the requested task.
- Do not commit credentials, OAuth JSON, tokens, private keys, signing certificates, or `.env` files. Ignore rules are not a substitute for reviewing staged changes.
- Do not restore hard-coded Google API keys or borrowed OAuth client credentials. The current Innertube requests omit API-key query parameters; live search and browse probes passed after that change.
- Removing a secret from the latest source does not remove it from Git history. Obtain explicit approval before rewriting published history or force-pushing.
- Keep phone pairing tokens private. Do not expose the LAN remote to the public internet.
- Never disable OS security features or weaken repository security settings to make a build pass.
- Report verification limits honestly, especially for unofficial YouTube endpoints and Windows-only behavior.
- Do not claim Premium is required or guarantee availability for all tracks. The current public playback lookup does not use account credentials.
- Do not commit or push unless requested. Never force-push without specific approval.
