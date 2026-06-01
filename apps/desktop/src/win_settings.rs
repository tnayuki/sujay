//! Native Win32 settings dialog (mirrors the macOS NSPanel dialog).
//!
//! Uses standard Win32 controls — a `SysTabControl32` with Audio / Recording /
//! OSC pages, native combo boxes, checkboxes and edit fields — so it has the OS
//! look. Fixed window size; switching tabs only shows/hides control groups, the
//! height never changes.
//!
//! Integration model: the dialog is *modeless* (the parent's winit message loop
//! pumps it), but the parent window is disabled while it is open for a modal
//! feel. This avoids running a nested message loop, which would re-enter winit's
//! `ApplicationHandler`. On Save the chosen [`PreferencesState`] is written to a
//! result slot and `APPLIED` is set; the app polls these in `about_to_wait`.

#![allow(clippy::too_many_arguments)]

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;

use sujay_ui::ui_state::PreferencesState;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{CreateFontW, DeleteObject};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, INITCOMMONCONTROLSEX, ICC_TAB_CLASSES, NMHDR, TCIF_TEXT, TCITEMW,
    TCM_GETCURSEL, TCM_INSERTITEMW, TCN_SELCHANGE,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const ID_SAVE: usize = 1;
const ID_CANCEL: usize = 2;
const ID_TAB: usize = 3;

/// Result of the most recent Save (consumed by the app in `about_to_wait`).
static RESULT: Mutex<Option<PreferencesState>> = Mutex::new(None);
static APPLIED: AtomicBool = AtomicBool::new(false);
/// Set by the menu handler; the app opens the dialog from its event loop.
static OPEN_REQUEST: AtomicBool = AtomicBool::new(false);
/// Live dialog HWND (0 when closed), to avoid opening twice.
static DIALOG_HWND: AtomicIsize = AtomicIsize::new(0);

/// Called from the menu's WM_COMMAND handler — requests the app to open the dialog.
pub fn request_open() {
    OPEN_REQUEST.store(true, Ordering::Relaxed);
}

/// True (once) if the menu asked to open the dialog and it is not already open.
pub fn take_open_request() -> bool {
    OPEN_REQUEST.swap(false, Ordering::Relaxed) && DIALOG_HWND.load(Ordering::Relaxed) == 0
}

/// Take the saved preferences if the user pressed Save since the last poll.
pub fn take_result() -> Option<PreferencesState> {
    if APPLIED.swap(false, Ordering::Relaxed) {
        RESULT.lock().unwrap().take()
    } else {
        None
    }
}

struct DialogState {
    parent: HWND,
    tabs: [Vec<HWND>; 3],
    device: HWND,
    chans: [HWND; 4], // main_l, main_r, cue_l, cue_r
    device_names: Vec<String>,
    device_max: Vec<i32>,
    rec_dir: HWND,
    rec_auto: HWND,
    rec_name: HWND,
    rec_fmt: HWND,
    osc_enabled: HWND,
    osc_host: HWND,
    osc_port: HWND,
    font: isize,
    base: PreferencesState,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn set_font(ctrl: HWND, font: isize) {
    SendMessageW(ctrl, WM_SETFONT, font as WPARAM, 1);
}

unsafe fn get_text(ctrl: HWND) -> String {
    let len = GetWindowTextLengthW(ctrl);
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let n = GetWindowTextW(ctrl, buf.as_mut_ptr(), buf.len() as i32);
    String::from_utf16_lossy(&buf[..n as usize])
}

unsafe fn combo_sel(ctrl: HWND) -> i32 {
    SendMessageW(ctrl, CB_GETCURSEL, 0, 0) as i32
}

unsafe fn checked(ctrl: HWND) -> bool {
    // BST_CHECKED == 1
    SendMessageW(ctrl, BM_GETCHECK, 0, 0) == 1
}

unsafe fn mk_label(parent: HWND, font: isize, x: i32, y: i32, w: i32, text: &str) -> HWND {
    let class = wide("STATIC");
    let t = wide(text);
    let h = CreateWindowExW(
        0,
        class.as_ptr(),
        t.as_ptr(),
        WS_CHILD | WS_VISIBLE, // SS_LEFT (0) is the default static alignment
        x, y, w, 18,
        parent,
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );
    set_font(h, font);
    h
}

unsafe fn mk_combo(parent: HWND, font: isize, x: i32, y: i32, w: i32) -> HWND {
    let class = wide("COMBOBOX");
    let empty = wide("");
    let h = CreateWindowExW(
        0,
        class.as_ptr(),
        empty.as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | CBS_DROPDOWNLIST as u32,
        x, y, w, 220,
        parent,
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );
    set_font(h, font);
    h
}

unsafe fn combo_add(ctrl: HWND, text: &str) {
    let t = wide(text);
    SendMessageW(ctrl, CB_ADDSTRING, 0, t.as_ptr() as LPARAM);
}

unsafe fn mk_edit(parent: HWND, font: isize, x: i32, y: i32, w: i32, text: &str) -> HWND {
    let class = wide("EDIT");
    let t = wide(text);
    let h = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        class.as_ptr(),
        t.as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
        x, y, w, 24,
        parent,
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );
    set_font(h, font);
    h
}

unsafe fn mk_checkbox(parent: HWND, font: isize, x: i32, y: i32, w: i32, text: &str, on: bool) -> HWND {
    let class = wide("BUTTON");
    let t = wide(text);
    let h = CreateWindowExW(
        0,
        class.as_ptr(),
        t.as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        x, y, w, 20,
        parent,
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );
    set_font(h, font);
    let check: WPARAM = if on { 1 } else { 0 }; // BST_CHECKED / BST_UNCHECKED
    SendMessageW(h, BM_SETCHECK, check, 0);
    h
}

unsafe fn mk_button(parent: HWND, font: isize, id: usize, x: i32, y: i32, w: i32, text: &str, default: bool) -> HWND {
    let class = wide("BUTTON");
    let t = wide(text);
    let style = WS_CHILD | WS_VISIBLE | WS_TABSTOP
        | if default { BS_DEFPUSHBUTTON } else { BS_PUSHBUTTON } as u32;
    let h = CreateWindowExW(
        0,
        class.as_ptr(),
        t.as_ptr(),
        style,
        x, y, w, 26,
        parent,
        id as _,
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );
    set_font(h, font);
    h
}

unsafe fn show_tab(state: &DialogState, idx: usize) {
    for (i, group) in state.tabs.iter().enumerate() {
        let cmd = if i == idx { SW_SHOW } else { SW_HIDE };
        for &h in group {
            ShowWindow(h, cmd);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_NOTIFY => {
            let nmhdr = lparam as *const NMHDR;
            if !nmhdr.is_null() && (*nmhdr).idFrom == ID_TAB && (*nmhdr).code == TCN_SELCHANGE {
                let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const DialogState;
                if !state.is_null() {
                    let tab = (*nmhdr).hwndFrom;
                    let sel = SendMessageW(tab, TCM_GETCURSEL, 0, 0) as usize;
                    show_tab(&*state, sel.min(2));
                }
            }
            0
        }
        WM_COMMAND => {
            let id = wparam & 0xFFFF;
            let code = (wparam >> 16) & 0xFFFF;
            let ctrl = lparam as HWND;
            // Edit-time behaviour for the Audio tab combos (mirrors macOS):
            //  - device change → rebuild channel lists for the new device, clamp
            //  - channel change → enforce first-wins uniqueness across the 4 slots
            if code == CBN_SELCHANGE as usize && !ctrl.is_null() {
                let sp = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const DialogState;
                if !sp.is_null() {
                    let state = &*sp;
                    if ctrl == state.device {
                        rebuild_channel_combos(state);
                        return 0;
                    } else if state.chans.contains(&ctrl) {
                        enforce_channel_uniqueness(state, ctrl);
                        return 0;
                    }
                }
            }
            if id == ID_SAVE {
                let sp = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const DialogState;
                if !sp.is_null() {
                    let next = read_state(&*sp);
                    *RESULT.lock().unwrap() = Some(next);
                    APPLIED.store(true, Ordering::Relaxed);
                }
                DestroyWindow(hwnd);
                0
            } else if id == ID_CANCEL {
                DestroyWindow(hwnd);
                0
            } else {
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            let sp = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DialogState;
            if !sp.is_null() {
                let state = Box::from_raw(sp);
                EnableWindow(state.parent, 1);
                // Return focus to the parent window.
                windows_sys::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow(state.parent);
                if state.font != 0 {
                    DeleteObject(state.font as _);
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            DIALOG_HWND.store(0, Ordering::Relaxed);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Max channels the currently-selected device exposes (System Default → the
/// largest count across known devices, so the list is generous but bounded).
unsafe fn current_max_channels(state: &DialogState) -> i32 {
    let dev = combo_sel(state.device);
    if dev <= 0 {
        state.device_max.iter().copied().max().unwrap_or(2).max(2)
    } else {
        state.device_max.get((dev - 1) as usize).copied().unwrap_or(2).max(2)
    }
}

/// Device changed: rebuild every channel combo's item list to the new device's
/// channel count, clamping any now-out-of-range selection (macOS: deviceChanged).
unsafe fn rebuild_channel_combos(state: &DialogState) {
    let max_ch = current_max_channels(state);
    for &c in &state.chans {
        let prev = combo_sel(c); // 0 = "-", n = channel n
        SendMessageW(c, CB_RESETCONTENT, 0, 0);
        combo_add(c, "-");
        for ch in 0..max_ch {
            combo_add(c, &format!("{}", ch + 1));
        }
        let count = SendMessageW(c, CB_GETCOUNT, 0, 0) as i32;
        let clamped = if prev >= 0 && prev < count { prev } else { 0 };
        SendMessageW(c, CB_SETCURSEL, clamped as WPARAM, 0);
    }
}

/// Channel changed: clear any other slot that now duplicates this selection
/// (first-wins; macOS: channelChanged). Programmatic CB_SETCURSEL does not emit
/// CBN_SELCHANGE, so this does not recurse.
unsafe fn enforce_channel_uniqueness(state: &DialogState, changed: HWND) {
    let changed_sel = combo_sel(changed);
    if changed_sel <= 0 {
        return; // "-" selected → nothing to de-duplicate
    }
    for &c in &state.chans {
        if c != changed && combo_sel(c) == changed_sel {
            SendMessageW(c, CB_SETCURSEL, 0, 0);
        }
    }
}

unsafe fn read_state(state: &DialogState) -> PreferencesState {
    let dev_sel = combo_sel(state.device);
    let selected_device = if dev_sel <= 0 {
        None
    } else {
        state.device_names.get((dev_sel - 1) as usize).cloned()
    };
    let new_max = if dev_sel <= 0 {
        i32::MAX
    } else {
        state.device_max.get((dev_sel - 1) as usize).copied().unwrap_or(i32::MAX)
    };

    let raw: [Option<i32>; 4] = [
        { let s = combo_sel(state.chans[0]); if s <= 0 { None } else { Some(s - 1) } },
        { let s = combo_sel(state.chans[1]); if s <= 0 { None } else { Some(s - 1) } },
        { let s = combo_sel(state.chans[2]); if s <= 0 { None } else { Some(s - 1) } },
        { let s = combo_sel(state.chans[3]); if s <= 0 { None } else { Some(s - 1) } },
    ];
    let mut seen = HashSet::<i32>::new();
    let resolved: Vec<Option<i32>> = raw
        .iter()
        .map(|ch| match ch {
            Some(v) if *v < new_max && seen.insert(*v) => Some(*v),
            _ => None,
        })
        .collect();

    let mut next = state.base.clone();
    next.audio_device_id = selected_device;
    next.main_channels = [resolved[0], resolved[1]];
    next.cue_channels = [resolved[2], resolved[3]];
    next.recording_directory = get_text(state.rec_dir);
    next.recording_auto_create_directory = checked(state.rec_auto);
    next.recording_naming_strategy = if combo_sel(state.rec_name) == 1 {
        "sequential".to_owned()
    } else {
        "timestamp".to_owned()
    };
    next.recording_format = if combo_sel(state.rec_fmt) == 1 { "ogg".to_owned() } else { "wav".to_owned() };
    next.osc_enabled = checked(state.osc_enabled);
    next.osc_host = get_text(state.osc_host);
    next.osc_port = get_text(state.osc_port)
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .unwrap_or(state.base.osc_port);
    next
}

unsafe fn register_class() {
    static mut REGISTERED: bool = false;
    if REGISTERED {
        return;
    }
    let class = wide("SujaySettingsDialog");
    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: GetModuleHandleW(std::ptr::null()),
        hIcon: std::ptr::null_mut(),
        hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
        hbrBackground: 16 as _, // COLOR_BTNFACE (15) + 1
        lpszMenuName: std::ptr::null(),
        lpszClassName: class.as_ptr(),
    };
    RegisterClassW(&wc);
    REGISTERED = true;
}

/// Open the native settings dialog as a child of `parent`, populated from `current`.
pub unsafe fn open(parent: HWND, current: &PreferencesState) {
    if DIALOG_HWND.load(Ordering::Relaxed) != 0 {
        return;
    }

    let icc = INITCOMMONCONTROLSEX {
        dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
        dwICC: ICC_TAB_CLASSES,
    };
    InitCommonControlsEx(&icc);
    register_class();

    // Native UI font (MS Shell Dlg 2 maps to the system UI face: Tahoma/Segoe).
    // -12 ≈ 9 pt at 96 dpi. CLEARTYPE_QUALITY = 5, DEFAULT_CHARSET = 1.
    let face = wide("MS Shell Dlg 2");
    let font = CreateFontW(-12, 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, face.as_ptr()) as isize;

    let class = wide("SujaySettingsDialog");
    let title = wide("Settings");

    // Desired client size; expand to the full window size so controls fit exactly.
    let (cw, ch) = (496, 466);
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
    let exstyle = WS_EX_DLGMODALFRAME;
    let mut rc = windows_sys::Win32::Foundation::RECT { left: 0, top: 0, right: cw, bottom: ch };
    AdjustWindowRectEx(&mut rc, style, 0, exstyle);
    let win_w = rc.right - rc.left;
    let win_h = rc.bottom - rc.top;

    let hwnd = CreateWindowExW(
        exstyle,
        class.as_ptr(),
        title.as_ptr(),
        style,
        0, 0, win_w, win_h,
        parent,
        std::ptr::null_mut(),
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );
    if hwnd.is_null() {
        return;
    }

    // Center over the parent window.
    let mut pr: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
    GetWindowRect(parent, &mut pr);
    let px = pr.left + ((pr.right - pr.left) - win_w) / 2;
    let py = pr.top + ((pr.bottom - pr.top) - win_h) / 2;
    SetWindowPos(hwnd, std::ptr::null_mut(), px.max(0), py.max(0), 0, 0, SWP_NOSIZE | SWP_NOZORDER);

    // ── Tab control ───────────────────────────────────────────────────────
    let tab_class = wide("SysTabControl32");
    let empty = wide("");
    let tab = CreateWindowExW(
        0,
        tab_class.as_ptr(),
        empty.as_ptr(),
        WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
        8, 8, 480, 404,
        hwnd,
        ID_TAB as _,
        GetModuleHandleW(std::ptr::null()),
        std::ptr::null_mut(),
    );
    set_font(tab, font);
    for (i, label) in ["Audio", "Recording", "OSC"].iter().enumerate() {
        let mut t = wide(label);
        let item = TCITEMW {
            mask: TCIF_TEXT,
            dwState: 0,
            dwStateMask: 0,
            pszText: t.as_mut_ptr(),
            cchTextMax: 0,
            iImage: -1,
            lParam: 0,
        };
        SendMessageW(tab, TCM_INSERTITEMW, i, &item as *const _ as LPARAM);
    }

    // Content origin inside the tab's display area.
    let cx = 24;

    // ── Audio tab ───────────────────────────────────────────────────────────
    let mut tab0 = Vec::new();
    tab0.push(mk_label(hwnd, font, cx, 48, 300, "Audio Device"));
    let device = mk_combo(hwnd, font, cx, 68, 440);
    combo_add(device, "System Default");
    for d in &current.audio_devices {
        combo_add(device, &format!("{} ({} ch)", d.name, d.max_output_channels));
    }
    let mut dev_sel_idx = 0i32;
    if let Some(ref id) = current.audio_device_id {
        if let Some((i, _)) = current.audio_devices.iter().enumerate().find(|(_, d)| &d.name == id) {
            dev_sel_idx = (i + 1) as i32;
        }
    }
    SendMessageW(device, CB_SETCURSEL, dev_sel_idx as WPARAM, 0);
    tab0.push(device);

    let selected_max = current
        .audio_device_id
        .as_ref()
        .and_then(|id| current.audio_devices.iter().find(|d| &d.name == id))
        .map(|d| d.max_output_channels as i32)
        .unwrap_or(2)
        .max(2);

    tab0.push(mk_label(hwnd, font, cx, 104, 300, "Output Routing"));
    let chan_specs = [
        ("Main L", cx, 128, current.main_channels[0]),
        ("Main R", 180, 128, current.main_channels[1]),
        ("Cue L", cx, 196, current.cue_channels[0]),
        ("Cue R", 180, 196, current.cue_channels[1]),
    ];
    let mut chans = [std::ptr::null_mut(); 4];
    for (i, (label, x, y, sel)) in chan_specs.iter().enumerate() {
        tab0.push(mk_label(hwnd, font, *x, *y, 120, label));
        let combo = mk_combo(hwnd, font, *x, *y + 20, 120);
        combo_add(combo, "-");
        for c in 0..selected_max {
            combo_add(combo, &format!("{}", c + 1));
        }
        let idx = sel.map(|v| (v + 1).max(0)).unwrap_or(0);
        SendMessageW(combo, CB_SETCURSEL, idx as WPARAM, 0);
        chans[i] = combo;
        tab0.push(combo);
    }

    // ── Recording tab ─────────────────────────────────────────────────────
    let mut tab1 = Vec::new();
    tab1.push(mk_label(hwnd, font, cx, 48, 300, "Recording Directory"));
    let rec_dir = mk_edit(hwnd, font, cx, 68, 440, &current.recording_directory);
    tab1.push(rec_dir);
    let rec_auto = mk_checkbox(hwnd, font, cx, 104, 400, "Auto-create recording directory", current.recording_auto_create_directory);
    tab1.push(rec_auto);
    tab1.push(mk_label(hwnd, font, cx, 144, 150, "Naming"));
    let rec_name = mk_combo(hwnd, font, 180, 140, 200);
    combo_add(rec_name, "timestamp");
    combo_add(rec_name, "sequential");
    SendMessageW(rec_name, CB_SETCURSEL, if current.recording_naming_strategy == "sequential" { 1 } else { 0 }, 0);
    tab1.push(rec_name);
    tab1.push(mk_label(hwnd, font, cx, 180, 150, "Format"));
    let rec_fmt = mk_combo(hwnd, font, 180, 176, 160);
    combo_add(rec_fmt, "wav");
    combo_add(rec_fmt, "ogg");
    SendMessageW(rec_fmt, CB_SETCURSEL, if current.recording_format == "ogg" { 1 } else { 0 }, 0);
    tab1.push(rec_fmt);

    // ── OSC tab ─────────────────────────────────────────────────────────────
    let mut tab2 = Vec::new();
    let osc_enabled = mk_checkbox(hwnd, font, cx, 48, 400, "Enable OSC", current.osc_enabled);
    tab2.push(osc_enabled);
    tab2.push(mk_label(hwnd, font, cx, 90, 90, "Host"));
    let osc_host = mk_edit(hwnd, font, 120, 86, 220, &current.osc_host);
    tab2.push(osc_host);
    tab2.push(mk_label(hwnd, font, 350, 90, 44, "Port"));
    let osc_port = mk_edit(hwnd, font, 400, 86, 64, &current.osc_port.to_string());
    tab2.push(osc_port);

    // ── Buttons ─────────────────────────────────────────────────────────────
    mk_button(hwnd, font, ID_SAVE, 400, 424, 80, "Save", true);
    mk_button(hwnd, font, ID_CANCEL, 308, 424, 84, "Cancel", false);

    let state = Box::new(DialogState {
        parent,
        tabs: [tab0, tab1, tab2],
        device,
        chans,
        device_names: current.audio_devices.iter().map(|d| d.name.clone()).collect(),
        device_max: current.audio_devices.iter().map(|d| d.max_output_channels as i32).collect(),
        rec_dir,
        rec_auto,
        rec_name,
        rec_fmt,
        osc_enabled,
        osc_host,
        osc_port,
        font,
        base: current.clone(),
    });
    show_tab(&state, 0);
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
    DIALOG_HWND.store(hwnd as isize, Ordering::Relaxed);

    EnableWindow(parent, 0);
    ShowWindow(hwnd, SW_SHOW);
}
