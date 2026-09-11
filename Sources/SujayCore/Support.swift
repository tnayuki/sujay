import Foundation

enum EQBand: UInt8, CustomStringConvertible {
  case low = 0
  case mid = 1
  case high = 2

  var description: String {
    switch self {
    case .low: "low"
    case .mid: "mid"
    case .high: "high"
    }
  }
}

enum RecordingFormat: String, Codable, CaseIterable {
  case wav
  case m4a

  var fileExtension: String { rawValue }
}

/// Shared JSON coding for everything that crosses from Rust and for the
/// settings file (snake_case keys).
enum JSON {
  static let decoder: JSONDecoder = {
    let decoder = JSONDecoder()
    decoder.keyDecodingStrategy = .convertFromSnakeCase
    return decoder
  }()

  static let encoder: JSONEncoder = {
    let encoder = JSONEncoder()
    encoder.keyEncodingStrategy = .convertToSnakeCase
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    return encoder
  }()
}
