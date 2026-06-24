// Intercept the native minimize button.
//
// On Windows, `WS_MINIMIZE` changes the window's state and causes it to lose
// foreground, which empirically breaks enigo-based `SendInput` remote control
// in this app. Instead of minimizing, we move the window offscreen (to the
// conventional far point (-32000, -32000)) while keeping it in the normal
// "restored" state. The taskbar button is preserved because we never hide or
// minimize the window, so the user can click it again to bring the window back.
//
// Mechanism: replace the window's WNDPROC with `SetWindowLongPtrW(GWLP_WNDPROC)`
// and swallow `WM_SYSCOMMAND` whose wParam low word equals `SC_MINIMIZE`.
// Restoration happens in two complementary paths:
//   1. The window stayed foreground after being hidden offscreen: clicking its
//      taskbar button makes Windows send another SC_MINIMIZE (toggle). Our proc
//      sees HIDDEN=true and restores the position instead.
//   2. The window lost foreground (user activated another app): clicking the
//      taskbar button reactivates the window and Tauri fires `Focused(true)`,
//      which calls `take_restore()` from `lib.rs`.
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM, RECT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GetWindowRect, GWLP_WNDPROC, SC_MINIMIZE, SetWindowLongPtrW, SetWindowPos,
    HWND_TOP, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, WM_SYSCOMMAND, WNDPROC,
};

/// The conventional far point used to park a window offscreen.
const HIDE_X: i32 = -32000;
const HIDE_Y: i32 = -32000;

static INSTALLED: AtomicBool = AtomicBool::new(false);
/// True while the window is parked offscreen waiting to be restored.
static HIDDEN: AtomicBool = AtomicBool::new(false);
/// The saved top-left screen position to return to on restore.
static RESTORE: Lazy<Mutex<Option<(i32, i32)>>> = Lazy::new(|| Mutex::new(None));
/// The original WNDPROC address, so we can delegate all other messages.
static OLD_PROC: AtomicIsize = AtomicIsize::new(0);

/// Install the minimize interceptor on the given top-level window. Idempotent.
pub fn install(hwnd: HWND) {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe {
        // Atomically swap in our proc and remember the previous one. This must
        // be a single call; SetWindowLongPtrW returns the old value directly.
        let old = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, new_proc as *const () as isize);
        OLD_PROC.store(old, Ordering::SeqCst);
    }
    log::info!("win_hide: minimize interceptor installed on hwnd {hwnd:#x}");
}

/// Called from the Tauri `Focused(true)` handler. If the window is currently
/// hidden offscreen, atomically clear the flag and return the position to
/// restore to; otherwise return None.
pub fn take_restore() -> Option<(i32, i32)> {
    if HIDDEN.swap(false, Ordering::SeqCst) {
        RESTORE.lock().ok().and_then(|mut g| g.take())
    } else {
        None
    }
}

/// The replacement window procedure.
unsafe extern "system" fn new_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // The low 12 bits of wParam carry the system command id; mask off the rest.
    if msg == WM_SYSCOMMAND && (wp & 0xFFF0) == SC_MINIMIZE as usize {
        toggle_hide(hwnd);
        // Swallow the command so DefWindowProc never enters WS_MINIMIZE state.
        return 0;
    }
    let old = OLD_PROC.load(Ordering::SeqCst);
    let proc: WNDPROC = std::mem::transmute::<isize, WNDPROC>(old);
    CallWindowProcW(proc, hwnd, msg, wp, lp)
}

/// First SC_MINIMIZE: park offscreen and remember the position. Second
/// SC_MINIMIZE (the taskbar toggle on an already-active window): restore.
fn toggle_hide(hwnd: HWND) {
    if HIDDEN.load(Ordering::SeqCst) {
        // Restore path triggered from the taskbar toggle.
        HIDDEN.store(false, Ordering::SeqCst);
        if let Some((x, y)) = RESTORE.lock().ok().and_then(|mut g| g.take()) {
            unsafe { set_pos(hwnd, x, y) };
        }
    } else {
        // Hide path: capture current position first, then move offscreen.
        if let Some((x, y)) = get_pos(hwnd) {
            if let Ok(mut g) = RESTORE.lock() {
                *g = Some((x, y));
            }
        }
        HIDDEN.store(true, Ordering::SeqCst);
        unsafe { set_pos(hwnd, HIDE_X, HIDE_Y) };
    }
}

/// Read the window's current top-left in physical screen coordinates.
fn get_pos(hwnd: HWND) -> Option<(i32, i32)> {
    unsafe {
        let mut rc: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut rc) != 0 {
            Some((rc.left, rc.top))
        } else {
            None
        }
    }
}

/// Move the window's top-left to (x, y) without touching size, z-order, or
/// focus, so the offscreen shuffle is invisible to the rest of the app.
unsafe fn set_pos(hwnd: HWND, x: i32, y: i32) {
    SetWindowPos(
        hwnd,
        HWND_TOP,
        x,
        y,
        0,
        0,
        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
}
