import AppKit

/// The AppleScript object model.
///
/// Scripting addresses the console the way any scriptable app is addressed — `deck 1`,
/// `cue point "1" of deck 2`, `track "…"` — rather than through flat verbs over a hidden
/// selection. The commands themselves live in `Scripting.swift`.
///
/// `ConsoleModel` and the engine stay where they are: these `SD…` proxies hold an identity only —
/// a deck index, a cue's label, a rekordbox id — and re-resolve the real thing on every access. So
/// a proxy AppKit builds and discards mid-specifier-evaluation is always safe, and one that
/// outlives what it names reads empty rather than stale.
///
/// The `@objc(SD…)` names matter: the sdef binds each class by its Objective-C runtime name, and
/// Swift would otherwise mangle it to `Sujay.SD…`, leaving every property `missing value`.

/// The running console, or nil before the window has appeared.
private func consoleModel() -> ConsoleModel? { ConsoleModel.current }

/// The container every top-level element hangs from. The application is the root of the
/// specifier tree, so its own container specifier is nil.
private func applicationClassDescription() -> NSScriptClassDescription? {
  NSApp.classDescription as? NSScriptClassDescription
}

// MARK: - Application

extension NSApplication {
  @objc var decks: [SDDeck] { consoleModel() == nil ? [] : [SDDeck(index: 0), SDDeck(index: 1)] }

  @objc(valueInDecksWithUniqueID:)
  func valueInDecks(withUniqueID id: Any) -> SDDeck? {
    guard consoleModel() != nil, let number = id as? NSNumber, (1...2).contains(number.intValue)
    else { return nil }
    return SDDeck(index: number.intValue - 1)
  }

  @objc(valueInDecksWithName:)
  func valueInDecks(withName name: String) -> SDDeck? {
    guard consoleModel() != nil else { return nil }
    switch name.uppercased() {
    case "A", "1": return SDDeck(index: 0)
    case "B", "2": return SDDeck(index: 1)
    default: return nil
    }
  }

  /// Streaming entries in the rekordbox library are not files, so scripting sees the same local
  /// tracks the library table does.
  @objc var libraryTracks: [SDTrack] {
    (consoleModel()?.library.tracks ?? []).filter(\.isLocalFile).map { SDTrack(trackID: $0.id) }
  }

  @objc(valueInLibraryTracksWithUniqueID:)
  func valueInLibraryTracks(withUniqueID id: Any) -> SDTrack? {
    guard let key = id as? String,
      consoleModel()?.library.tracks.contains(where: { $0.id == key && $0.isLocalFile }) == true
    else { return nil }
    return SDTrack(trackID: key)
  }

  @objc(valueInLibraryTracksWithName:)
  func valueInLibraryTracks(withName name: String) -> SDTrack? {
    guard
      let track = consoleModel()?.library.tracks.first(where: {
        $0.isLocalFile && $0.title.localizedCaseInsensitiveCompare(name) == .orderedSame
      })
    else { return nil }
    return SDTrack(trackID: track.id)
  }

  /// Folders hold other playlists rather than tracks; the picker leaves them out and so does this.
  @objc var libraryPlaylists: [SDPlaylist] {
    (consoleModel()?.library.playlists ?? []).filter { !$0.isFolder }.map {
      SDPlaylist(playlistID: $0.id)
    }
  }

  @objc(valueInLibraryPlaylistsWithUniqueID:)
  func valueInLibraryPlaylists(withUniqueID id: Any) -> SDPlaylist? {
    guard let key = id as? String,
      consoleModel()?.library.playlists.contains(where: { $0.id == key && !$0.isFolder }) == true
    else { return nil }
    return SDPlaylist(playlistID: key)
  }

  @objc(valueInLibraryPlaylistsWithName:)
  func valueInLibraryPlaylists(withName name: String) -> SDPlaylist? {
    guard
      let playlist = consoleModel()?.library.playlists.first(where: {
        !$0.isFolder && $0.name.localizedCaseInsensitiveCompare(name) == .orderedSame
      })
    else { return nil }
    return SDPlaylist(playlistID: playlist.id)
  }

  @objc var crossfader: Double {
    get { Double(consoleModel()?.crossfader ?? 0.5) }
    set { consoleModel()?.setCrossfader(Float(newValue)) }
  }

  @objc var masterTempo: Double {
    get { Double(consoleModel()?.masterTempo ?? 0) }
    set { consoleModel()?.setMasterTempo(Float(newValue)) }
  }

  @objc var microphoneEnabled: Bool {
    get { consoleModel()?.micEnabled ?? false }
    set {
      guard let model = consoleModel(), model.micEnabled != newValue else { return }
      model.toggleMic()
    }
  }

  @objc var microphoneAvailable: Bool { consoleModel()?.micAvailable ?? false }

  @objc var recording: Bool {
    get { consoleModel()?.isRecording ?? false }
    set {
      guard let model = consoleModel(), model.isRecording != newValue else { return }
      model.toggleRecording()
    }
  }

  @objc var libraryPath: String { consoleModel()?.library.masterDbPath ?? "" }
}

// MARK: - Deck

/// A deck is addressed by its index — 0 is A, 1 is B — which is all the identity it needs: the two
/// decks are made once with the model and outlive every script.
@objc(SDDeck)
final class SDDeck: NSObject {
  let index: Int

  init(index: Int) {
    self.index = index
  }

  var model: ConsoleModel? { consoleModel() }
  private var deck: DeckModel? { consoleModel()?.deck(index) }

  @objc var uniqueID: Int { index + 1 }
  @objc var name: String { index == 0 ? "A" : "B" }
  @objc var title: String { deck?.track?.title ?? "" }
  @objc var loaded: Bool { deck?.hasTrack ?? false }
  @objc var playing: Bool { deck?.playing ?? false }
  @objc var bpm: Double { Double(deck?.bpm ?? 0) }
  @objc var level: Double { Double(deck?.peak ?? 0) }
  @objc var loopEnabled: Bool { deck?.loopEnabled ?? false }
  @objc var loopBeats: Double { Double(deck?.loopBeats ?? 0) }

  /// Seconds, both ways: the engine seeks by fraction of the track, and a script has no reason to
  /// know the length first.
  @objc var position: Double {
    get {
      guard let deck, deck.sampleRate > 0 else { return 0 }
      return deck.positionFrames / Double(deck.sampleRate)
    }
    set {
      guard let deck, deck.totalFrames > 0, deck.sampleRate > 0 else { return }
      let frames = newValue * Double(deck.sampleRate)
      model?.seek(index, Float(min(max(frames / deck.totalFrames, 0), 1)))
    }
  }

  @objc var duration: Double {
    guard let deck, deck.sampleRate > 0 else { return 0 }
    return deck.totalFrames / Double(deck.sampleRate)
  }

  @objc var gain: Double {
    get { Double(deck?.gain ?? 0) }
    set { model?.setDeckGain(index, Float(min(max(newValue, 0), 1))) }
  }

  @objc var monitoring: Bool {
    get { deck?.cueEnabled ?? false }
    set {
      guard let deck, deck.cueEnabled != newValue else { return }
      model?.toggleCue(index)
    }
  }

  @objc var lowKill: Bool {
    get { deck?.eqLow ?? false }
    set { model?.setEQ(index, .low, kill: newValue) }
  }

  @objc var midKill: Bool {
    get { deck?.eqMid ?? false }
    set { model?.setEQ(index, .mid, kill: newValue) }
  }

  @objc var highKill: Bool {
    get { deck?.eqHigh ?? false }
    set { model?.setEQ(index, .high, kill: newValue) }
  }

  @objc var cuePoints: [SDCuePoint] {
    (deck?.track?.cues ?? []).map { SDCuePoint(deckIndex: index, label: $0.label) }
  }

  @objc(valueInCuePointsWithUniqueID:)
  func valueInCuePoints(withUniqueID id: Any) -> SDCuePoint? {
    guard let key = id as? String else { return nil }
    return valueInCuePoints(withName: key)
  }

  @objc(valueInCuePointsWithName:)
  func valueInCuePoints(withName name: String) -> SDCuePoint? {
    guard deck?.track?.cues.contains(where: { $0.label == name }) == true else { return nil }
    return SDCuePoint(deckIndex: index, label: name)
  }

  override var objectSpecifier: NSScriptObjectSpecifier? {
    guard let description = applicationClassDescription() else { return nil }
    return NSUniqueIDSpecifier(
      containerClassDescription: description, containerSpecifier: nil, key: "decks",
      uniqueID: NSNumber(value: uniqueID))
  }

  override func isEqual(_ object: Any?) -> Bool {
    (object as? SDDeck)?.index == index
  }
  override var hash: Int { index }
}

// MARK: - Cue point

/// Cues belong to the loaded track, so the proxy keys on the deck and the cue's label; a deck
/// loaded with something else in the meantime simply has no cue by that name.
@objc(SDCuePoint)
final class SDCuePoint: NSObject {
  let deckIndex: Int
  let label: String

  init(deckIndex: Int, label: String) {
    self.deckIndex = deckIndex
    self.label = label
  }

  var cue: CuePoint? {
    consoleModel()?.deck(deckIndex).track?.cues.first { $0.label == label }
  }

  @objc var uniqueID: String { label }
  @objc var name: String { label }
  @objc var loop: Bool { cue?.loopEnd != nil }

  @objc var position: Double {
    guard let cue, let deck = consoleModel()?.deck(deckIndex), deck.sampleRate > 0 else { return 0 }
    return Double(cue.position) * deck.totalFrames / Double(deck.sampleRate)
  }

  override var objectSpecifier: NSScriptObjectSpecifier? {
    guard let container = SDDeck(index: deckIndex).objectSpecifier,
      let description = container.keyClassDescription
    else { return nil }
    return NSUniqueIDSpecifier(
      containerClassDescription: description, containerSpecifier: container, key: "cuePoints",
      uniqueID: label)
  }

  override func isEqual(_ object: Any?) -> Bool {
    guard let other = object as? SDCuePoint else { return false }
    return other.deckIndex == deckIndex && other.label == label
  }
  override var hash: Int { label.hashValue }
}

// MARK: - Library

@objc(SDTrack)
final class SDTrack: NSObject {
  let trackID: String

  init(trackID: String) {
    self.trackID = trackID
  }

  var track: Track? { consoleModel()?.library.tracks.first { $0.id == trackID } }

  @objc var uniqueID: String { trackID }
  @objc var name: String { track?.title ?? "" }
  @objc var artist: String { track?.artist ?? "" }
  @objc var album: String { track?.album ?? "" }
  @objc var bpm: Double { Double(track?.bpm ?? 0) }
  @objc var duration: Double { Double(track?.durationSeconds ?? 0) }
  @objc var posixPath: String { track?.filePath ?? "" }
  /// The same file as a `file`, which is what the rest of AppleScript passes around — `open
  /// location of track 1` in the Finder, say. `POSIX path` stays for the string a shell wants,
  /// the pair System Events and Image Events expose.
  @objc var location: NSURL? { track.map { $0.fileURL as NSURL } }

  override var objectSpecifier: NSScriptObjectSpecifier? {
    guard let description = applicationClassDescription() else { return nil }
    return NSUniqueIDSpecifier(
      containerClassDescription: description, containerSpecifier: nil, key: "libraryTracks",
      uniqueID: trackID)
  }

  override func isEqual(_ object: Any?) -> Bool {
    (object as? SDTrack)?.trackID == trackID
  }
  override var hash: Int { trackID.hashValue }
}

@objc(SDPlaylist)
final class SDPlaylist: NSObject {
  let playlistID: String

  init(playlistID: String) {
    self.playlistID = playlistID
  }

  private var playlist: Playlist? {
    consoleModel()?.library.playlists.first { $0.id == playlistID }
  }

  @objc var uniqueID: String { playlistID }
  @objc var name: String { playlist?.name ?? "" }

  /// The playlist's own order, dropping what cannot be played.
  @objc var tracks: [SDTrack] {
    guard let playlist, let library = consoleModel()?.library else { return [] }
    let byID = Dictionary(
      uniqueKeysWithValues: library.tracks.filter(\.isLocalFile).map { ($0.id, $0) })
    return playlist.trackIds.compactMap { byID[$0] == nil ? nil : SDTrack(trackID: $0) }
  }

  @objc(valueInTracksWithUniqueID:)
  func valueInTracks(withUniqueID id: Any) -> SDTrack? {
    guard let key = id as? String, playlist?.trackIds.contains(key) == true else { return nil }
    return NSApp.valueInLibraryTracks(withUniqueID: key)
  }

  @objc(valueInTracksWithName:)
  func valueInTracks(withName name: String) -> SDTrack? {
    tracks.first { $0.name.localizedCaseInsensitiveCompare(name) == .orderedSame }
  }

  override var objectSpecifier: NSScriptObjectSpecifier? {
    guard let description = applicationClassDescription() else { return nil }
    return NSUniqueIDSpecifier(
      containerClassDescription: description, containerSpecifier: nil, key: "libraryPlaylists",
      uniqueID: playlistID)
  }

  override func isEqual(_ object: Any?) -> Bool {
    (object as? SDPlaylist)?.playlistID == playlistID
  }
  override var hash: Int { playlistID.hashValue }
}
