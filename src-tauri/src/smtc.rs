//! Windows System Media Transport Controls integration.
//!
//! Publishes playback state to the OS so the media overlay, volume flyout,
//! lock screen, and hardware media buttons see and control DeskTube.
//! All calls are best-effort: SMTC failure must never break playback.

use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Emitter, Manager};
use windows::core::HSTRING;
use windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, SystemMediaTransportControls,
    SystemMediaTransportControlsButton, SystemMediaTransportControlsButtonPressedEventArgs,
    SystemMediaTransportControlsTimelineProperties,
};
use windows::Storage::Streams::RandomAccessStreamReference;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

struct Smtc {
    controls: SystemMediaTransportControls,
}

static SMTC: OnceLock<Mutex<Option<Smtc>>> = OnceLock::new();
static LAST_STATUS: AtomicI32 = AtomicI32::new(i32::MIN);
static LAST_POSITION: AtomicU64 = AtomicU64::new(0);

fn cell() -> &'static Mutex<Option<Smtc>> {
    SMTC.get_or_init(|| Mutex::new(None))
}

/// Initializes the SMTC session for the main window and routes hardware media
/// buttons into the app's `media-command` events.
pub fn init(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    // AUMID controls the app name shown in the media flyout/lock screen.
    let _ = unsafe { SetCurrentProcessExplicitAppUserModelID(windows::core::w!("DeskTube")) };
    let hwnd = HWND(window.hwnd().map(|handle| handle.0 as isize).unwrap_or(0));
    let app = app.clone();
    std::thread::spawn(move || {
        let controls: windows::core::Result<SystemMediaTransportControls> =
            windows::core::factory::<
                SystemMediaTransportControls,
                ISystemMediaTransportControlsInterop,
            >()
            .and_then(|interop| unsafe { interop.GetForWindow(hwnd) });
        let Ok(controls) = controls else {
            return;
        };
        let _ = controls.SetIsEnabled(true);
        let _ = controls.SetIsPlayEnabled(true);
        let _ = controls.SetIsPauseEnabled(true);
        let _ = controls.SetIsNextEnabled(true);
        let _ = controls.SetIsPreviousEnabled(true);
        let _ = controls.SetPlaybackStatus(MediaPlaybackStatus::Stopped);

        let _ = controls.ButtonPressed(&TypedEventHandler::new(
            move |_: &Option<SystemMediaTransportControls>,
                  args: &Option<SystemMediaTransportControlsButtonPressedEventArgs>| {
                let command = args.as_ref().and_then(|args| args.Button().ok()).and_then(
                    |button| match button {
                        SystemMediaTransportControlsButton::Play
                        | SystemMediaTransportControlsButton::Pause => Some("toggle"),
                        SystemMediaTransportControlsButton::Next => Some("next"),
                        SystemMediaTransportControlsButton::Previous => Some("previous"),
                        _ => None,
                    },
                );
                if let Some(command) = command {
                    let _ = app.emit("media-command", command);
                }
                Ok(())
            },
        ));

        if let Ok(mut slot) = cell().lock() {
            *slot = Some(Smtc { controls });
        }
    });
}

fn with_controls(f: impl FnOnce(&SystemMediaTransportControls)) {
    if let Ok(slot) = cell().lock() {
        if let Some(smtc) = slot.as_ref() {
            f(&smtc.controls);
        }
    }
}

/// Publishes the current track's metadata and artwork.
pub fn now_playing(title: &str, artist: &str, thumbnail: Option<&str>) {
    let title = title.to_string();
    let artist = artist.to_string();
    let thumbnail = thumbnail.map(String::from);
    with_controls(move |controls| {
        let Ok(updater) = controls.DisplayUpdater() else {
            return;
        };
        let _ = updater.SetType(MediaPlaybackType::Music);
        if let Ok(music) = updater.MusicProperties() {
            let _ = music.SetTitle(&HSTRING::from(&title));
            let _ = music.SetArtist(&HSTRING::from(&artist));
            let _ = music.SetAlbumArtist(&HSTRING::from(&artist));
        }
        if let Some(url) = thumbnail {
            if let Ok(reference) = Uri::CreateUri(&HSTRING::from(&url))
                .and_then(|uri| RandomAccessStreamReference::CreateFromUri(&uri))
            {
                let _ = updater.SetThumbnail(&reference);
            }
        }
        let _ = updater.Update();
    });
}

/// Publishes playback status and position so the OS shows the right buttons.
pub fn update_playback(status: &str, position_ms: u64, duration_ms: u64) {
    let playback_status = match status {
        "playing" => MediaPlaybackStatus::Playing,
        "paused" => MediaPlaybackStatus::Paused,
        _ => MediaPlaybackStatus::Stopped,
    };
    let status_changed = LAST_STATUS.swap(playback_status.0, Ordering::SeqCst) != playback_status.0;
    // Only refresh the timeline when it drifts or status changes; rewriting it
    // every 500 ms would make the OS progress bar jitter.
    if !status_changed
        && LAST_POSITION
            .swap(position_ms, Ordering::SeqCst)
            .abs_diff(position_ms)
            < 2_000
    {
        return;
    }
    with_controls(move |controls| {
        let _ = controls.SetPlaybackStatus(playback_status);
        if let Ok(timeline) = SystemMediaTransportControlsTimelineProperties::new() {
            let _ = timeline.SetStartTime(TimeSpan::default());
            let _ = timeline.SetEndTime(TimeSpan {
                Duration: duration_ms as i64 * 10_000,
            });
            let _ = timeline.SetMinSeekTime(TimeSpan::default());
            let _ = timeline.SetMaxSeekTime(TimeSpan {
                Duration: duration_ms as i64 * 10_000,
            });
            let _ = timeline.SetPosition(TimeSpan {
                Duration: position_ms as i64 * 10_000,
            });
            let _ = controls.UpdateTimelineProperties(&timeline);
        }
    });
}
