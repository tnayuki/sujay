import AVFoundation
import Foundation
import os

/// The audio engine: two decks mixed and routed to any channels of the
/// chosen output device, with talkover and session recording. One
/// `AVAudioSourceNode` renders everything; the AVAudioEngine only carries it
/// to the device. Commands run on the main thread; the render thread reads
/// deck state under try-locks and publishes `EngineState`.
final class Engine {
  static let maximumFrames = 4096

  let sampleRate: Double = 44100
  private let engine = AVAudioEngine()
  private var source: AVAudioSourceNode?
  private let decks: [Deck]
  private let recorder: Recorder
  private var mic: MicInput?

  // Mix state; the render thread reads these plainly (single-word writes).
  private var crossfader: Float = 0.5
  private var masterTempo: Float = 130
  private var micEnabled = false
  private let talkoverDucking: Float = 0.5
  private let micGain: Float = 1
  private var micPeak: Float = 0

  // Routing.
  private var outputChannels = 2
  private var mainChannels: [Int32?] = [0, 1]
  private var cueChannels: [Int32?] = [nil, nil]
  private(set) var currentDeviceID: AudioDeviceID?

  // Published state.
  private let stateLock = OSAllocatedUnfairLock()
  private var published = EngineState()

  // Render scratch, allocated once.
  private var deckLeft: [UnsafeMutablePointer<Float>] = []
  private var deckRight: [UnsafeMutablePointer<Float>] = []
  private var micLeft: UnsafeMutablePointer<Float>
  private var micRight: UnsafeMutablePointer<Float>
  private var mixLeft: UnsafeMutablePointer<Float>
  private var mixRight: UnsafeMutablePointer<Float>
  private var cueLeft: UnsafeMutablePointer<Float>
  private var cueRight: UnsafeMutablePointer<Float>
  private var scratchLists: [UnsafeMutablePointer<AudioBufferList>] = []

  init?() {
    guard let a = Deck(sampleRate: sampleRate, maximumFrames: Self.maximumFrames),
      let b = Deck(sampleRate: sampleRate, maximumFrames: Self.maximumFrames)
    else { return nil }
    decks = [a, b]
    recorder = Recorder(sampleRate: sampleRate)
    func buffer() -> UnsafeMutablePointer<Float> {
      let pointer = UnsafeMutablePointer<Float>.allocate(capacity: Self.maximumFrames)
      pointer.initialize(repeating: 0, count: Self.maximumFrames)
      return pointer
    }
    deckLeft = [buffer(), buffer()]
    deckRight = [buffer(), buffer()]
    micLeft = buffer()
    micRight = buffer()
    mixLeft = buffer()
    mixRight = buffer()
    cueLeft = buffer()
    cueRight = buffer()
    for _ in 0..<2 {
      let list = AudioBufferList.allocate(maximumBuffers: 2)
      scratchLists.append(list.unsafeMutablePointer)
    }
    guard rebuildGraph(deviceID: AudioDevices.defaultOutputDeviceID()) else { return nil }
  }

  deinit {
    engine.stop()
    for pointer in deckLeft + deckRight + [micLeft, micRight, mixLeft, mixRight, cueLeft, cueRight]
    {
      pointer.deallocate()
    }
    for list in scratchLists { free(list) }
  }

  // MARK: Device

  /// Pick the output device (nil = system default) and the main / cue
  /// channel pairs (-1 or nil disables a side). Rebuilds the graph.
  @discardableResult
  func configureDevice(deviceID name: String?, main: [Int32?], cue: [Int32?]) -> Bool {
    let id: AudioDeviceID?
    if let name, let found = AudioDevices.deviceID(named: name) {
      id = found
    } else {
      if let name { NSLog("sujay: device '\(name)' not found, using default") }
      id = AudioDevices.defaultOutputDeviceID()
    }
    guard rebuildGraph(deviceID: id) else { return false }
    let clamp = { (channel: Int32?) -> Int32? in
      guard let channel, channel >= 0, Int(channel) < self.outputChannels else { return nil }
      return channel
    }
    mainChannels = [clamp(main.first ?? nil), clamp(main.dropFirst().first ?? nil)]
    if mainChannels[0] == nil, mainChannels[1] == nil {
      mainChannels = [0, outputChannels > 1 ? 1 : nil]
    }
    cueChannels = [clamp(cue.first ?? nil), clamp(cue.dropFirst().first ?? nil)]
    NSLog(
      "sujay: device configured, channels=\(outputChannels) main=\(mainChannels) cue=\(cueChannels) mic=\(mic == nil ? "N/A" : "available")"
    )
    return true
  }

  private func rebuildGraph(deviceID: AudioDeviceID?) -> Bool {
    engine.stop()
    if let source { engine.detach(source) }
    mic = nil
    if let deviceID, let unit = engine.outputNode.audioUnit {
      var id = deviceID
      let status = AudioUnitSetProperty(
        unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &id,
        UInt32(MemoryLayout<AudioDeviceID>.size))
      if status != noErr { NSLog("sujay: could not select output device (\(status))") }
    }
    currentDeviceID = deviceID
    outputChannels = max(deviceID.map(AudioDevices.outputChannelCount) ?? 2, 2)
    guard
      let format = AVAudioFormat(
        standardFormatWithSampleRate: sampleRate, channels: AVAudioChannelCount(outputChannels))
    else { return false }
    let source = AVAudioSourceNode(format: format) { [unowned self] _, _, frameCount, output in
      render(frames: Int(frameCount), into: output)
      return noErr
    }
    engine.attach(source)
    engine.connect(source, to: engine.outputNode, format: format)
    self.source = source
    do {
      try engine.start()
    } catch {
      NSLog("sujay: audio engine start failed: \(error)")
      return false
    }
    if let deviceID { mic = MicInput(deviceID: deviceID, sampleRate: sampleRate) }
    return true
  }

  // MARK: Tracks

  /// Hand interleaved stereo PCM at the engine's sample rate to a deck.
  /// `beats` are audio frame indices.
  @discardableResult
  func loadTrack(deck: UInt8, pcm: [Float], bpm: Float?, beats: [Float], trackID: String) -> Bool {
    let target = decks[Self.index(deck)]
    target.lock.withLock {
      target.load(pcm: pcm, bpm: bpm, beats: beats, trackID: trackID, masterTempo: masterTempo)
    }
    return true
  }

  // MARK: Commands (deck 1 = A, 2 = B)

  private static func index(_ deck: UInt8) -> Int { deck <= 1 ? 0 : 1 }

  func play(_ deck: UInt8) {
    let index = Self.index(deck)
    let target = decks[index]
    let other = decks[1 - index]
    target.lock.withLock {
      guard target.hasTrack else { return }
      other.lock.withLock { alignToPlayingOther(target: target, reference: other) }
      target.playing = true
    }
  }

  /// Starting a deck while the other plays lands it on the beat matching the
  /// other's current beat (by index), or on the other's position without grids.
  private func alignToPlayingOther(target: Deck, reference: Deck) {
    guard reference.playing, reference.hasTrack else { return }
    var position = reference.position
    if !reference.beats.isEmpty {
      let referenceIndex =
        max(
          reference.beats.firstIndex { $0 > Float(reference.position) } ?? reference.beats.count, 1)
        - 1
      if !target.beats.isEmpty {
        let index = min(referenceIndex, target.beats.count - 1)
        position = Int(target.beats[index].rounded())
      }
    }
    target.seek(toFrame: position)
  }

  func stop(_ deck: UInt8) {
    let target = decks[Self.index(deck)]
    target.lock.withLock { target.playing = false }
  }

  /// `position` is a fraction of the track (0...1).
  func seek(_ deck: UInt8, _ position: Double) {
    let target = decks[Self.index(deck)]
    target.lock.withLock {
      target.seek(toFrame: Int(Double(target.totalFrames) * min(max(position, 0), 1)))
    }
  }

  func setCrossfader(_ position: Double) { crossfader = Float(min(max(position, 0), 1)) }

  func setMasterTempo(_ bpm: Double) {
    guard bpm > 0, bpm <= 300 else { return }
    masterTempo = Float(bpm)
    for deck in decks {
      deck.lock.withLock { deck.updateRate(masterTempo: masterTempo) }
    }
  }

  func setDeckGain(_ deck: UInt8, _ gain: Double) {
    let target = decks[Self.index(deck)]
    target.lock.withLock { target.faderGain = Float(min(max(gain, 0), 1)) }
  }

  func setEQ(_ deck: UInt8, _ band: EQBand, kill: Bool) {
    let target = decks[Self.index(deck)]
    target.lock.withLock {
      switch band {
      case .low: target.eq.killLow = kill
      case .mid: target.eq.killMid = kill
      case .high: target.eq.killHigh = kill
      }
    }
  }

  func setCue(_ deck: UInt8, _ enabled: Bool) {
    let target = decks[Self.index(deck)]
    target.lock.withLock { target.cueEnabled = enabled }
  }

  func setMicEnabled(_ enabled: Bool) { micEnabled = enabled && mic != nil }

  /// Loop bounds as fractions of the track (0...1).
  func setLoop(_ deck: UInt8, start: Double, end: Double) {
    let target = decks[Self.index(deck)]
    target.lock.withLock {
      let total = Double(target.totalFrames)
      target.loopStart = Int(total * min(max(start, 0), 1))
      target.loopEnd = Int(total * min(max(end, 0), 1))
      target.loopEnabled = target.loopEnd > target.loopStart
    }
  }

  /// Loop bounds in seconds; the playhead is moved inside the loop.
  func setBeatLoop(_ deck: UInt8, startSeconds: Double, endSeconds: Double) {
    let target = decks[Self.index(deck)]
    target.lock.withLock {
      let start = Int(startSeconds * sampleRate)
      let end = min(Int(endSeconds * sampleRate), target.totalFrames)
      guard end > start else { return }
      target.loopStart = start
      target.loopEnd = end
      target.loopEnabled = true
      if target.position >= end || target.position < start { target.seek(toFrame: start) }
    }
  }

  func clearLoop(_ deck: UInt8) {
    let target = decks[Self.index(deck)]
    target.lock.withLock {
      target.loopEnabled = false
      target.loopStart = 0
      target.loopEnd = 0
    }
  }

  @discardableResult
  func startRecording(path: String, format: RecordingFormat) -> Bool {
    do {
      try recorder.start(path: path, recordingFormat: format)
      return true
    } catch {
      NSLog("sujay: recording failed to start: \(error)")
      return false
    }
  }

  func stopRecording() { recorder.stop() }

  // MARK: State

  func state() -> EngineState {
    stateLock.withLock { published }
  }

  // MARK: Render (audio thread)

  private func render(frames: Int, into output: UnsafeMutablePointer<AudioBufferList>) {
    let frames = min(frames, Self.maximumFrames)
    var active = [false, false]
    var playing = [false, false]
    var gains: [Float] = [0, 0]
    var cue = [false, false]
    for (i, deck) in decks.enumerated() {
      guard deck.lock.lockIfAvailable() else { continue }
      playing[i] = deck.playing
      active[i] = deck.render(
        frames: frames, left: deckLeft[i], right: deckRight[i], scratch: scratchLists[i])
      gains[i] = deck.gain
      cue[i] = deck.cueEnabled
      if active[i] {
        var peak: Float = 0
        for f in 0..<frames {
          peak = max(peak, abs(deckLeft[i][f]), abs(deckRight[i][f]))
        }
        deck.updateMeters(peak: peak * deck.gain)
      } else {
        deck.updateMeters(peak: 0)
      }
      deck.lock.unlock()
    }

    // Crossfader: equal-power, and a stopped deck contributes nothing.
    let faderA = playing[0] ? cos(crossfader * .pi / 2) : 0
    let faderB = playing[1] ? sin(crossfader * .pi / 2) : 0
    let gainA = active[0] ? faderA * gains[0] : 0
    let gainB = active[1] ? faderB * gains[1] : 0
    for f in 0..<frames {
      mixLeft[f] = deckLeft[0][f] * gainA + deckLeft[1][f] * gainB
      mixRight[f] = deckRight[0][f] * gainA + deckRight[1][f] * gainB
    }

    // Talkover: duck the music and add the microphone.
    if micEnabled, let mic, mic.pull(frames: frames, left: micLeft, right: micRight) {
      let music = 1 - talkoverDucking
      var peak: Float = 0
      for f in 0..<frames {
        peak = max(peak, abs(micLeft[f]), abs(micRight[f]))
        mixLeft[f] = mixLeft[f] * music + micLeft[f] * micGain
        mixRight[f] = mixRight[f] * music + micRight[f] * micGain
      }
      micPeak = peak
    } else {
      micPeak = mic?.peak ?? 0
    }

    // Cue: pre-fader mix of the decks whose cue is on.
    let cueOn = cueChannels[0] != nil || cueChannels[1] != nil
    if cueOn {
      for f in 0..<frames {
        var left: Float = 0
        var right: Float = 0
        if cue[0], active[0] {
          left += deckLeft[0][f]
          right += deckRight[0][f]
        }
        if cue[1], active[1] {
          left += deckLeft[1][f]
          right += deckRight[1][f]
        }
        cueLeft[f] = left * 0.5
        cueRight[f] = right * 0.5
      }
    }

    recorder.push(left: mixLeft, right: mixRight, frames: frames)

    // Route to the device's channels.
    let out = UnsafeMutableAudioBufferListPointer(output)
    for channel in 0..<out.count {
      guard let data = out[channel].mData?.assumingMemoryBound(to: Float.self) else { continue }
      data.update(repeating: 0, count: frames)
    }
    func write(_ channel: Int32?, _ samples: UnsafeMutablePointer<Float>) {
      guard let channel, Int(channel) < out.count,
        let data = out[Int(channel)].mData?.assumingMemoryBound(to: Float.self)
      else { return }
      for f in 0..<frames { data[f] = min(max(samples[f], -1), 1) }
    }
    switch (mainChannels[0], mainChannels[1]) {
    case (.some, .some):
      write(mainChannels[0], mixLeft)
      write(mainChannels[1], mixRight)
    case (.some(let single), .none), (.none, .some(let single)):
      for f in 0..<frames { mixLeft[f] = (mixLeft[f] + mixRight[f]) * 0.5 }
      write(single, mixLeft)
    default:
      break
    }
    if cueOn {
      switch (cueChannels[0], cueChannels[1]) {
      case (.some, .some):
        write(cueChannels[0], cueLeft)
        write(cueChannels[1], cueRight)
      case (.some(let single), .none), (.none, .some(let single)):
        for f in 0..<frames { cueLeft[f] = (cueLeft[f] + cueRight[f]) * 0.5 }
        write(single, cueLeft)
      default:
        break
      }
    }

    publish()
  }

  private func publish() {
    guard stateLock.lockIfAvailable() else { return }
    defer { stateLock.unlock() }
    for (i, deck) in decks.enumerated() {
      guard deck.lock.lockIfAvailable() else { continue }
      var state = DeckState()
      state.positionFrames = Double(deck.playheadFrames)
      state.totalFrames = Double(deck.totalFrames)
      state.peak = deck.peak
      state.peakHold = deck.peakHold
      state.gain = deck.faderGain
      state.bpm = deck.bpm ?? 0
      state.loopStart = Float(deck.loopStart)
      state.loopEnd = Float(deck.loopEnd)
      state.playing = deck.playing
      state.cueEnabled = deck.cueEnabled
      state.eqLow = deck.eq.killLow
      state.eqMid = deck.eq.killMid
      state.eqHigh = deck.eq.killHigh
      state.loopEnabled = deck.loopEnabled
      state.loaded = deck.hasTrack
      state.sampleRate = Float(sampleRate)
      published.decks[i] = state
      deck.lock.unlock()
    }
    published.masterTempo = masterTempo
    published.crossfader = crossfader
    published.micPeak = micPeak
    published.micAvailable = mic != nil
    published.micEnabled = micEnabled
    published.isRecording = recorder.isRecording
  }
}
