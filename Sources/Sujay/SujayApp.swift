import AppKit
import SwiftUI

@main
struct SujayApp: App {
  @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
  @State private var model = ConsoleModel()

  var body: some Scene {
    Window("Sujay", id: "main") {
      ContentView()
        .environment(model)
        .frame(minWidth: 960, minHeight: 680)
        .onAppear {
          model.start()
          appDelegate.model = model
        }
    }
    .defaultSize(width: 1100, height: 780)

    Settings {
      SettingsView()
        .environment(model)
    }
  }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
  var model: ConsoleModel?

  /// The Rust core reports through stderr. When the app is launched from
  /// Finder or `open` that goes nowhere, so redirect it to
  /// ~/Library/Logs/Sujay/sujay.log; a terminal launch keeps its terminal.
  func applicationWillFinishLaunching(_ notification: Notification) {
    guard isatty(STDERR_FILENO) == 0 else { return }
    let directory = FileManager.default.urls(for: .libraryDirectory, in: .userDomainMask)[0]
      .appendingPathComponent("Logs/Sujay", isDirectory: true)
    try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let path = directory.appendingPathComponent("sujay.log").path
    if freopen(path, "a", stderr) != nil {
      setvbuf(stderr, nil, _IONBF, 0)
      NSLog("sujay: stderr redirected to \(path)")
    }
  }

  func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    true
  }

  func applicationWillTerminate(_ notification: Notification) {
    model?.shutdown()
  }
}
