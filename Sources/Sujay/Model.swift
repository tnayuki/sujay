import CSujay
import Foundation
import Observation

/// The console's view model: drives the core at display rate and republishes
/// its state for SwiftUI. Main thread only.
@Observable
final class ConsoleModel {
  @ObservationIgnored let engine = Engine()

  /// Fast numeric state, refreshed every frame.
  var snapshot = SujaySnapshot()
  /// Titles, BPM text and cues, refreshed when the core says they changed.
  var text = ConsoleText()
  var library = Library()
  var preferences = Preferences()
  var decks: [DeckBuffers] = [DeckBuffers(), DeckBuffers()]
  /// Peak-hold level per deck, decayed here at frame rate.
  var peakHold: [Float] = [0, 0]
  var clock = ""
  private(set) var started = false

  @ObservationIgnored private var timer: Timer?
  @ObservationIgnored private var lastClockSecond = -1

  /// Engine start enumerates CoreAudio devices synchronously, and CoreAudio
  /// delivers its property notifications on the main run loop — doing that
  /// from inside a SwiftUI update deadlocks the main thread. So the start runs
  /// on a background thread and the frame timer begins once it has returned;
  /// the core is not touched from the main thread until then.
  func start() {
    guard !started else { return }
    started = true
    let engine = engine
    DispatchQueue.global(qos: .userInitiated).async {
      let ok = engine.start()
      DispatchQueue.main.async { [weak self] in
        if !ok {
          NSLog("sujay: audio engine failed to start")
        }
        self?.beginFrames()
      }
    }
  }

  private func beginFrames() {
    let timer = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in
      self?.frame()
    }
    RunLoop.main.add(timer, forMode: .common)
    self.timer = timer
  }

  func shutdown() {
    timer?.invalidate()
    timer = nil
    engine.shutdown()
  }

  private func frame() {
    let tick = engine.tick()
    if tick.console != 0, let text = engine.consoleText() {
      self.text = text
    }
    if tick.library != 0, let library = engine.library() {
      self.library = library
    }
    if tick.preferences != 0, let preferences = engine.preferences() {
      self.preferences = preferences
    }
    if tick.deck.0 != 0 {
      decks[0] = engine.deckBuffers(1)
    }
    if tick.deck.1 != 0 {
      decks[1] = engine.deckBuffers(2)
    }
    snapshot = engine.snapshot()
    for index in 0..<2 {
      let peak = deck(index).peak
      // ~1.5 s from full to zero at 60 fps, as the egui meter did at 30.
      peakHold[index] = peak > peakHold[index] ? peak : max(0, peakHold[index] - 0.006)
    }
    let now = Int(Date().timeIntervalSince1970)
    if now != lastClockSecond {
      lastClockSecond = now
      clock = Self.clockFormatter.string(from: Date())
    }
  }

  private static let clockFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateFormat = "HH:mm:ss"
    return formatter
  }()

  // MARK: Reading

  func deck(_ index: Int) -> SujayDeckSnapshot {
    index == 0 ? snapshot.deck.0 : snapshot.deck.1
  }

  func deckText(_ index: Int) -> DeckText {
    text.deck(index)
  }

  /// The core reports "---" as the title of an empty deck.
  func hasTrack(_ index: Int) -> Bool {
    let title = deckText(index).title
    return !title.isEmpty && title != "---"
  }

  func timeText(_ index: Int) -> String {
    let deck = deck(index)
    guard deck.loaded != 0, deck.sample_rate > 0 else { return "0:00" }
    let seconds = Int(deck.position_frames / deck.sample_rate)
    return String(format: "%d:%02d", seconds / 60, seconds % 60)
  }

  // MARK: Commands (deck index 0 = A, 1 = B)

  private func id(_ index: Int) -> UInt8 { UInt8(index + 1) }

  func togglePlay(_ index: Int) {
    if deck(index).playing != 0 {
      engine.stop(id(index))
    } else {
      engine.play(id(index))
    }
  }

  func setCrossfader(_ position: Float) { engine.setCrossfader(position) }
  func setMasterTempo(_ bpm: Float) { engine.setMasterTempo(bpm) }
  func setDeckGain(_ index: Int, _ gain: Float) { engine.setDeckGain(id(index), gain) }
  func toggleCue(_ index: Int) { engine.setCue(id(index), deck(index).cue_enabled == 0) }
  func setEQ(_ index: Int, _ band: EQBand, kill: Bool) { engine.setEQ(id(index), band, kill: kill) }
  func seek(_ index: Int, _ position: Float) { engine.seek(id(index), position) }
  func recallCue(_ index: Int, _ cue: CuePoint) {
    engine.recallCue(id(index), position: cue.position, loopEnd: cue.loopEnd)
  }
  func toggleLoop(_ index: Int, beats: Float) { engine.toggleLoop(id(index), beats: beats) }
  func toggleMic() { engine.setMicEnabled(snapshot.mic_enabled == 0) }
  func toggleRecording() {
    if snapshot.is_recording != 0 {
      engine.stopRecording()
    } else {
      engine.startRecording()
    }
  }
  func loadFile(_ index: Int, _ url: URL) { engine.loadFile(id(index), url) }
  func applyPreferences(_ preferences: Preferences) { engine.apply(preferences) }
  func refreshAudioDevices() {
    engine.refreshAudioDevices()
    if let preferences = engine.preferences() {
      self.preferences = preferences
    }
  }
}
