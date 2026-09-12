import AppKit

/// Drive the console from AppleScript.
///
/// The console is otherwise reachable only by hand, and a window that moved is a click at the
/// wrong coordinates. Being scriptable means both state and actions are reachable from `osascript`
/// — for personal automation, and for checking a change without a screenshot.
///
/// The objects live in `SDScripting.swift`; this file is the verbs. Commands are delivered on the
/// main thread, which is where `ConsoleModel` and the engine are used anyway.

/// Resolve a command argument to a proxy. The value may already be the proxy, an array of them
/// (an `evaluatedReceivers` list), or an unevaluated object specifier — handle all three.
private func resolve<T>(_ value: Any?, as type: T.Type) -> T? {
  if let object = value as? T { return object }
  if let array = value as? [Any] { return array.lazy.compactMap { resolve($0, as: T.self) }.first }
  if let specifier = value as? NSScriptObjectSpecifier {
    return resolve(specifier.objectsByEvaluatingSpecifier, as: T.self)
  }
  return nil
}

extension NSScriptCommand {
  func argument<T>(_ key: String, as type: T.Type) -> T? {
    resolve(evaluatedArguments?[key], as: T.self)
  }

  /// Report failure as a real script error, the way any scriptable app does, so the caller's
  /// `try` block sees it rather than a string that reads like a result.
  @discardableResult
  func fail(_ message: String) -> Any? {
    scriptErrorNumber = 1
    scriptErrorString = message
    return nil
  }
}

// MARK: - Deck verbs

/// `play`, `pause` and `loop` take the deck as their receiver — `play deck 1`, or
/// `tell deck 1 to play` — so they are methods on the proxy rather than `NSScriptCommand`
/// subclasses: a custom command class would own the dispatch and the object direct-parameter
/// would never reach the proxy.
extension SDDeck {
  @objc func handlePlayScriptCommand(_ command: NSScriptCommand) -> Any? {
    guard let model else { return command.fail("the console is not running") }
    guard loaded else { return command.fail("deck \(name) has no track") }
    if !playing { model.togglePlay(index) }
    return "ok"
  }

  @objc func handlePauseScriptCommand(_ command: NSScriptCommand) -> Any? {
    guard let model else { return command.fail("the console is not running") }
    if playing { model.togglePlay(index) }
    return "ok"
  }

  @objc func handleLoopScriptCommand(_ command: NSScriptCommand) -> Any? {
    guard let model else { return command.fail("the console is not running") }
    guard let beats = command.evaluatedArguments?["beats"] as? NSNumber else {
      return command.fail("a loop length in beats is required")
    }
    guard loaded else { return command.fail("deck \(name) has no track") }
    model.toggleLoop(index, beats: max(Float(beats.doubleValue), 0))
    return "ok"
  }
}

extension SDCuePoint {
  @objc func handleRecallScriptCommand(_ command: NSScriptCommand) -> Any? {
    guard let model = ConsoleModel.current else {
      return command.fail("the console is not running")
    }
    guard let cue else { return command.fail("no cue point \(label) on this deck") }
    model.recallCue(deckIndex, cue)
    return "ok"
  }
}

// MARK: - Commands

/// `load "…" into deck 1`, `load POSIX file "…" into deck 1`, or `load track "…" into deck 2`. The
/// deck is named by `into` because
/// the direct parameter is what to load; a verb whose direct parameter is not a specifier does not
/// reach a `responds-to` method.
///
/// Decoding and the rekordbox join run in the background, so the command suspends itself and
/// replies once the deck has the track — otherwise the next line of every script would be a wait
/// loop, and `play deck 1` straight after a load would fail.
@objc(LoadCommand)
final class LoadCommand: NSScriptCommand {
  /// Long enough for a decode of any track, short enough to answer before the default Apple event
  /// timeout of two minutes.
  private static let timeout: TimeInterval = 60

  override func performDefaultImplementation() -> Any? {
    guard let model = ConsoleModel.current else {
      return fail("the console is not running")
    }
    guard let deck = argument("intoDeck", as: SDDeck.self) else { return fail("no such deck") }

    let path: String
    if let track = resolveDirectParameter(as: SDTrack.self) {
      guard let file = track.track?.filePath, !file.isEmpty else {
        return fail("that track is not in the library")
      }
      path = file
    } else if let url = directParameter as? URL {
      // `POSIX file "…"`, an alias, or a Finder item: Cocoa hands a file direct parameter over as
      // a URL.
      guard url.isFileURL else { return fail("\(url) is not a file") }
      path = url.path
    } else if let text = directParameter as? String {
      path = (text as NSString).expandingTildeInPath
    } else {
      return fail("a file, a POSIX path or a track is required")
    }
    guard FileManager.default.fileExists(atPath: path) else { return fail("no file at \(path)") }

    // What is on the deck now, so the wait ends on this load rather than on what was already there.
    let before = model.deck(deck.index).track?.id
    model.loadFile(deck.index, URL(fileURLWithPath: path))
    suspendExecution()
    wait(deck: deck, model: model, before: before, until: Date().addingTimeInterval(Self.timeout))
    return nil
  }

  private func wait(deck: SDDeck, model: ConsoleModel, before: UUID?, until deadline: Date) {
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.05) { [self] in
      if let track = model.deck(deck.index).track, track.id != before {
        resumeExecution(withResult: "loaded \(track.title) onto deck \(deck.name)")
        return
      }
      guard Date() < deadline else {
        // A decode that failed logs its reason and leaves the deck alone, so there is nothing to
        // wait for and nothing to report but the silence.
        fail("the track did not load; see ~/Library/Logs/Sujay/sujay.log")
        resumeExecution(withResult: nil)
        return
      }
      wait(deck: deck, model: model, before: before, until: deadline)
    }
  }

  private func resolveDirectParameter<T>(as type: T.Type) -> T? {
    if let object = directParameter as? T { return object }
    if let specifier = directParameter as? NSScriptObjectSpecifier {
      return specifier.objectsByEvaluatingSpecifier as? T
    }
    return nil
  }
}

/// Both decks and the mixer in three lines. Hidden, like the rest of the harness surface: it exists
/// so a check can read the whole console at once without a screenshot.
@objc(StatusCommand)
final class StatusCommand: NSScriptCommand {
  override func performDefaultImplementation() -> Any? {
    guard let model = ConsoleModel.current else {
      return fail("the console is not running")
    }
    var lines: [String] = []
    for index in 0..<2 {
      let proxy = SDDeck(index: index)
      guard proxy.loaded else {
        lines.append("deck \(proxy.name): empty")
        continue
      }
      let deck = model.deck(index)
      let kills =
        (deck.eqLow ? "L" : "-") + (deck.eqMid ? "M" : "-") + (deck.eqHigh ? "H" : "-")
      lines.append(
        String(
          format: "deck %@: %@ \"%@\" %.1f/%.1fs bpm %.1f gain %.2f monitor %@ kill %@ loop %g",
          proxy.name, proxy.playing ? "playing" : "paused", proxy.title, proxy.position,
          proxy.duration, proxy.bpm, proxy.gain, deck.cueEnabled ? "on" : "off", kills,
          proxy.loopEnabled ? proxy.loopBeats : 0))
    }
    lines.append(
      String(
        format: "mixer: crossfader %.2f master tempo %.1f mic %@ recording %@", model.crossfader,
        model.masterTempo, model.micEnabled ? "on" : "off", model.isRecording ? "on" : "off"))
    return lines.joined(separator: "\n")
  }
}
