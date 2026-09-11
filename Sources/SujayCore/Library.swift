import CSujay
import Foundation

/// The rekordbox reader (`crates/library`) through its C ABI. Both calls are
/// blocking and run off the main thread.
enum RekordboxReader {
  private static func decode<T: Decodable>(_ type: T.Type, _ pointer: UnsafeMutablePointer<CChar>?)
    throws -> T
  {
    guard let pointer else { throw LibraryError.load("no data") }
    defer { sujay_string_free(pointer) }
    let data = Data(bytes: pointer, count: strlen(pointer))
    if let failure = try? JSON.decoder.decode(ErrorPayload.self, from: data) {
      throw LibraryError.load(failure.error)
    }
    return try JSON.decoder.decode(type, from: data)
  }

  /// The browse list without per-track analysis; nil path = the newest
  /// `master.db` under ~/Library/Pioneer.
  static func loadLibrary(masterDB: String? = nil) throws -> Library {
    if let masterDB {
      return try masterDB.withCString { try decode(Library.self, sujay_library_load_json($0)) }
    }
    return try decode(Library.self, sujay_library_load_json(nil))
  }

  /// One track's beat grid, cues and 3-band waveform colours.
  static func analysis(masterDB: String, contentID: String) throws -> TrackAnalysis {
    try masterDB.withCString { db in
      try contentID.withCString { id in
        try decode(TrackAnalysis.self, sujay_library_track_analysis_json(db, id))
      }
    }
  }

  private struct ErrorPayload: Decodable {
    var error: String
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
