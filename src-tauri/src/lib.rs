mod auth;
mod innertube;
mod player;
mod remote;
mod smtc;
mod taskbar;

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use auth::{AuthManager, AuthStatus, DeviceAuthPrompt, LibraryTrack, PlaylistSummary};
use innertube::{AlbumPage, ArtistPage, Innertube, LyricsResult, TrackResult};
use player::Player;
use tauri::{AppHandle, Emitter};
use tauri_plugin_shell::ShellExt;

type AudioCacheEntry = Arc<Mutex<Option<PathBuf>>>;

struct AppState {
    auth: Arc<AuthManager>,
    innertube: Innertube,
    player: Player,
    audio_cache: Mutex<HashMap<String, AudioCacheEntry>>,
    play_sequence: AtomicU64,
    library_cache: Mutex<Option<Vec<LibraryTrack>>>,
    library_refreshing: AtomicBool,
}

fn library_cache_path() -> Option<PathBuf> {
    let app_data = std::env::var("APPDATA").ok()?;
    Some(
        std::path::Path::new(&app_data)
            .join("DeskTube")
            .join("liked-songs.json"),
    )
}

fn store_library_cache(state: &AppState, tracks: &[LibraryTrack]) {
    if let Some(path) = library_cache_path() {
        if let Ok(json) = serde_json::to_string(tracks) {
            let _ = std::fs::write(path, json);
        }
    }
    if let Ok(mut cache) = state.library_cache.try_lock() {
        *cache = Some(tracks.to_vec());
    }
}

fn spawn_library_refresh(app: &AppHandle, state: &Arc<AppState>) {
    if state.library_refreshing.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        let result = state.auth.liked_songs().await;
        if let Ok(tracks) = result {
            if !tracks.is_empty() {
                store_library_cache(&state, &tracks);
                let _ = app.emit("library-updated", tracks);
            }
        }
        state.library_refreshing.store(false, Ordering::SeqCst);
    });
}

#[tauri::command]
async fn auth_status(state: tauri::State<'_, Arc<AppState>>) -> Result<AuthStatus, String> {
    state.auth.status().await
}

#[tauri::command]
async fn begin_google_auth(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<DeviceAuthPrompt, String> {
    state.auth.begin_sign_in(app).await
}

#[tauri::command]
async fn sign_out(state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    *state.library_cache.lock().await = None;
    if let Some(path) = library_cache_path() {
        let _ = std::fs::remove_file(path);
    }
    state.auth.sign_out().await
}

#[tauri::command]
async fn liked_songs(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<LibraryTrack>, String> {
    {
        let cache = state.library_cache.lock().await;
        if let Some(tracks) = cache.as_ref() {
            let tracks = tracks.clone();
            drop(cache);
            spawn_library_refresh(&app, &state);
            return Ok(tracks);
        }
    }
    if let Some(path) = library_cache_path() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(tracks) = serde_json::from_str::<Vec<LibraryTrack>>(&data) {
                if !tracks.is_empty() {
                    *state.library_cache.lock().await = Some(tracks.clone());
                    spawn_library_refresh(&app, &state);
                    return Ok(tracks);
                }
            }
        }
    }
    let tracks = state.auth.liked_songs().await?;
    if !tracks.is_empty() {
        store_library_cache(&state, &tracks);
    }
    Ok(tracks)
}

#[tauri::command]
async fn account_playlists(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<PlaylistSummary>, String> {
    state.auth.playlists().await
}

#[tauri::command]
async fn playlist_items(
    state: tauri::State<'_, Arc<AppState>>,
    playlist_id: String,
) -> Result<Vec<LibraryTrack>, String> {
    state.auth.playlist_items(&playlist_id).await
}

#[tauri::command]
async fn like_track(
    state: tauri::State<'_, Arc<AppState>>,
    video_id: String,
) -> Result<(), String> {
    state.auth.rate_track(&video_id, "like").await
}

#[tauri::command]
async fn unlike_track(
    state: tauri::State<'_, Arc<AppState>>,
    video_id: String,
) -> Result<(), String> {
    state.auth.rate_track(&video_id, "none").await
}

#[tauri::command]
async fn create_playlist(
    state: tauri::State<'_, Arc<AppState>>,
    title: String,
) -> Result<PlaylistSummary, String> {
    state.auth.create_playlist(&title).await
}

#[tauri::command]
async fn search_tracks(
    state: tauri::State<'_, Arc<AppState>>,
    query: String,
) -> Result<Vec<TrackResult>, String> {
    state.innertube.search(&query).await
}

#[tauri::command]
async fn fetch_lyrics(
    state: tauri::State<'_, Arc<AppState>>,
    video_id: String,
    title: String,
    artist: String,
    duration_ms: u64,
) -> Result<Option<LyricsResult>, String> {
    state
        .innertube
        .lyrics(&video_id, &title, &artist, duration_ms)
        .await
}

fn supported_audio_file(path: &PathBuf) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 12];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    header.starts_with(&[0x1A, 0x45, 0xDF, 0xA3])
        || (&header[4..8] == b"ftyp" && &header[8..12] != b"dash")
}

async fn audio_file(app: &AppHandle, state: &AppState, video_id: &str) -> Result<PathBuf, String> {
    let cache_entry = {
        let mut cache = state.audio_cache.lock().await;
        cache
            .entry(video_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone()
    };
    let mut cached_path = cache_entry.lock().await;
    if let Some(path) = cached_path
        .as_ref()
        .filter(|path| path.is_file() && supported_audio_file(path))
    {
        return Ok(path.clone());
    }

    let cache_dir = std::env::temp_dir().join("desktube-audio");
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("failed to create audio cache: {e}"))?;
    let safe_id: String = video_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(*character, '-' | '_'))
        .collect();
    let output_path = cache_dir.join(format!("{safe_id}.mp4"));

    if let Err(direct_error) = state.innertube.download_audio(video_id, &output_path).await {
        let output = app
            .shell()
            .sidecar("yt-dlp")
            .map_err(|e| format!("failed to locate yt-dlp sidecar: {e}"))?
            .args([
                "--no-warnings",
                "--no-playlist",
                "--no-part",
                "-f",
                "18/22/best[ext=mp4][vcodec!=none][acodec!=none]",
                "-o",
                output_path
                    .to_str()
                    .ok_or_else(|| "invalid temporary file path".to_string())?,
                &format!("https://www.youtube.com/watch?v={video_id}"),
            ])
            .output()
            .await
            .map_err(|e| format!("yt-dlp failed to start: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "audio lookup failed: {direct_error}; yt-dlp failed: {}",
                stderr.trim()
            ));
        }
    }
    if !output_path.is_file() {
        return Err("audio download completed without creating a file".to_string());
    }
    if !supported_audio_file(&output_path) {
        let _ = std::fs::remove_file(&output_path);
        return Err("audio download returned an unsupported container".to_string());
    }

    *cached_path = Some(output_path.clone());
    Ok(output_path)
}

#[tauri::command]
async fn prepare_track(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    video_id: String,
) -> Result<(), String> {
    audio_file(&app, &state, &video_id).await.map(|_| ())
}

#[tauri::command]
async fn play_track(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    video_id: String,
    duration_ms: u64,
    title: Option<String>,
    artist: Option<String>,
    thumbnail: Option<String>,
) -> Result<(), String> {
    let sequence = state.play_sequence.fetch_add(1, Ordering::SeqCst) + 1;
    let output_path = audio_file(&app, &state, &video_id).await?;
    if state.play_sequence.load(Ordering::SeqCst) != sequence {
        return Ok(());
    }
    smtc::now_playing(
        title.as_deref().unwrap_or("DeskTube"),
        artist.as_deref().unwrap_or(""),
        thumbnail.as_deref(),
    );
    remote::update_track(
        title.as_deref().unwrap_or("DeskTube"),
        artist.as_deref().unwrap_or(""),
        thumbnail.as_deref(),
    );
    state.player.play_file(output_path, duration_ms)
}

#[tauri::command]
async fn queue_next_track(
    app: AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    video_id: String,
    duration_ms: u64,
    crossfade_ms: u64,
) -> Result<(), String> {
    let output_path = audio_file(&app, &state, &video_id).await?;
    state
        .player
        .queue_next(output_path, duration_ms, crossfade_ms)
}

#[tauri::command]
fn announce_track(title: String, artist: String, thumbnail: Option<String>) -> Result<(), String> {
    smtc::now_playing(&title, &artist, thumbnail.as_deref());
    remote::update_track(&title, &artist, thumbnail.as_deref());
    Ok(())
}

#[tauri::command]
fn remote_pairing() -> Result<Option<serde_json::Value>, String> {
    Ok(remote::pairing())
}

#[tauri::command]
async fn radio_tracks(
    state: tauri::State<'_, Arc<AppState>>,
    video_id: String,
) -> Result<Vec<TrackResult>, String> {
    state.innertube.radio(&video_id).await
}

#[tauri::command]
async fn artist_page(
    state: tauri::State<'_, Arc<AppState>>,
    browse_id: String,
) -> Result<ArtistPage, String> {
    state.innertube.artist_page(&browse_id).await
}

#[tauri::command]
async fn album_page(
    state: tauri::State<'_, Arc<AppState>>,
    browse_id: String,
) -> Result<AlbumPage, String> {
    state.innertube.album_page(&browse_id).await
}

#[tauri::command]
fn audio_devices() -> Result<Vec<String>, String> {
    Ok(player::output_devices())
}

#[tauri::command]
fn set_audio_device(
    state: tauri::State<'_, Arc<AppState>>,
    name: Option<String>,
) -> Result<(), String> {
    state.player.set_device(name)
}

#[tauri::command]
fn pause(state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    state.player.pause();
    Ok(())
}

#[tauri::command]
fn resume(state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    state.player.resume();
    Ok(())
}

#[tauri::command]
fn seek(state: tauri::State<'_, Arc<AppState>>, position_ms: u64) -> Result<(), String> {
    state.player.seek(position_ms)
}

#[tauri::command]
fn set_volume(state: tauri::State<'_, Arc<AppState>>, volume: f32) -> Result<(), String> {
    state.player.set_volume(volume);
    Ok(())
}

/// Emits `playback-state` to the frontend every 500 ms.
fn spawn_position_emitter(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let stats = state.player.stats();
            smtc::update_playback(&stats.status, stats.position_ms, stats.duration_ms);
            taskbar::set_playing(stats.status == "playing");
            remote::update_state(&stats.status, stats.position_ms, stats.duration_ms);
            let _ = app.emit("playback-state", stats);
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let state = Arc::new(AppState {
        auth: AuthManager::new().expect("Google OAuth configuration failed"),
        innertube: Innertube::new(),
        player: Player::new(),
        audio_cache: Mutex::new(HashMap::new()),
        play_sequence: AtomicU64::new(0),
        library_cache: Mutex::new(None),
        library_refreshing: AtomicBool::new(false),
    });
    let emitter_state = state.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    use tauri_plugin_global_shortcut::{Code, Modifiers, ShortcutState};
                    if event.state != ShortcutState::Pressed {
                        return;
                    }
                    let command = if shortcut.matches(Modifiers::empty(), Code::MediaPlayPause) {
                        Some("toggle")
                    } else if shortcut.matches(Modifiers::empty(), Code::MediaTrackNext) {
                        Some("next")
                    } else if shortcut.matches(Modifiers::empty(), Code::MediaTrackPrevious) {
                        Some("previous")
                    } else {
                        None
                    };
                    if let Some(command) = command {
                        let _ = app.emit("media-command", command);
                    }
                })
                .build(),
        )
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            auth_status,
            begin_google_auth,
            sign_out,
            liked_songs,
            account_playlists,
            playlist_items,
            like_track,
            unlike_track,
            create_playlist,
            search_tracks,
            fetch_lyrics,
            prepare_track,
            play_track,
            queue_next_track,
            announce_track,
            remote_pairing,
            radio_tracks,
            artist_page,
            album_page,
            audio_devices,
            set_audio_device,
            pause,
            resume,
            seek,
            set_volume
        ])
        .setup(move |app| {
            // Best-effort media keys: a second instance or another app may
            // already own them — that's not fatal.
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            for key in ["MediaPlayPause", "MediaTrackNext", "MediaTrackPrevious"] {
                let _ = app.global_shortcut().register(
                    key.parse::<tauri_plugin_global_shortcut::Shortcut>()
                        .expect("valid media key"),
                );
            }
            smtc::init(app.handle());
            taskbar::init(app.handle());
            remote::start(app.handle().clone());
            spawn_position_emitter(app.handle().clone(), emitter_state);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running DeskTube");
}
