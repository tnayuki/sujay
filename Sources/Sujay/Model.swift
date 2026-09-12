import Foundation
import Observation

/// The console's model and the host orchestration in one place: it owns the
/// engine, the preferences, the rekordbox library, the loaded tracks and the
/// frame timer, and republishes engine state for SwiftUI. Main thread only,
/// except where noted.
@Observable
final class ConsoleModel {
  /// The running console. There is exactly one, and both the app delegate and the scripting layer
  /// reach it here: SwiftUI hands the `@NSApplicationDelegateAdaptor` a delegate that is not the
  /// one `NSApp` keeps, so a reference stored on it from a view never arrives.
  static private(set) weak var current: ConsoleModel?

  @ObservationIgnored private(set) var engine: Engine?

  /// Per-deck state, one observable property per fact.
  let decks = [DeckModel(), DeckModel()]
  // Console-wide state; each property notifies only when it changes.
  var masterTempo: Float = 130
  var crossfader: Float = 0.5
  var micPeak: Float = 0
  var micAvailable = false
  var micEnabled = false
  var isRecording = false
  var recElapsedSecs: UInt32 = 0
  var cpuPercent: Double = 0
  var memoryBytes: UInt64 = 0
  var clock = ""
  var library = Library()
  var libraryStatus = "Loading rekordbox library…"
  var preferences = Preferences.load()
  var audioDevices: [AudioDevice] = []
  private(set) var started = false

  @ObservationIgnored private var timer: Timer?
  @ObservationIgnored private var lastClockSecond = -1
  @ObservationIgnored private var usage = SystemUsage()
  @ObservationIgnored private var lastUsageSample = Date.distantPast
  @ObservationIgnored private var recordingStartedAt: Date?
  @ObservationIgnored private var libraryIndex: [String: Track] = [:]
  @ObservationIgnored private var libraryModified: Date?
  @ObservationIgnored private var libraryLoadInFlight = false
  @ObservationIgnored private var nextLibraryCheck = Date.distantPast
  @ObservationIgnored private var loadGeneration = [0, 0]
  /// Called after every frame's state update; the waveform and meter views
  /// redraw from here rather than through SwiftUI.
  @ObservationIgnored private var frameListeners: [UUID: () -> Void] = [:]

  // MARK: Lifecycle

  /// Engine start enumerates CoreAudio devices synchronously, and CoreAudio
  /// delivers its notifications on the main run loop, so it runs on a
  /// background thread; the frame timer begins once it has returned.
  func start() {
    guard !started else { return }
    started = true
    Self.current = self
    DispatchQueue.global(qos: .userInitiated).async { [self] in
      let engine = Engine()
      let devices = AudioDevices.outputDevices()
      DispatchQueue.main.async { [self] in
        guard let engine else {
          NSLog("sujay: audio engine failed to start")
          return
        }
        self.engine = engine
        audioDevices = devices
        preferences.normalize(devices: devices)
        engine.configureDevice(
          deviceID: preferences.audioDeviceId, main: preferences.mainChannels,
          cue: preferences.cueChannels)
        beginFrames()
        reloadLibrary()
        // Headless testing: SUJAY_AUTOPLAY=<audio file> loads it on deck A and plays.
        if let path = ProcessInfo.processInfo.environment["SUJAY_AUTOPLAY"] {
          loadFile(0, URL(fileURLWithPath: path))
          DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [self] in
            if hasTrack(0), !deck(0).playing { togglePlay(0) }
          }
        }
      }
    }
  }

  func shutdown() {
    timer?.invalidate()
    timer = nil
    engine = nil
  }

  private func beginFrames() {
    let timer = Timer(timeInterval: 1.0 / 60.0, repeats: true) { [weak self] _ in
      self?.frame()
    }
    RunLoop.main.add(timer, forMode: .common)
    self.timer = timer
  }

  private func frame() {
    guard let engine else { return }
    let state = engine.state()
    for (index, deck) in decks.enumerated() { deck.apply(state.decks[index]) }
    set(\.masterTempo, state.masterTempo)
    set(\.crossfader, state.crossfader)
    set(\.micPeak, state.micPeak)
    set(\.micAvailable, state.micAvailable)
    set(\.micEnabled, state.micEnabled)
    set(\.isRecording, state.isRecording)
    if state.isRecording {
      if recordingStartedAt == nil { recordingStartedAt = Date() }
      set(\.recElapsedSecs, UInt32(Date().timeIntervalSince(recordingStartedAt ?? Date())))
    } else {
      recordingStartedAt = nil
      set(\.recElapsedSecs, 0)
    }

    for listener in frameListeners.values { listener() }

    let now = Date()
    if now.timeIntervalSince(lastUsageSample) >= 2 {
      lastUsageSample = now
      let sample = usage.sample()
      set(\.cpuPercent, sample.cpuPercent)
      set(\.memoryBytes, sample.memoryBytes)
    }

    let second = Int(now.timeIntervalSince1970)
    if second != lastClockSecond {
      lastClockSecond = second
      clock = Self.clockFormatter.string(from: now)
    }
    pollLibraryReload()
  }

  private static let clockFormatter: DateFormatter = {
    let formatter = DateFormatter()
    formatter.dateFormat = "HH:mm:ss"
    return formatter
  }()

  // MARK: Frame listeners

  func addFrameListener(_ listener: @escaping () -> Void) -> UUID {
    let id = UUID()
    frameListeners[id] = listener
    return id
  }

  func removeFrameListener(_ id: UUID) {
    frameListeners[id] = nil
  }

  // MARK: Reading

  func deck(_ index: Int) -> DeckModel { decks[index] }
  func hasTrack(_ index: Int) -> Bool { decks[index].hasTrack }

  // MARK: Library

  /// Load the browse list on a background thread; called at start and when
  /// rekordbox rewrites `master.db`.
  func reloadLibrary() {
    guard !libraryLoadInFlight else { return }
    libraryLoadInFlight = true
    DispatchQueue.global(qos: .utility).async { [self] in
      let result = Result { try RekordboxReader.loadLibrary() }
      DispatchQueue.main.async { [self] in
        libraryLoadInFlight = false
        switch result {
        case .success(let loaded):
          library = loaded
          libraryIndex = loaded.index()
          libraryStatus = loaded.masterDbPath
          libraryModified = Self.modificationDate(loaded.masterDbPath)
          NSLog("sujay: rekordbox library loaded, \(loaded.tracks.count) tracks")
        case .failure(let error):
          NSLog("sujay: rekordbox library failed: \(error)")
          if library.tracks.isEmpty { libraryStatus = "Rekordbox library not found" }
        }
      }
    }
  }

  private func pollLibraryReload() {
    let now = Date()
    guard !libraryLoadInFlight, now >= nextLibraryCheck, !library.masterDbPath.isEmpty else {
      return
    }
    nextLibraryCheck = now.addingTimeInterval(2)
    if let modified = Self.modificationDate(library.masterDbPath), modified != libraryModified {
      NSLog("sujay: master.db changed, reloading library")
      reloadLibrary()
    }
  }

  private static func modificationDate(_ path: String) -> Date? {
    (try? FileManager.default.attributesOfItem(atPath: path))?[.modificationDate] as? Date
  }

  // MARK: Loading

  /// Decode `url` in the background and hand it to `index`'s deck, joining
  /// rekordbox metadata by path when the library knows the file. A newer
  /// load for the same deck wins.
  func loadFile(_ index: Int, _ url: URL) {
    guard let engine else { return }
    loadGeneration[index] += 1
    let generation = loadGeneration[index]
    let known =
      libraryIndex[url.path] ?? libraryIndex[url.resolvingSymlinksInPath().path]
    let masterDB = library.masterDbPath
    let sampleRate = engine.sampleRate
    NSLog("sujay: load deck \(index + 1) \(url.path) rekordbox=\(known?.id ?? "-")")

    DispatchQueue.global(qos: .userInitiated).async { [self] in
      var analysis: TrackAnalysis?
      if let known, !masterDB.isEmpty {
        do {
          analysis = try RekordboxReader.analysis(masterDB: masterDB, contentID: known.id)
        } catch {
          NSLog("sujay: rekordbox analysis unavailable for \(url.lastPathComponent): \(error)")
        }
      }
      let decoded: AudioDecoder.Decoded
      do {
        decoded = try AudioDecoder.decode(url, sampleRate: sampleRate)
      } catch {
        NSLog("sujay: decode failed for \(url.path): \(error)")
        return
      }
      let beats =
        analysis?.beatsMs.map { Float(Double($0) / 1000 * sampleRate) }.filter(\.isFinite) ?? []
      let track = LoadedTrack(
        title: known?.title.isEmpty == false ? known!.title : url.lastPathComponent,
        bpm: known?.bpm, beats: beats,
        cues: CuePoint.from(
          analysis?.cues ?? [], totalFrames: decoded.frames, sampleRate: sampleRate),
        waveform: decoded.waveform, waveformColors: analysis?.waveformRgb ?? [],
        totalFrames: decoded.frames)
      DispatchQueue.main.async { [self] in
        guard generation == loadGeneration[index], let engine = self.engine else { return }
        if engine.loadTrack(
          deck: UInt8(index + 1), pcm: decoded.pcm, bpm: known?.bpm, beats: beats,
          trackID: track.title)
        {
          decks[index].track = track
          NSLog("sujay: deck \(index + 1) loaded \(track.title) bpm=\(track.bpmText)")
        } else {
          NSLog("sujay: engine stayed busy; deck \(index + 1) not loaded")
        }
      }
    }
  }

  // MARK: Commands (deck index 0 = A, 1 = B)

  /// Every command below also writes what it set into the published state instead of waiting for
  /// it to come back: the engine publishes on its render callback and the frame timer reads that
  /// a frame later, so anything reading straight back — a script above all — would see the old
  /// value for tens of milliseconds. What the engine refuses is not mirrored, and the next frame
  /// corrects whatever the engine rounded.

  private func id(_ index: Int) -> UInt8 { UInt8(index + 1) }

  func togglePlay(_ index: Int) {
    let deck = self.deck(index)
    if deck.playing {
      engine?.stop(id(index))
      deck.playing = false
    } else {
      guard deck.hasTrack else { return }
      engine?.play(id(index))
      deck.playing = true
    }
  }

  func setCrossfader(_ position: Float) {
    engine?.setCrossfader(Double(position))
    set(\.crossfader, min(max(position, 0), 1))
  }

  func setMasterTempo(_ bpm: Float) {
    engine?.setMasterTempo(Double(bpm))
    guard bpm > 0, bpm <= 300 else { return }
    set(\.masterTempo, bpm)
  }

  func setDeckGain(_ index: Int, _ gain: Float) {
    engine?.setDeckGain(id(index), Double(gain))
    let clamped = min(max(gain, 0), 1)
    if deck(index).gain != clamped { deck(index).gain = clamped }
  }

  func toggleCue(_ index: Int) {
    let enabled = !deck(index).cueEnabled
    engine?.setCue(id(index), enabled)
    deck(index).cueEnabled = enabled
  }

  func setEQ(_ index: Int, _ band: EQBand, kill: Bool) {
    engine?.setEQ(id(index), band, kill: kill)
    let deck = self.deck(index)
    switch band {
    case .low: if deck.eqLow != kill { deck.eqLow = kill }
    case .mid: if deck.eqMid != kill { deck.eqMid = kill }
    case .high: if deck.eqHigh != kill { deck.eqHigh = kill }
    }
  }

  func seek(_ index: Int, _ position: Float) {
    engine?.seek(id(index), Double(position))
    let deck = self.deck(index)
    deck.positionFrames = Double(min(max(position, 0), 1)) * deck.totalFrames
  }

  func toggleMic() {
    let enabled = !micEnabled
    engine?.setMicEnabled(enabled)
    set(\.micEnabled, enabled && micAvailable)
  }

  /// Jump to a cue; a loop cue also arms its loop, a plain cue clears any loop.
  func recallCue(_ index: Int, _ cue: CuePoint) {
    guard let engine else { return }
    engine.seek(id(index), Double(cue.position))
    let deck = self.deck(index)
    deck.positionFrames = Double(cue.position) * deck.totalFrames
    if let loopEnd = cue.loopEnd {
      engine.setLoop(id(index), start: Double(cue.position), end: Double(loopEnd))
      armLoop(
        index, start: cue.position * Float(deck.totalFrames), end: loopEnd * Float(deck.totalFrames)
      )
    } else {
      engine.clearLoop(id(index))
      clearLoop(index)
    }
  }

  /// Mirror an armed loop, in frames, the way the engine will publish it.
  private func armLoop(_ index: Int, start: Float, end: Float) {
    let deck = self.deck(index)
    let clamped = min(end, Float(deck.totalFrames))
    guard clamped > start else { return }
    deck.loopStart = start
    deck.loopEnd = clamped
    deck.loopEnabled = true
  }

  private func clearLoop(_ index: Int) {
    let deck = self.deck(index)
    deck.loopStart = 0
    deck.loopEnd = 0
    deck.loopEnabled = false
  }

  private func set<T: Equatable>(_ keyPath: ReferenceWritableKeyPath<ConsoleModel, T>, _ value: T) {
    if self[keyPath: keyPath] != value { self[keyPath: keyPath] = value }
  }

  /// Set a loop of `beats` from the beat before the playhead, snapped to the
  /// track's beat grid; past the grid, or without one, a beat is 60/120 s.
  /// `beats <= 0` clears the loop.
  func toggleLoop(_ index: Int, beats: Float) {
    guard let engine else { return }
    guard beats > 0 else {
      engine.clearLoop(id(index))
      clearLoop(index)
      return
    }
    let grid = decks[index].track?.beats ?? []
    let current = Float(deck(index).positionFrames)
    let sampleRate = Float(engine.sampleRate)
    let fallbackInterval = sampleRate * 60 / 120

    let startIndex = max(grid.firstIndex { $0 > current } ?? grid.count, 1) - 1
    let start = startIndex < grid.count ? grid[startIndex] : current
    let whole = Int(beats.rounded(.down))
    let fraction = beats - beats.rounded(.down)
    let end: Float
    if fraction < 0.001 {
      let endIndex = startIndex + whole
      if endIndex < grid.count {
        end = grid[endIndex]
      } else {
        let interval =
          grid.count >= 2 ? grid[grid.count - 1] - grid[grid.count - 2] : fallbackInterval
        end = start + interval * Float(whole)
      }
    } else {
      let interval: Float
      if startIndex + 1 < grid.count {
        interval = grid[startIndex + 1] - start
      } else if grid.count >= 2 {
        interval = grid[grid.count - 1] - grid[grid.count - 2]
      } else {
        interval = fallbackInterval
      }
      end = start + interval * beats
    }
    engine.setBeatLoop(
      id(index), startSeconds: Double(start / sampleRate), endSeconds: Double(end / sampleRate))
    armLoop(index, start: start, end: end)
  }

  func toggleRecording() {
    guard let engine else { return }
    if isRecording {
      // No mirror here: the writer thread is still draining the ring into the file, and the engine
      // goes on reporting a recording until it has.
      engine.stopRecording()
      return
    }
    do {
      let path = try preferences.recordingPath()
      if engine.startRecording(path: path, format: preferences.format) {
        isRecording = true
      } else {
        NSLog("sujay: recording did not start")
      }
    } catch {
      NSLog("sujay: recording path: \(error)")
    }
  }

  // MARK: Preferences

  func refreshAudioDevices() {
    audioDevices = AudioDevices.outputDevices()
    preferences.normalize(devices: audioDevices)
  }

  /// Persist edited preferences and apply them to the engine.
  func applyPreferences(_ edited: Preferences) {
    var next = edited
    next.normalize(devices: audioDevices)
    preferences = next
    do {
      try next.save()
    } catch {
      NSLog("sujay: saving preferences failed: \(error)")
    }
    engine?.configureDevice(
      deviceID: next.audioDeviceId, main: next.mainChannels, cue: next.cueChannels)
  }
}
