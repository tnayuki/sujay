//! Sujay — Rust-native host entry point.
//!
//! This binary is the future primary runtime for Sujay, replacing Electron as
//! the application host.  It owns:
//!
//!   - Process lifecycle (init, event loop, graceful shutdown)
//!   - Settings loading (JSON file — identical schema as the TypeScript side)
//!   - AudioEngine initialization and direct Rust API (no NAPI bridge needed)
//!   - Native window / UI management (via sujay_ui / wgpu + egui)
//!   - MCP HTTP server for AI-automation integrations
//!
//! ## Current status — Phase 5 scaffold
//!
//! This binary compiles and exits cleanly.  The real implementation will be
//! layered in incrementally as each sub-system is migrated away from Electron.
//! See `docs/phase5-rust-native-migration.md` for the full plan and build
//! instructions.
//!
//! ## Why a separate binary instead of NAPI?
//!
//! The existing `packages/audio` and `packages/ui` crates are compiled as
//! `cdylib` NAPI addons that run inside a Node.js/Electron process.  The
//! Rust-native path links those crates directly as Rust libraries, cutting out
//! the Node.js/Electron layer entirely.  `packages/audio/Cargo.toml` will gain
//! `rlib` to its `crate-type` list so it can be consumed both ways.

fn main() {
    // Initialise logging from RUST_LOG (default: info).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!(
        "Sujay {} — Rust-native host starting",
        env!("CARGO_PKG_VERSION")
    );

    // ── Phase 5 TODO: Settings ───────────────────────────────────────────────
    // Load app settings from the same JSON file the Electron path writes.
    //
    //   let settings_path = dirs::data_dir()
    //       .expect("no data dir")
    //       .join("Sujay/settings.json");
    //   let settings = Settings::load(&settings_path).unwrap_or_default();
    //   tracing::info!("settings loaded from {}", settings_path.display());

    // ── Phase 5 TODO: Audio engine ───────────────────────────────────────────
    // Link sujay_audio directly (requires adding `rlib` to its crate-type).
    //
    //   let audio_engine = sujay_audio::AudioEngine::new(
    //       None,        // device id — use system default
    //       Some(2),     // stereo
    //       Some(44100), // sample rate
    //       Some(Box::new(|state| { /* state callback */ })),
    //   ).expect("failed to create audio engine");
    //   tracing::info!("audio engine initialised");

    // ── Phase 5 TODO: Native window ──────────────────────────────────────────
    // Create the application window via winit + attach sujay_ui (wgpu/egui).
    //
    //   let event_loop = winit::event_loop::EventLoop::new();
    //   let window = winit::window::WindowBuilder::new()
    //       .with_title("Sujay")
    //       .with_inner_size(winit::dpi::LogicalSize::new(1100.0_f64, 540.0_f64))
    //       .build(&event_loop)
    //       .expect("failed to create window");
    //   sujay_ui::attach(window.raw_window_handle(), 0, 0, 1100, 540);
    //   tracing::info!("native window created");

    // ── Phase 5 TODO: MCP server ─────────────────────────────────────────────
    // Spawn the MCP HTTP server on localhost:8888 (same port as Electron path).
    //
    //   let rt = tokio::runtime::Runtime::new().unwrap();
    //   rt.spawn(mcp::Server::run(8888, audio_engine.clone()));

    // ── Phase 5 TODO: Event loop ─────────────────────────────────────────────
    //
    //   event_loop.run(move |event, _, control_flow| {
    //       *control_flow = winit::event_loop::ControlFlow::Poll;
    //       match event {
    //           winit::event::Event::WindowEvent {
    //               event: winit::event::WindowEvent::CloseRequested, ..
    //           } => {
    //               audio_engine.close();
    //               *control_flow = winit::event_loop::ControlFlow::Exit;
    //           }
    //           _ => {}
    //       }
    //   });

    tracing::info!(
        "Phase 5 scaffold complete — primary runtime is still Electron. \
         Run `npm start` from the repo root to launch the app."
    );
}
