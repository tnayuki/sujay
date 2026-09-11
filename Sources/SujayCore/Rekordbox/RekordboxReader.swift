import Foundation

/// Reads a rekordbox library straight out of `master.db` and the ANLZ files beside it.
///
/// Both calls are blocking and are meant to run off the main thread: the browse list is one
/// pass over the database, and analysis is one row plus the three ANLZ files for that track.
enum RekordboxReader {
  /// The browse list without per-track analysis; nil path = the newest `master.db` under
  /// ~/Library/Pioneer.
  static func loadLibrary(masterDB: String? = nil) throws -> Library {
    guard let path = masterDB ?? defaultMasterDBPath() else {
      throw LibraryError.load("no rekordbox master.db found under ~/Library/Pioneer")
    }
    let db = try MasterDB(path: path)
    return Library(masterDbPath: path, tracks: try tracks(db), playlists: try playlists(db))
  }

  /// One track's beat grid, cues and waveform colours.
  static func analysis(masterDB: String, contentID: String) throws -> TrackAnalysis {
    let db = try MasterDB(path: masterDB)
    var analysisDataPath: String?
    try db.query(
      "SELECT AnalysisDataPath FROM djmdContent WHERE ID = ?", [contentID]
    ) { analysisDataPath = $0.nonEmptyText(0) }

    var anlz: ANLZ?
    if let analysisDataPath {
      // AnalysisDataPath is rooted at the share directory that sits next to master.db, and
      // names the .DAT; its siblings .EXT and .2EX hold the rest of the analysis.
      let share = URL(fileURLWithPath: masterDB).deletingLastPathComponent()
        .appendingPathComponent("share")
      let file = share.appendingPathComponent(
        analysisDataPath.hasPrefix("/") ? String(analysisDataPath.dropFirst()) : analysisDataPath)
      anlz = ANLZ(directory: file.deletingLastPathComponent())
    }

    return TrackAnalysis(
      beatsMs: anlz?.beatsMs() ?? [],
      cues: merge(try storedCues(db, contentID: contentID), anlz?.cues() ?? []),
      waveformRgb: anlz?.waveformRGB() ?? [])
  }

  // MARK: Tracks

  private static func tracks(_ db: MasterDB) throws -> [Track] {
    var tracks: [Track] = []
    try db.query(
      """
      SELECT c.ID, c.Title, c.FileNameL, c.FileNameS,
             c.SrcArtistName, artist.Name, c.SrcAlbumName, album.Name,
             c.BPM, c.Length, c.Rating, c.Tag, c.ReleaseDate, c.DateCreated,
             c.rb_LocalFolderPath, c.FolderPath, c.OrgFolderPath
        FROM djmdContent c
        LEFT JOIN djmdArtist artist ON artist.ID = c.ArtistID
        LEFT JOIN djmdAlbum album ON album.ID = c.AlbumID
       WHERE c.rb_local_deleted = 0
      """
    ) { row in
      guard let id = row.text(0),
        let filePath = audioPath(
          folders: [row.nonEmptyText(14), row.nonEmptyText(15), row.nonEmptyText(16)],
          fileNames: [row.nonEmptyText(2), row.nonEmptyText(3)])
      else { return }
      tracks.append(
        Track(
          id: id,
          title: row.nonEmptyText(1) ?? row.nonEmptyText(2) ?? "Untitled",
          artist: row.nonEmptyText(4) ?? row.nonEmptyText(5) ?? "",
          album: row.nonEmptyText(6) ?? row.nonEmptyText(7) ?? "",
          // rekordbox stores BPM in centi-BPM, but older rows hold whole BPM.
          bpm: row.int(8).map { $0 > 300 ? Float($0) / 100 : Float($0) },
          // Length is in seconds, not milliseconds.
          durationSeconds: row.int(9).map(Float.init),
          rating: row.int(10),
          tags: row.nonEmptyText(11),
          releaseDate: row.nonEmptyText(12) ?? row.nonEmptyText(13),
          filePath: filePath))
    }
    return tracks
  }

  /// rekordbox often stores the whole track path in a folder column rather than the
  /// directory the name suggests, so a value that already looks like a file is taken as one.
  private static func audioPath(folders: [String?], fileNames: [String?]) -> String? {
    guard let folder = folders.compactMap({ $0 }).first else { return nil }
    let url = URL(fileURLWithPath: folder)
    if !url.pathExtension.isEmpty { return folder }
    guard let fileName = fileNames.compactMap({ $0 }).first else { return nil }
    return url.appendingPathComponent(fileName).path
  }

  // MARK: Playlists

  private static func playlists(_ db: MasterDB) throws -> [Playlist] {
    var trackIDs: [String: [String]] = [:]
    try db.query(
      """
      SELECT PlaylistID, ContentID FROM djmdSongPlaylist
       WHERE rb_local_deleted = 0 ORDER BY PlaylistID, TrackNo
      """
    ) { row in
      guard let playlist = row.text(0), let content = row.text(1) else { return }
      trackIDs[playlist, default: []].append(content)
    }

    var playlists: [Playlist] = []
    try db.query(
      """
      SELECT ID, Name, ParentID, Attribute FROM djmdPlaylist
       WHERE rb_local_deleted = 0 ORDER BY Seq
      """
    ) { row in
      guard let id = row.text(0) else { return }
      playlists.append(
        Playlist(
          id: id, name: row.text(1) ?? "", parentId: row.text(2) ?? "",
          isFolder: row.int(3) == 1, trackIds: trackIDs[id] ?? []))
    }
    return ordered(playlists)
  }

  /// Flattens the playlist tree depth-first, so the browse list reads the way it does in
  /// rekordbox: a folder is immediately followed by what is inside it.
  private static func ordered(_ playlists: [Playlist]) -> [Playlist] {
    guard playlists.count > 1 else { return playlists }
    let known = Set(playlists.map(\.id))
    var children: [String: [Playlist]] = [:]
    for playlist in playlists {
      let parent = known.contains(playlist.parentId) ? playlist.parentId : ""
      children[parent, default: []].append(playlist)
    }
    var result: [Playlist] = []
    result.reserveCapacity(playlists.count)
    func visit(_ parent: String) {
      for playlist in children[parent] ?? [] {
        result.append(playlist)
        visit(playlist.id)
      }
    }
    visit("")
    return result
  }

  // MARK: Cues

  /// The cues rekordbox keeps in the database, which carry the colour and comment the user
  /// set. Older libraries have them only in the ANLZ files, hence the merge in `analysis`.
  private static func storedCues(_ db: MasterDB, contentID: String) throws -> [RekordboxCue] {
    var cues: [RekordboxCue] = []
    try db.query(
      "SELECT Cues FROM contentCue WHERE ContentID = ? AND rb_local_deleted = 0", [contentID]
    ) { row in
      guard let json = row.text(0), let data = json.data(using: .utf8),
        let entries = try? JSONDecoder().decode([StoredCue].self, from: data)
      else { return }
      for (index, entry) in entries.enumerated() {
        guard let time = UInt32(exactly: max(entry.inMsec, 0)) else { continue }
        let out = entry.outMsec.flatMap { UInt32(exactly: max($0, 0)) } ?? 0
        let loopTime = out > time ? out : 0
        cues.append(
          RekordboxCue(
            hotCue: UInt32(index + 1), timeMs: time, loopTimeMs: loopTime, isLoop: loopTime > 0,
            colorRgb: color(index: entry.colorTableIndex),
            comment: entry.comment.flatMap { $0.isEmpty ? nil : $0 }))
      }
    }
    return cues
  }

  private struct StoredCue: Decodable {
    var inMsec: Int
    var outMsec: Int?
    var colorTableIndex: Int?
    var comment: String?

    enum CodingKeys: String, CodingKey {
      case inMsec = "InMsec"
      case outMsec = "OutMsec"
      case colorTableIndex = "ColorTableIndex"
      case comment = "Comment"
    }
  }

  /// rekordbox's cue colour palette, indexed from one.
  private static func color(index: Int?) -> [UInt8]? {
    let palette: [[UInt8]] = [
      [0xCC, 0x00, 0x00], [0xCC, 0x44, 0x00], [0xCC, 0x88, 0x00], [0xCC, 0xCC, 0x00],
      [0x88, 0xCC, 0x00], [0x00, 0xCC, 0x00], [0x00, 0xCC, 0x88], [0x00, 0xCC, 0xCC],
      [0x00, 0x88, 0xCC], [0x00, 0x00, 0xCC], [0x88, 0x00, 0xCC], [0xCC, 0x00, 0xCC],
      [0xCC, 0x00, 0x88], [0xFF, 0xFF, 0xFF],
    ]
    guard let index, index >= 1, index <= palette.count else { return nil }
    return palette[index - 1]
  }

  /// Database and ANLZ cues describe the same points, so they are merged by position and the
  /// duplicates dropped — keeping the database's entry, which has the colour and comment.
  private static func merge(_ stored: [RekordboxCue], _ fromANLZ: [RekordboxCue])
    -> [RekordboxCue]
  {
    var seen = Set<String>()
    return (stored + fromANLZ)
      .sorted { ($0.timeMs, $0.hotCue) < ($1.timeMs, $1.hotCue) }
      .filter { seen.insert("\($0.timeMs)/\($0.loopTimeMs)/\($0.isLoop)").inserted }
  }

  // MARK: Location

  /// The newest `master.db` under ~/Library/Pioneer, which is what rekordbox is using when
  /// several versions are installed side by side.
  private static func defaultMasterDBPath() -> String? {
    let base = FileManager.default.homeDirectoryForCurrentUser
      .appendingPathComponent("Library/Pioneer")
    let directories =
      (try? FileManager.default.contentsOfDirectory(
        at: base, includingPropertiesForKeys: [.isDirectoryKey])) ?? []
    return
      directories
      .filter { $0.lastPathComponent.hasPrefix("rekordbox") }
      .map { $0.appendingPathComponent("master.db") }
      .compactMap { url -> (Date, String)? in
        guard let values = try? url.resourceValues(forKeys: [.contentModificationDateKey]),
          let modified = values.contentModificationDate
        else { return nil }
        return (modified, url.path)
      }
      .max { $0.0 < $1.0 }?.1
  }
}

enum LibraryError: Error, CustomStringConvertible {
  case load(String)

  var description: String {
    switch self {
    case .load(let message): message
    }
  }
}
