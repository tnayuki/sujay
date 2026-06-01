//! Native macOS settings dialog (NSPanel + NSTabView).
//!
//! Mirrors the Windows dialog in `win_settings.rs`, but uses AppKit: a real
//! modal `NSPanel` with Audio / Recording / OSC tabs and standard controls.
//! Edit-time channel uniqueness and device-driven channel clamping are handled
//! by ObjC action callbacks on a dynamically-registered handler class.
//!
//! The only entry point used by the app is [`show_native_preferences_dialog`].

use cocoa::base::{id, nil};
use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize, NSString};
use objc::declare::ClassDecl;
use objc::runtime::{Object, Sel};
use objc::{class, msg_send, sel, sel_impl};

use crate::nsstring_to_string;


/// Popup references passed to ObjC action callbacks via thread-local.
struct ChannelPopupContext {
    device_popup: id,
    channel_popups: [id; 4], // main_l, main_r, cue_l, cue_r
    /// max_output_channels per device popup item: index 0 = System Default (i32::MAX)
    device_max_channels: Vec<i32>,
}

std::thread_local! {
    static CHANNEL_CTX: std::cell::Cell<usize> = std::cell::Cell::new(0);
}

fn settings_dialog_class() -> *const objc::runtime::Class {
    use std::sync::Once;
    static INIT: Once = Once::new();
    static mut CLASS: *const objc::runtime::Class = std::ptr::null();
    INIT.call_once(|| unsafe {
        let superclass = class!(NSObject);
        let mut decl = ClassDecl::new("SujaySettingsDialogHandler", superclass).unwrap();
        decl.add_method(
            sel!(saveSettings:),
            settings_dialog_save as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(cancelSettings:),
            settings_dialog_cancel as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(channelChanged:),
            settings_channel_changed as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(deviceChanged:),
            settings_device_changed as extern "C" fn(&Object, Sel, id),
        );
        CLASS = decl.register();
    });
    unsafe { CLASS }
}

extern "C" fn settings_dialog_save(_this: &Object, _cmd: Sel, _sender: id) {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, stopModalWithCode: 1000i64];
    }
}

extern "C" fn settings_dialog_cancel(_this: &Object, _cmd: Sel, _sender: id) {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, stopModalWithCode: 1001i64];
    }
}

/// Edit-time uniqueness: when a channel popup changes, clear other popups that
/// happen to have the same channel selected (first-wins, like the old Electron UI).
extern "C" fn settings_channel_changed(_this: &Object, _cmd: Sel, sender: id) {
    CHANNEL_CTX.with(|cell| {
        let ptr = cell.get();
        if ptr == 0 { return; }
        let ctx = unsafe { &*(ptr as *const ChannelPopupContext) };
        let changed_idx: i64 = unsafe { msg_send![sender, indexOfSelectedItem] };
        if changed_idx <= 0 { return; } // "-" selected – nothing to de-dup
        let changed_val = (changed_idx - 1) as i32;
        for popup in &ctx.channel_popups {
            if *popup != sender {
                let idx: i64 = unsafe { msg_send![*popup, indexOfSelectedItem] };
                if idx > 0 && (idx - 1) as i32 == changed_val {
                    unsafe { let _: () = msg_send![*popup, selectItemAtIndex: 0i64]; }
                }
            }
        }
    });
}

/// Edit-time clamping: when the device popup changes, rebuild channel popup items
/// to only show channels the new device supports, then clamp any out-of-range selection.
extern "C" fn settings_device_changed(_this: &Object, _cmd: Sel, _sender: id) {
    CHANNEL_CTX.with(|cell| {
        let ptr = cell.get();
        if ptr == 0 { return; }
        let ctx = unsafe { &*(ptr as *const ChannelPopupContext) };
        let dev_idx: i64 = unsafe { msg_send![ctx.device_popup, indexOfSelectedItem] };
        let max_ch = ctx.device_max_channels
            .get(dev_idx as usize)
            .copied()
            .unwrap_or(i32::MAX);
        for popup in &ctx.channel_popups {
            let prev_idx: i64 = unsafe { msg_send![*popup, indexOfSelectedItem] };
            // Rebuild items so the list reflects the new device's channel count.
            unsafe {
                let _: () = msg_send![*popup, removeAllItems];
                let _: () = msg_send![*popup, addItemWithTitle: NSString::alloc(nil).init_str("-")];
                for ch in 0..max_ch {
                    let title = format!("{}", ch + 1);
                    let _: () = msg_send![*popup, addItemWithTitle:
                        NSString::alloc(nil).init_str(&title)];
                }
                let count: i64 = msg_send![*popup, numberOfItems];
                let clamped = if prev_idx < count { prev_idx } else { 0 };
                let _: () = msg_send![*popup, selectItemAtIndex: clamped];
            }
        }
    });
}

pub(crate) unsafe fn show_native_preferences_dialog(
    current: &sujay_ui::ui_state::PreferencesState,
) -> Option<sujay_ui::ui_state::PreferencesState> {
    let _pool = NSAutoreleasePool::new(nil);

    // ── NSPanel (no icon area, full layout control) ───────────────────────────
    let style_mask: u64 = 1 | 2; // NSTitledWindowMask | NSClosableWindowMask
    let panel: id = msg_send![class!(NSPanel), alloc];
    let panel: id = msg_send![panel,
        initWithContentRect: NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(480.0, 440.0))
        styleMask: style_mask
        backing: 2u64
        defer: false];
    let _: () = msg_send![panel, setTitle: NSString::alloc(nil).init_str("Settings")];
    let _: () = msg_send![panel, center];
    let content_view: id = msg_send![panel, contentView];

    // ── Tab view (full width, above button bar) ───────────────────────────────
    let tab_view: id = msg_send![class!(NSTabView), alloc];
    let tab_view: id = msg_send![tab_view, initWithFrame:
        NSRect::new(NSPoint::new(0.0, 52.0), NSSize::new(480.0, 388.0))];
    let _: () = msg_send![content_view, addSubview: tab_view];

    // ── Buttons ───────────────────────────────────────────────────────────────
    let btn_handler: id = msg_send![settings_dialog_class(), new];

    let save_btn: id = msg_send![class!(NSButton), alloc];
    let save_btn: id = msg_send![save_btn, initWithFrame:
        NSRect::new(NSPoint::new(384.0, 12.0), NSSize::new(80.0, 28.0))];
    let _: () = msg_send![save_btn, setTitle: NSString::alloc(nil).init_str("Save")];
    let _: () = msg_send![save_btn, setBezelStyle: 1u64];
    let _: () = msg_send![save_btn, setTarget: btn_handler];
    let _: () = msg_send![save_btn, setAction: sel!(saveSettings:)];
    let _: () = msg_send![save_btn, setKeyEquivalent: NSString::alloc(nil).init_str("\r")];
    let _: () = msg_send![content_view, addSubview: save_btn];

    let cancel_btn: id = msg_send![class!(NSButton), alloc];
    let cancel_btn: id = msg_send![cancel_btn, initWithFrame:
        NSRect::new(NSPoint::new(296.0, 12.0), NSSize::new(84.0, 28.0))];
    let _: () = msg_send![cancel_btn, setTitle: NSString::alloc(nil).init_str("Cancel")];
    let _: () = msg_send![cancel_btn, setBezelStyle: 1u64];
    let _: () = msg_send![cancel_btn, setTarget: btn_handler];
    let _: () = msg_send![cancel_btn, setAction: sel!(cancelSettings:)];
    let _: () = msg_send![cancel_btn, setKeyEquivalent: NSString::alloc(nil).init_str("\x1b")];
    let _: () = msg_send![content_view, addSubview: cancel_btn];

    // ── Helper: static non-editable label ────────────────────────────────────
    let mk_label = |x: f64, y: f64, w: f64, text: &str| -> id {
        let f: id = unsafe { msg_send![class!(NSTextField), alloc] };
        let f: id = unsafe { msg_send![f, initWithFrame: NSRect::new(NSPoint::new(x, y), NSSize::new(w, 20.0))] };
        let _: () = unsafe { msg_send![f, setStringValue: NSString::alloc(nil).init_str(text)] };
        let _: () = unsafe { msg_send![f, setEditable: false] };
        let _: () = unsafe { msg_send![f, setBordered: false] };
        let _: () = unsafe { msg_send![f, setDrawsBackground: false] };
        f
    };

    // ── Tab: Audio ────────────────────────────────────────────────────────────
    // Tab content area ≈ 360 px tall (388 tabview − ~28 tab bar).
    // 16 pt padding on all sides; items flow top-to-bottom.
    let audio_tab_item: id = msg_send![class!(NSTabViewItem), alloc];
    let audio_tab_item: id = msg_send![audio_tab_item, initWithIdentifier: nil];
    let _: () = msg_send![audio_tab_item, setLabel: NSString::alloc(nil).init_str("Audio")];
    let audio_tab_view: id = msg_send![class!(NSView), alloc];
    let audio_tab_view: id = msg_send![audio_tab_view, initWithFrame:
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(480.0, 360.0))];

    let dev_label = mk_label(16.0, 324.0, 448.0, "Audio Device");

    let device_popup: id = msg_send![class!(NSPopUpButton), alloc];
    let device_popup: id = msg_send![device_popup, initWithFrame:
        NSRect::new(NSPoint::new(16.0, 294.0), NSSize::new(448.0, 26.0)) pullsDown: false];
    let _: () = msg_send![device_popup, addItemWithTitle: NSString::alloc(nil).init_str("System Default")];
    for dev in &current.audio_devices {
        let title = format!("{} ({} ch)", dev.name, dev.max_output_channels);
        let _: () = msg_send![device_popup, addItemWithTitle: NSString::alloc(nil).init_str(&title)];
    }
    if let Some(ref id) = current.audio_device_id {
        if let Some((idx, _)) = current.audio_devices.iter().enumerate().find(|(_, d)| &d.name == id) {
            let _: () = msg_send![device_popup, selectItemAtIndex: (idx + 1) as i64];
        }
    }

    let selected_max_channels = current
        .audio_device_id
        .as_ref()
        .and_then(|id| current.audio_devices.iter().find(|d| &d.name == id))
        .map(|d| d.max_output_channels as i32)
        .unwrap_or(2)
        .max(2);

    let routing_label = mk_label(16.0, 256.0, 448.0, "Output Routing");

    let mk_channel_popup = |x: f64, y: f64, selected: Option<i32>| {
        let popup: id = unsafe { msg_send![class!(NSPopUpButton), alloc] };
        let popup: id = unsafe { msg_send![popup, initWithFrame: NSRect::new(NSPoint::new(x, y), NSSize::new(110.0, 26.0)) pullsDown: false] };
        let _: () = unsafe { msg_send![popup, addItemWithTitle: NSString::alloc(nil).init_str("-")] };
        for ch in 0..selected_max_channels {
            let title = format!("{}", ch + 1);
            let _: () = unsafe { msg_send![popup, addItemWithTitle: NSString::alloc(nil).init_str(&title)] };
        }
        let idx = selected.map(|v| (v + 1) as i64).unwrap_or(0).max(0);
        let _: () = unsafe { msg_send![popup, selectItemAtIndex: idx] };
        popup
    };

    // Two-column channel routing (L / R), 16 pt left padding, 248 px column gap
    let main_l_label = mk_label(16.0, 228.0, 60.0, "Main L");
    let main_l_popup = mk_channel_popup(16.0, 198.0, current.main_channels[0]);
    let main_r_label = mk_label(248.0, 228.0, 60.0, "Main R");
    let main_r_popup = mk_channel_popup(248.0, 198.0, current.main_channels[1]);
    let cue_l_label  = mk_label(16.0, 160.0, 60.0, "Cue L");
    let cue_l_popup  = mk_channel_popup(16.0, 130.0, current.cue_channels[0]);
    let cue_r_label  = mk_label(248.0, 160.0, 60.0, "Cue R");
    let cue_r_popup  = mk_channel_popup(248.0, 130.0, current.cue_channels[1]);

    // ── Connect edit-time uniqueness + clamping callbacks ─────────────────────
    let ctx = Box::new(ChannelPopupContext {
        device_popup,
        channel_popups: [main_l_popup, main_r_popup, cue_l_popup, cue_r_popup],
        device_max_channels: {
            let mut v = vec![i32::MAX]; // index 0 = System Default
            for dev in &current.audio_devices {
                v.push(dev.max_output_channels as i32);
            }
            v
        },
    });
    let ctx_ptr = Box::into_raw(ctx) as usize;
    CHANNEL_CTX.with(|cell| cell.set(ctx_ptr));

    // Device popup → clamp channel items on change
    let _: () = msg_send![device_popup, setTarget: btn_handler];
    let _: () = msg_send![device_popup, setAction: sel!(deviceChanged:)];
    // Channel popups → enforce uniqueness on change
    for popup in &[main_l_popup, main_r_popup, cue_l_popup, cue_r_popup] {
        let _: () = msg_send![*popup, setTarget: btn_handler];
        let _: () = msg_send![*popup, setAction: sel!(channelChanged:)];
    }

    for sv in &[dev_label, device_popup, routing_label,
                main_l_label, main_l_popup, main_r_label, main_r_popup,
                cue_l_label,  cue_l_popup,  cue_r_label,  cue_r_popup] {
        let _: () = msg_send![audio_tab_view, addSubview: *sv];
    }
    let _: () = msg_send![audio_tab_item, setView: audio_tab_view];
    let _: () = msg_send![tab_view, addTabViewItem: audio_tab_item];

    // ── Tab: Recording ────────────────────────────────────────────────────────
    let recording_tab_item: id = msg_send![class!(NSTabViewItem), alloc];
    let recording_tab_item: id = msg_send![recording_tab_item, initWithIdentifier: nil];
    let _: () = msg_send![recording_tab_item, setLabel: NSString::alloc(nil).init_str("Recording")];
    let recording_tab_view: id = msg_send![class!(NSView), alloc];
    let recording_tab_view: id = msg_send![recording_tab_view, initWithFrame:
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(480.0, 360.0))];

    let rec_dir_label = mk_label(16.0, 324.0, 448.0, "Recording Directory");

    let rec_dir_field: id = msg_send![class!(NSTextField), alloc];
    let rec_dir_field: id = msg_send![rec_dir_field, initWithFrame:
        NSRect::new(NSPoint::new(16.0, 294.0), NSSize::new(448.0, 26.0))];
    let _: () = msg_send![rec_dir_field, setStringValue: NSString::alloc(nil).init_str(&current.recording_directory)];

    let rec_auto_cb: id = msg_send![class!(NSButton), alloc];
    let rec_auto_cb: id = msg_send![rec_auto_cb, initWithFrame:
        NSRect::new(NSPoint::new(16.0, 256.0), NSSize::new(380.0, 20.0))];
    let _: () = msg_send![rec_auto_cb, setButtonType: 3u64];
    let _: () = msg_send![rec_auto_cb, setTitle: NSString::alloc(nil).init_str("Auto-create recording directory")];
    let _: () = msg_send![rec_auto_cb, setState: if current.recording_auto_create_directory { 1i64 } else { 0i64 }];

    let rec_name_label = mk_label(16.0, 224.0, 160.0, "Naming");
    let rec_name_popup: id = msg_send![class!(NSPopUpButton), alloc];
    let rec_name_popup: id = msg_send![rec_name_popup, initWithFrame:
        NSRect::new(NSPoint::new(180.0, 220.0), NSSize::new(284.0, 26.0)) pullsDown: false];
    let _: () = msg_send![rec_name_popup, addItemWithTitle: NSString::alloc(nil).init_str("timestamp")];
    let _: () = msg_send![rec_name_popup, addItemWithTitle: NSString::alloc(nil).init_str("sequential")];
    let rec_name_idx = if current.recording_naming_strategy == "sequential" { 1 } else { 0 };
    let _: () = msg_send![rec_name_popup, selectItemAtIndex: rec_name_idx];

    let rec_fmt_label = mk_label(16.0, 184.0, 160.0, "Format");
    let fmt_popup: id = msg_send![class!(NSPopUpButton), alloc];
    let fmt_popup: id = msg_send![fmt_popup, initWithFrame:
        NSRect::new(NSPoint::new(180.0, 180.0), NSSize::new(180.0, 26.0)) pullsDown: false];
    let _: () = msg_send![fmt_popup, addItemWithTitle: NSString::alloc(nil).init_str("wav")];
    let _: () = msg_send![fmt_popup, addItemWithTitle: NSString::alloc(nil).init_str("ogg")];
    let fmt_idx = if current.recording_format == "ogg" { 1 } else { 0 };
    let _: () = msg_send![fmt_popup, selectItemAtIndex: fmt_idx];

    for sv in &[rec_dir_label, rec_dir_field, rec_auto_cb,
                rec_name_label, rec_name_popup, rec_fmt_label, fmt_popup] {
        let _: () = msg_send![recording_tab_view, addSubview: *sv];
    }
    let _: () = msg_send![recording_tab_item, setView: recording_tab_view];
    let _: () = msg_send![tab_view, addTabViewItem: recording_tab_item];

    // ── Tab: OSC ──────────────────────────────────────────────────────────────
    let osc_tab_item: id = msg_send![class!(NSTabViewItem), alloc];
    let osc_tab_item: id = msg_send![osc_tab_item, initWithIdentifier: nil];
    let _: () = msg_send![osc_tab_item, setLabel: NSString::alloc(nil).init_str("OSC")];
    let osc_tab_view: id = msg_send![class!(NSView), alloc];
    let osc_tab_view: id = msg_send![osc_tab_view, initWithFrame:
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(480.0, 360.0))];

    let osc_enabled_cb: id = msg_send![class!(NSButton), alloc];
    let osc_enabled_cb: id = msg_send![osc_enabled_cb, initWithFrame:
        NSRect::new(NSPoint::new(16.0, 324.0), NSSize::new(380.0, 20.0))];
    let _: () = msg_send![osc_enabled_cb, setButtonType: 3u64];
    let _: () = msg_send![osc_enabled_cb, setTitle: NSString::alloc(nil).init_str("Enable OSC")];
    let _: () = msg_send![osc_enabled_cb, setState: if current.osc_enabled { 1i64 } else { 0i64 }];

    let osc_host_label = mk_label(16.0, 286.0, 100.0, "Host");
    let osc_host_field: id = msg_send![class!(NSTextField), alloc];
    let osc_host_field: id = msg_send![osc_host_field, initWithFrame:
        NSRect::new(NSPoint::new(120.0, 282.0), NSSize::new(228.0, 26.0))];
    let _: () = msg_send![osc_host_field, setStringValue: NSString::alloc(nil).init_str(&current.osc_host)];

    let osc_port_label = mk_label(356.0, 286.0, 44.0, "Port");
    let osc_port_field: id = msg_send![class!(NSTextField), alloc];
    let osc_port_field: id = msg_send![osc_port_field, initWithFrame:
        NSRect::new(NSPoint::new(404.0, 282.0), NSSize::new(60.0, 26.0))];
    let _: () = msg_send![osc_port_field, setStringValue: NSString::alloc(nil).init_str(&current.osc_port.to_string())];

    for sv in &[osc_enabled_cb, osc_host_label, osc_host_field, osc_port_label, osc_port_field] {
        let _: () = msg_send![osc_tab_view, addSubview: *sv];
    }
    let _: () = msg_send![osc_tab_item, setView: osc_tab_view];
    let _: () = msg_send![tab_view, addTabViewItem: osc_tab_item];

    // ── Run modal ─────────────────────────────────────────────────────────────
    let app: id = msg_send![class!(NSApplication), sharedApplication];
    let code: i64 = msg_send![app, runModalForWindow: panel];
    let _: () = msg_send![panel, orderOut: nil];
    // Free context and clear thread-local regardless of result.
    CHANNEL_CTX.with(|cell| {
        let ptr = cell.get();
        if ptr != 0 {
            drop(Box::from_raw(ptr as *mut ChannelPopupContext));
            cell.set(0);
        }
    });
    if code != 1000 {
        return None;
    }

    let selected_idx: i64 = msg_send![device_popup, indexOfSelectedItem];
    let selected_device = if selected_idx <= 0 {
        None
    } else {
        current.audio_devices.get((selected_idx - 1) as usize).map(|d| d.name.clone())
    };

    // Max channels for the newly-selected device (used for clamping).
    let new_max_channels = selected_device
        .as_ref()
        .and_then(|name| current.audio_devices.iter().find(|d| &d.name == name))
        .map(|d| d.max_output_channels as i32)
        .unwrap_or(i32::MAX);

    let read_channel = |popup: id| -> Option<i32> {
        let idx: i64 = unsafe { msg_send![popup, indexOfSelectedItem] };
        if idx <= 0 { None } else { Some((idx - 1) as i32) }
    };

    // Read all 4 slots, then apply clamping + uniqueness (first-wins order:
    // Main L → Main R → Cue L → Cue R, same as the original Electron UI).
    let raw: [Option<i32>; 4] = [
        read_channel(main_l_popup),
        read_channel(main_r_popup),
        read_channel(cue_l_popup),
        read_channel(cue_r_popup),
    ];
    let mut seen = std::collections::HashSet::<i32>::new();
    let resolved: Vec<Option<i32>> = raw.iter().map(|ch| match ch {
        Some(v) if *v < new_max_channels && seen.insert(*v) => Some(*v),
        _ => None,
    }).collect();
    let main_channels = [resolved[0], resolved[1]];
    let cue_channels  = [resolved[2], resolved[3]];

    let dir_value: id = msg_send![rec_dir_field, stringValue];
    let rec_auto_state: i64 = msg_send![rec_auto_cb, state];
    let rec_name_idx: i64 = msg_send![rec_name_popup, indexOfSelectedItem];
    let fmt_selected: i64 = msg_send![fmt_popup, indexOfSelectedItem];
    let osc_enabled_state: i64 = msg_send![osc_enabled_cb, state];
    let osc_host_value: id = msg_send![osc_host_field, stringValue];
    let osc_port_value: id = msg_send![osc_port_field, stringValue];

    let mut next = current.clone();
    next.audio_device_id = selected_device;
    next.main_channels = main_channels;
    next.cue_channels = cue_channels;
    next.recording_directory = nsstring_to_string(dir_value);
    next.recording_auto_create_directory = rec_auto_state != 0;
    next.recording_naming_strategy = if rec_name_idx == 1 {
        "sequential".to_owned()
    } else {
        "timestamp".to_owned()
    };
    next.recording_format = if fmt_selected == 1 { "ogg".to_owned() } else { "wav".to_owned() };
    next.osc_enabled = osc_enabled_state != 0;
    next.osc_host = nsstring_to_string(osc_host_value);
    next.osc_port = nsstring_to_string(osc_port_value)
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .unwrap_or(current.osc_port);
    Some(next)
}
