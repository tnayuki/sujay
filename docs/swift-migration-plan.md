# Swift-first macOS migration plan (issue #32)

Sujay becomes a Swift-first macOS application. The Rust audio engine stays; the UI layer,
the host binary, the window, the menus and the settings dialog move to Swift/SwiftUI, and
Windows support ends.

This is the plan document for phase 1. It records what is decided and why. The README
describes only what currently works.

---

## Decisions

**Rust is going away, in stages.** The first plan kept the audio engine and the rekordbox reader
in Rust for good and moved only the host and UI. That is superseded: the end state is a Swift-only
app, reached one layer at a time so each step ships and runs. Stage 1 moves `crates/core` and
decoding to Swift (AVFoundation decodes everything symphonia did, on this platform, for free), so
the C ABI shrinks to the engine and the rekordbox reader. Stage 2 moves the rekordbox reader
(SQLCipher-backed `master.db`, ANLZ). Stage 3 replaces the engine with AVAudioEngine —
`AVAudioUnitTimePitch` is the pitch-preserving time stretch, `AVAudioUnitEQ` the kills, mixer nodes
the crossfader and gain, a tap the recorder — and the Rust workspace is deleted. The earlier
decisions below still describe stage 0 and 1 accurately where they talk about the boundary; where
they argue for keeping something in Rust, this paragraph wins.

**The UI is ported in one move, not hosted.** `sujay_decks::attach_raw` takes an `NSView`
pointer, so a Swift shell could embed the existing egui console immediately and replace it
piecewise. That path is rejected: it keeps two renderers, two input paths and two style
systems alive across several releases, and issue #32's acceptance criterion is that the
primary UI is no longer driven by the Rust UI layer. The port lands as one console.

**Windows is dropped.** `apps/desktop/src/win_settings.rs`, `crates/decks/src/renderer_windows.rs`
and every `cfg(target_os = "windows")` branch go away with `crates/decks`. macOS-only is what
"Swift-first macOS application" means, and keeping a second host is what made the current
`main.rs` a 1959-line mix of orchestration and platform glue.

**The rekordbox import splits: acquisition stays in Rust, browsing moves to Swift.** What
`crates/library` actually does is two things. Opening the encrypted `master.db` and parsing the
ANLZ binaries (beat grid, hot cues, 3-band waveform colours) is what the `rbox` crate provides,
and rewriting it in Swift buys nothing; the beat grid and cues are analysis the engine consumes —
beat loops already, slices next — not data the UI merely displays. Browsing is the other thing,
and it is UI: the track list, the playlist tree, sorting, column formatting, load-to-deck. See
"Library".

**`sujay.xcodeproj` is the only build system for the app**, as in `../hukan`: hand-authored
and tracked, edited directly, file-system-synchronized folder groups so a new file under
`Sources/` just appears. No `Package.swift` — the same measurement hukan made applies here,
and a mixed SwiftPM/cargo graph buys nothing.

**The Rust core is built by the Xcode build, not committed as an `xcframework`.** This is a
deliberate departure from hukan's vendoring rule. `Clibgit2.xcframework` is committed because
libgit2 changes when someone bumps a version; `crates/audio` changes in the same commits as
the Swift that drives it, so a committed 20 MB binary would be rebuilt and re-committed
constantly and would make every Rust change invisible in review. A Run Script phase runs
`cargo build` and links the resulting static archive.

**State crosses the boundary at three rates, not one.** A single JSON snapshot per frame is
the obvious design and the wrong one — the console needs deck positions and peak levels at
display rate, where per-frame allocation and decoding is pure waste, while track titles and
the library list change a few times a minute. See "FFI shape".

---

## Target layout

```
sujay/
├── sujay.xcodeproj              # only build system for the app; hand-authored
├── Sources/Sujay/               # SwiftUI app (FS-synchronized group)
├── Sources/SujayCore/           # Swift wrapper over the C ABI
├── Resources/Info.plist         # LSMinimumSystemVersion 15.0
├── Resources/sujay.icns
├── Vendor/build-rust.sh         # cargo + cbindgen -> .build/rust/{libsujay_ffi.a,sujay.h}
├── Cargo.toml                   # workspace root
├── crates/audio/                # unchanged: engine_core, engine_backend, decoder, recorder
├── crates/library/              # unchanged: rekordbox master.db + ANLZ
├── crates/core/                 # NEW: host orchestration, no windowing, no UI
├── crates/ffi/                  # NEW: staticlib, C ABI over crates/core
├── .swift-format                # { "version": 1 }
└── .githooks/pre-commit         # blocks on xcrun swift-format lint --strict
```

`apps/desktop/` and `crates/decks/` were deleted once the Swift console had run for a while (#35).

---

## Rust side

### `crates/core` — what `main.rs` was, minus the platform

Everything in `apps/desktop/src/main.rs` that is not winit, AppKit or Win32 moves here, with
no behaviour change:

| Moves from `main.rs` | Notes |
| --- | --- |
| `AppPreferences`, load/save/normalize, `settings_file_path` | JSON in userData, unchanged format |
| `prepare_recording_path`, `recording_extension` | timestamp naming, auto-create directory |
| `spawn_decode`, `decode_track_full`, `DecodeResult` | background decode thread + channel |
| `spawn_rekordbox_library_load`, reload polling on `master.db` mtime | including the fast/full two-stage load |
| `rekordbox_track_overrides` / `_ids` / `normalize_track_path` | path-keyed metadata join |
| `rekordbox_cues_to_visuals`, `engine_state_to_console_visual` | snapshot assembly |
| `UiAction` dispatch, including the beat-grid loop maths in `ToggleLoop` | the beat grid moves out of `DECK_VISUALS` into core, where it belongs |
| titlebar sampling (`sysinfo`, clock, recording elapsed) | 1 Hz, as today |

The beat grid currently lives in the UI crate (`sujay_decks::get_deck_beat_info_raw` reads
`DECK_VISUALS`) and `main.rs` reaches back into the renderer to compute a beat loop. That
inversion does not survive: the grid is track analysis, it belongs to core, and the UI reads it.

`crates/core` exposes a single `Core` type with command methods (one per current `UiAction`),
a `snapshot()` for the fast fields, and versioned accessors for the slow ones. No `cfg` on
platform, no globals — the `static mut` singletons in `console_ui.rs` are not carried over.

### `crates/ffi` — C ABI staticlib

`crate-type = ["staticlib"]`, header generated by `cbindgen`. `Vendor/build-rust.sh` produces
`.build/rust/libsujay_ffi.a` (arm64 + x86_64 via `lipo`) and `.build/rust/include/sujay.h` with
a module map, and the Xcode target points `SWIFT_INCLUDE_PATHS` and `LIBRARY_SEARCH_PATHS` at it.

### FFI shape

Three rates, three mechanisms:

1. **Commands — plain C functions.** `sujay_play(core, deck)`, `sujay_set_crossfader(core, v)`,
   `sujay_set_eq(core, deck, band, kill)`, `sujay_load_file(core, deck, path)`, … One function
   per action, no serialization, errors as an `int32` status.

2. **Fast state — one POD struct, memcpy per frame.** Deck position/total/sample-rate, peak and
   peak-hold, playing/cue/EQ/loop flags, gain, crossfader, master tempo, mic peak, recording
   elapsed. No strings, no pointers, no allocation: `sujay_snapshot(core, &out)` at display rate.

3. **Slow state — versioned JSON.** Track titles, BPM text, rekordbox cues, the library track and
   playlist lists, preferences, the audio device list. Each carries a generation counter;
   `sujay_state_json(core, generation)` returns `NULL` when nothing changed, so Swift decodes only
   on an actual change.

4. **Bulk buffers — copy-out with a version stamp.** Waveform samples, rekordbox waveform colours,
   the beat grid. `sujay_waveform_version(core, deck)` then `sujay_copy_waveform(core, deck, buf, cap)`;
   Swift keeps its own buffer and re-copies only when the version moves. No borrowed pointers
   across the boundary — the decode thread can replace a track's buffer at any time.

Swift never sees a Rust pointer it must free except the JSON string, which has one
`sujay_string_free`.

---

## Swift side

`Sources/SujayCore/` wraps the C header in a Swift API (`Core` actor-free class, `Snapshot`
struct mirroring the POD, `Codable` types for the JSON). `Sources/Sujay/` is the UI.

The console port maps one-to-one from `crates/decks/src/console_ui.rs` (3288 lines):

| egui function | Swift |
| --- | --- |
| `draw_titlebar` | `TitlebarView` — clock, CPU/mem, MIC pill, REC pill, separators. Window is `.titlebarAppearsTransparent` with a custom bar, as today |
| `draw_deck`, `draw_deck_header`, `draw_deck_info_text`, `draw_deck_number`, `draw_thumbnail` | `DeckView` + `DeckHeaderView` |
| `draw_zoom_waveform`, `draw_full_waveform` | `WaveformView` — see "Waveform rendering" |
| `draw_level_meter`, `draw_volume_slider`, `draw_meters_section` | `LevelMeterView`, `DeckGainSlider` |
| `draw_eq_kills_column`, `draw_eq_kill_button` | `EQKillColumn` |
| `draw_cue_button`, `draw_play_stop_button` | `TransportButtons` |
| `draw_loop_buttons`, `draw_loop_button` | `LoopPadRow` — BEAT LOOP / SLIP LOOP groups |
| `draw_rekordbox_cue_buttons` | `CuePointRow` |
| `draw_tempo_section`, `draw_tempo_arrow`, `draw_tempo_display` | `TempoView` |
| `draw_crossfader` | `CrossfaderView` |
| `draw_library_panel`, `draw_sort_header`, `compare_library_tracks`, `LibrarySortState`, the `format_*` helpers, `truncate_for_column` | `LibraryView` — a `Table` with sortable columns. All of this is hand-rolled in egui and is standard in SwiftUI; it deletes rather than ports. See "Library" |
| `draw_preferences_modal` + `apps/desktop/src/mac_settings.rs` (NSPanel/NSTabView, 503 lines) | `SettingsScene` — `Settings { TabView }`, standard ⌘, window. The hand-built NSPanel and its ObjC target/action channel-uniqueness code both go |
| `setup_fonts`, `setup_style`, `paint_gradient_*`, `paint_glow` | a `Theme` enum + SwiftUI gradients/shadows |
| `MouseEvent` plumbing, `push_mouse_event_raw`, hit-testing in `main.rs` | deleted — SwiftUI owns input |

Drag-and-drop onto a deck becomes `.dropDestination(for: URL.self)`; the deck under the cursor
comes from the drop target, not from the `hovered_deck` hit-test in `main.rs`. The library
right-click "load to deck A/B" menu becomes `.contextMenu`.

### Waveform rendering

The zoom view redraws every frame at display rate with a few thousand segments, and the full
view redraws on seek. Start with SwiftUI `Canvas` for both and measure against the current
wgpu shader path; if the zoom view does not hold 60 fps, it moves to a `CAMetalLayer`-backed
`NSViewRepresentable` reusing `waveform.wgsl`'s logic. Only the zoom view would move — the
full view is not hot. The decision is deferred to measurement, not taken now.

### Frame loop

One `CADisplayLink` on the window's screen drives: `sujay_snapshot` → fast state into an
`@Observable` model → SwiftUI invalidates what changed. Slow state is polled on the same tick
by generation counter, which costs one integer compare when nothing changed.

---

## Library

`crates/library` keeps doing the part that is hard and already solved: `rbox` opens the encrypted
`master.db` and parses the ANLZ binaries. Swift owns the browse model.

The boundary the code already has is the one to use:

- **`load_from_master_db_fast`** — every track's display metadata, no ANLZ. This is the browse
  list: one generation-gated JSON decode per library load or reload, not per frame. At the current
  `master.db` size a single decode is the whole cost, which is why there is no intermediate SQLite
  cache for Swift to read — a second schema to keep in sync buys nothing at this scale.
- **`load_track_analysis_from_master_db`** — one track's beat grid, cues and waveform colours,
  read at deck-load time and delivered over the bulk-buffer path.

**The Swift browse model is source-agnostic.** Rekordbox is one provider, reached through the FFI.
Local-folder libraries — decided in principle but not built — are a second provider, and folder
scanning belongs in Swift (`FileManager`, AVFoundation metadata) rather than behind the FFI. The
track list type Swift renders must therefore not be "a rekordbox track"; `LibraryTrackItem`'s
current shape is already close to right, but `id` becomes a source-qualified identity.

Path-keyed joining (`rekordbox_track_overrides`, `rekordbox_track_ids`, `normalize_track_path` in
`main.rs`) stays in `crates/core`: it exists so a file dropped from Finder picks up its rekordbox
BPM and cues, which is an engine concern, not a browsing one.

## Beat-oriented workflow (the #32 half that is not a port)

Once the console runs on Swift, phase 1 adds the assistive layer. Design sketch, to be
detailed when the port lands:

- **Slices.** A slice is a beat range of a loaded track `(track, startBeat, beatCount)`,
  derived from the rekordbox beat grid already imported by `crates/library`. Extraction is a
  selection on the waveform, snapped to the grid — the same snapping `ToggleLoop` does today.
- **Pads.** Slices are playable as 4/8 pads per deck, triggered from the UI and from a
  keyboard row, quantized to the master tempo.
- **Assistive suggestions.** Given the current deck, suggest slices whose grid and key fit —
  ranked, never auto-fired. The AI role stays a ranking and a proposal; the user triggers.

This is what makes #32 more than a UI rewrite, and it is why the beat grid moves into
`crates/core` rather than staying in the UI layer.

---

## Sequence

1. **`crates/core` extraction** — done (#34).
2. **`crates/ffi` + `Vendor/build-rust.sh` + `sujay.xcodeproj`** — done (#34).
3. **The console port** — done (#34), then moved to the system look (#37 / #40).
4. **Retire the old host** — done (#35 / #41).
5. **Stage 1 of Rust removal: core in Swift** — done (#42's first PR): preferences, decode
   (AVFoundation), library load and reload, path-keyed rekordbox join, beat-loop maths,
   engine-state mapping, host stats all in Swift. `crates/core` and its JSON/snapshot ABI are gone;
   `crates/ffi` is a thin C surface over `AudioEngineCore` and `crates/library`.
6. **Stage 2: rekordbox reader in Swift** — `master.db` through SQLCipher, ANLZ parsing in Swift;
   `crates/library` goes.
7. **Stage 3: engine in Swift** — AVAudioEngine graph replacing `crates/audio`; the Rust workspace,
   `Vendor/build-rust.sh` and the script phase go.
8. **Beat workflow** — slices, pads, suggestions (#36), on whichever stage is current.

## Conventions

Taken from `../hukan`, which is the other Swift macOS app in this account:

```sh
xcodebuild build -project sujay.xcodeproj -scheme Sujay -derivedDataPath .build/DerivedData
xcodebuild test  -project sujay.xcodeproj -scheme Sujay -derivedDataPath .build/DerivedData
xcrun swift-format format -i -p -r Sources Tests
cargo clippy --all-targets          # still gates the Rust side
```

`.swift-format` is `{ "version": 1 }` — standard style, no house rules. `.githooks/pre-commit`
lints the staged blob and blocks the commit on any finding; activate once per clone with
`git config core.hooksPath .githooks`. Code, comments, documentation and commit messages are
in English.
