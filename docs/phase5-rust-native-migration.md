# Phase 5: Rust-Native App Migration

## Overview

Phase 5 retires Electron as the primary application host and replaces it
with a Rust binary (`packages/app`).  The goal is a self-contained
native executable that links the existing Rust crates directly — no
Node.js runtime required at launch.

This document describes the current state, the remaining work, and how
to build / run each path.

---

## Architecture: two host paths

```
┌─────────────────────────────────────────────────────────────────┐
│  Electron path (current shipped runtime)                        │
│                                                                 │
│  packages/app (Electron renderer + main)                        │
│    └── @sujay/audio  (Rust NAPI cdylib)                         │
│    └── @sujay/ui     (Rust NAPI cdylib)                         │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│  Rust-native path (Phase 5 target)                              │
│                                                                 │
│  packages/app/src/main.rs  (Rust binary)                        │
│    └── sujay_audio  (linked as rlib, same Rust source)          │
│    └── sujay_ui     (linked as rlib, same Rust source)          │
└─────────────────────────────────────────────────────────────────┘
```

Code that previously lived only in Electron has been extracted into
host-agnostic modules:

| Module | Location | Status |
|---|---|---|
| Audio engine lifecycle | `app/src/main/runtime/audio-runtime.ts` | ✅ Done — Electron-free |
| Settings persistence | `app/src/main/settings/app-settings-store.ts` | ✅ Done — plain JSON |
| Shell abstraction | `app/src/main/host/shell-host.ts` | ✅ Done |
| IPC abstraction | `app/src/main/host/electron-ipc-host.ts` | ✅ Done |
| Rust binary scaffold | `packages/app/src/main.rs` | ✅ Done — compiles |

Remaining Electron-only code in `app/src/main.ts` is marked
`[LEGACY-ELECTRON]` and is a candidate for removal once the Rust host
covers the same functionality.

---

## Building and running

### Electron path (current default)

```sh
# Development with hot reload
npm start

# Production package (macOS)
cd app && npm run package

# Create installer
cd app && npm run make
```

### Rust-native binary (Phase 5 scaffold)

```sh
# Debug build
cd packages/app && cargo build
./target/debug/sujay

# Release build
cd packages/app && cargo build --release
./target/release/sujay
```

The binary currently starts and exits.  Full functionality is gated
behind the TODOs in `packages/app/src/main.rs`.

---

## Phase 5 migration checklist

### Sub-systems to migrate

- [ ] **Settings loading** — read `settings.json` (same schema as TS side)
  - Add `serde` + `serde_json` to `packages/app/Cargo.toml`
  - Use `dirs` crate for platform data directory
- [ ] **Audio engine** — link `sujay_audio` as `rlib`
  - Add `rlib` to `crate-type` in `packages/audio/Cargo.toml`
  - Add `sujay_audio = { path = "../audio" }` dep
  - Wire state callback through a Rust channel
- [ ] **Native window** — create the application window
  - Add `winit` dep; create 1100×540 window
  - Attach `sujay_ui` (already a wgpu/egui renderer)
- [ ] **UI state push** — replace Electron IPC push with direct Rust calls
  - `sujay_ui::setConsoleState(...)`, `setDeckProgress(...)`, etc.
  - Called from the audio engine state callback
- [ ] **Library / Suno API** — track metadata and generation
  - Port `app/src/core/library-manager.ts` to Rust (or spawn as sidecar)
- [ ] **MCP server** — HTTP endpoint on `localhost:8888`
  - Use `axum` + `tokio`; same tool schema as current TS implementation
- [ ] **Graceful shutdown** — on window close, stop audio engine & flush recording

### Build / release changes needed

- [ ] Add `packages/app` to the Electron Forge extraResources or replace
  forge config entirely with a native bundler (e.g. `cargo-bundle`)
- [ ] Code-sign and notarise the Rust binary on macOS
- [ ] Update CI to build both paths and run smoke tests
- [ ] Remove Electron and Node.js from the macOS `.app` bundle once the
  Rust path is fully functional

---

## Acceptance criteria (issue #29)

1. ✅ App launches without Electron as primary runtime
   *(scaffold binary compiles and runs)*
2. ⬜ Core DJ workflow functional in Rust-native target
3. ✅ Settings/bootstrap/lifecycle owned by Rust-agnostic code
   *(audio-runtime.ts, app-settings-store.ts)*
4. ✅ Electron glue isolated as legacy-only
   *(`[LEGACY-ELECTRON]` markers in main.ts)*
5. ⬜ Build/release flow for Rust-native target documented
   *(this document — build commands above)*
