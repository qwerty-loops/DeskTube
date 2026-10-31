//! Windows taskbar thumbnail toolbar (prev / play-pause / next).
//!
//! Subclasses the main window to answer `TaskbarButtonCreated` and
//! `THBN_CLICKED` notifications, and draws monochrome glyph icons with GDI
//! so no image assets are needed. All calls are best-effort.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Emitter, Manager};
use windows::core::GUID;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList4, SetWindowSubclass, THBF_ENABLED, THB_FLAGS, THB_ICON,
    THB_TOOLTIP, THUMBBUTTON,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateIcon, RegisterWindowMessageW, HICON, WM_COMMAND,
};

const CLSID_TASKBAR_LIST: GUID = GUID::from_u128(0x56fdf344_fd6d_11d0_958a_006097c9a090);
const SUBCLASS_ID: usize = 0xDE57;
const THBN_CLICKED: usize = 0x1800;
const BTN_PREVIOUS: u32 = 200;
const BTN_PLAY_PAUSE: u32 = 201;
const BTN_NEXT: u32 = 202;

/// COM interfaces aren't marked Send, but this is only used to hold the
/// taskbar list for later `ThumbBarUpdateButtons` calls.
struct SendTaskbar(ITaskbarList4);
unsafe impl Send for SendTaskbar {}
unsafe impl Sync for SendTaskbar {}

static TASKBAR_CREATED_MSG: AtomicU32 = AtomicU32::new(0);
static MAIN_HWND: AtomicU32 = AtomicU32::new(0);
static TASKBAR: OnceLock<Mutex<Option<SendTaskbar>>> = OnceLock::new();
static PLAYING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Copy)]
enum Glyph {
    Previous,
    Play,
    Pause,
    Next,
}

/// Builds a 16x16 monochrome icon: white glyph on transparent background.
fn glyph_icon(glyph: Glyph) -> Option<HICON> {
    let filled = |x: i32, y: i32| -> bool {
        // Simple polygons in a 16x16 grid.
        let in_rect = |x0: i32, y0: i32, x1: i32, y1: i32| x >= x0 && x <= x1 && y >= y0 && y <= y1;
        let in_triangle = |ax: i32, ay: i32, bx: i32, by: i32, cx: i32, cy: i32| {
            let sign = |px: i32, py: i32, qx: i32, qy: i32, rx: i32, ry: i32| {
                (px - rx) * (qy - ry) - (qx - rx) * (py - ry)
            };
            let d1 = sign(x, y, ax, ay, bx, by);
            let d2 = sign(x, y, bx, by, cx, cy);
            let d3 = sign(x, y, cx, cy, ax, ay);
            let neg = d1 < 0 || d2 < 0 || d3 < 0;
            let pos = d1 > 0 || d2 > 0 || d3 > 0;
            !(neg && pos)
        };
        match glyph {
            Glyph::Previous => in_rect(3, 3, 5, 13) || in_triangle(13, 3, 13, 13, 6, 8),
            Glyph::Next => in_rect(11, 3, 13, 13) || in_triangle(3, 3, 3, 13, 10, 8),
            Glyph::Play => in_triangle(5, 3, 5, 13, 12, 8),
            Glyph::Pause => in_rect(4, 3, 6, 13) || in_rect(10, 3, 12, 13),
        }
    };

    let mut and_bits = [0_u8; 16 * 4];
    let mut xor_bits = [0_u8; 16 * 4];
    for y in 0..16_i32 {
        for x in 0..16_i32 {
            let byte = (y as usize) * 4 + (x as usize) / 8;
            let bit = 7 - (x as usize) % 8;
            if filled(x, y) {
                xor_bits[byte] |= 1 << bit;
            } else {
                and_bits[byte] |= 1 << bit;
            }
        }
    }
    unsafe { CreateIcon(None, 16, 16, 1, 1, and_bits.as_ptr(), xor_bits.as_ptr()) }.ok()
}

fn tooltip(text: &str) -> [u16; 260] {
    let mut tip = [0_u16; 260];
    for (slot, unit) in tip.iter_mut().zip(text.encode_utf16().take(259)) {
        *slot = unit;
    }
    tip
}

fn taskbar_cell() -> &'static Mutex<Option<SendTaskbar>> {
    TASKBAR.get_or_init(|| Mutex::new(None))
}

fn with_taskbar(f: impl FnOnce(&ITaskbarList4)) {
    if let Ok(slot) = taskbar_cell().lock() {
        if let Some(list) = slot.as_ref() {
            f(&list.0);
        }
    }
}

unsafe fn add_buttons(hwnd: HWND) {
    let list: windows::core::Result<ITaskbarList4> =
        CoCreateInstance(&CLSID_TASKBAR_LIST, None, CLSCTX_ALL);
    let Ok(list) = list else { return };
    let _ = list.HrInit();
    let playing = PLAYING.load(Ordering::SeqCst);
    let buttons = [
        THUMBBUTTON {
            dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
            iId: BTN_PREVIOUS,
            iBitmap: 0,
            hIcon: glyph_icon(Glyph::Previous).unwrap_or_default(),
            szTip: tooltip("Previous"),
            dwFlags: THBF_ENABLED,
        },
        THUMBBUTTON {
            dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
            iId: BTN_PLAY_PAUSE,
            iBitmap: 0,
            hIcon: glyph_icon(if playing { Glyph::Pause } else { Glyph::Play }).unwrap_or_default(),
            szTip: tooltip("Play / Pause"),
            dwFlags: THBF_ENABLED,
        },
        THUMBBUTTON {
            dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
            iId: BTN_NEXT,
            iBitmap: 0,
            hIcon: glyph_icon(Glyph::Next).unwrap_or_default(),
            szTip: tooltip("Next"),
            dwFlags: THBF_ENABLED,
        },
    ];
    if list.ThumbBarAddButtons(hwnd, &buttons).is_ok() {
        if let Ok(mut slot) = taskbar_cell().lock() {
            *slot = Some(SendTaskbar(list));
        }
    }
}

/// Swaps the middle button's glyph between play and pause.
pub fn set_playing(playing: bool) {
    if PLAYING.swap(playing, Ordering::SeqCst) == playing {
        return;
    }
    let hwnd = HWND(MAIN_HWND.load(Ordering::SeqCst) as isize);
    if hwnd.0 == 0 {
        return;
    }
    with_taskbar(|list| unsafe {
        let button = THUMBBUTTON {
            dwMask: THB_ICON | THB_FLAGS,
            iId: BTN_PLAY_PAUSE,
            iBitmap: 0,
            hIcon: glyph_icon(if playing { Glyph::Pause } else { Glyph::Play }).unwrap_or_default(),
            szTip: [0; 260],
            dwFlags: THBF_ENABLED,
        };
        let _ = list.ThumbBarUpdateButtons(hwnd, &[button]);
    });
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _uid: usize,
    refdata: usize,
) -> LRESULT {
    if msg == TASKBAR_CREATED_MSG.load(Ordering::SeqCst) {
        add_buttons(hwnd);
    } else if msg == WM_COMMAND && (wparam.0 >> 16) & 0xFFFF == THBN_CLICKED {
        let app = &*(refdata as *const AppHandle);
        let command = match wparam.0 as u32 & 0xFFFF {
            BTN_PREVIOUS => Some("previous"),
            BTN_PLAY_PAUSE => Some("toggle"),
            BTN_NEXT => Some("next"),
            _ => None,
        };
        if let Some(command) = command {
            let _ = app.emit("media-command", command);
        }
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// Subclasses the main window and registers the thumbnail toolbar buttons.
pub fn init(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let hwnd = HWND(window.hwnd().map(|h| h.0 as isize).unwrap_or(0));
    if hwnd.0 == 0 {
        return;
    }
    MAIN_HWND.store(hwnd.0 as u32, Ordering::SeqCst);
    let msg = unsafe { RegisterWindowMessageW(windows::core::w!("TaskbarButtonCreated")) };
    TASKBAR_CREATED_MSG.store(msg, Ordering::SeqCst);
    let app_ref = Box::leak(Box::new(app.clone())) as *mut AppHandle as usize;
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, app_ref);
        // The taskbar may already exist (e.g. after an explorer restart).
        add_buttons(hwnd);
    }
}
