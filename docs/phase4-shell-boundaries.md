# Phase 4: Electron shell decoupling boundaries

This document captures the host-neutral boundaries introduced in issue #26.

## Host abstraction boundary

The app shell lifecycle is now routed through a host interface in `app/src/main/host/shell-host.ts`.

Current Electron adapter responsibilities:
- Resolve platform and app metadata (`platform`, `appName`)
- Resolve host paths (`music`)
- Register lifecycle hooks (`ready`, `window-all-closed`, `activate`, `before-quit`)
- Create windows and own menu construction bridge
- Query process metrics for system info

Future Rust host mapping:
- `onReady` -> Rust runtime bootstrap callback
- `createBrowserWindow` -> Rust host window constructor (winit/tao/egui shell)
- `buildMenuFromTemplate` / `setApplicationMenu` -> host-native menu model
- `shouldOpenMainWindow` -> host window registry query
- `getAppMetrics` -> Rust runtime metrics source

Lifecycle hook wiring is now centralized in `app/src/main/runtime/lifecycle-bootstrap.ts`.

## Settings persistence boundary

`electron-store` is now isolated in `app/src/main/settings/app-settings-store.ts`.
Feature code reads/writes through the `AppSettingsStore` interface only.

Current settings ports:
- `getAudioConfig` / `setAudioConfig`
- `getOscConfig` / `setOscConfig`
- `getRecordingConfig` / `setRecordingConfig`

Future Rust host mapping:
- Implement the same interface on top of a Rust-backed config store (TOML/JSON/SQLite)
- Keep renderer-facing IPC unchanged while swapping backend persistence

## IPC contract boundary

IPC channel names are centralized in `app/src/main/ipc-contract.ts` and consumed by both `main.ts` and `preload.ts`.

This avoids string drift and makes host replacement simpler because the contract surface is explicit and auditable.

Main-process IPC registration and renderer event delivery are routed through `app/src/main/host/electron-ipc-host.ts` (`IpcHost`).
This confines direct `ipcMain` usage to an adapter module, which allows swapping to a Rust-host bridge with the same runtime-facing shape.

## IPC surface reduction in this phase

Removed main-process handlers that were not used by the current renderer bridge:
- `native-ui:set-progress`
- `native-ui:set-markers`
- `native-ui:set-console-state`

Native deck progress/markers/console synchronization remains internal to the main process via direct native module calls.

## Behavioral compatibility

No user-facing behavior change is intended in this phase.
- Existing preload API shape is preserved
- Existing recording/audio/OSC/native-ui workflows continue through the same renderer methods
- Lifecycle behavior remains equivalent with host adapter indirection
