import CSQLCipher
import Foundation

/// A read-only SQLCipher connection to rekordbox's `master.db`.
///
/// Read-only is not a precaution, it is the contract: sujay browses the library that
/// rekordbox owns and never writes back to it. Opening with `SQLITE_OPEN_READONLY` means a
/// bug here cannot corrupt a DJ's collection, and it keeps sujay out of the way of a
/// running rekordbox.
final class MasterDB {
  private let handle: OpaquePointer

  /// rekordbox encrypts `master.db`, and the passphrase is a constant shipped by every
  /// tool that reads the format. It is stored here XORed byte-wise so that neither this
  /// file nor the built binary contains the passphrase as a greppable string — that is the
  /// whole of the protection it offers, and the same gesture `rbox` and `pyrekordbox` make.
  private static let obfuscatedKey: [UInt8] = [
    0x6F, 0x6B, 0x69, 0x3D, 0x3F, 0x6F, 0x63, 0x69, 0x38, 0x68, 0x63, 0x63, 0x6A, 0x6C, 0x38,
    0x68, 0x6E, 0x3D, 0x3D, 0x3A, 0x63, 0x3D, 0x3D, 0x39, 0x63, 0x38, 0x6C, 0x3F, 0x62, 0x68,
    0x6A, 0x6F, 0x68, 0x39, 0x6C, 0x6F, 0x62, 0x3E, 0x6C, 0x3F, 0x68, 0x6A, 0x6E, 0x3F, 0x3D,
    0x6C, 0x3A, 0x63, 0x6A, 0x6C, 0x68, 0x69, 0x3A, 0x6A, 0x3D, 0x3D, 0x6F, 0x68, 0x6D, 0x6B,
    0x63, 0x6F, 0x62, 0x6C,
  ]
  private static let keyMask: UInt8 = 0x5B

  init(path: String) throws {
    var db: OpaquePointer?
    guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK, let db else {
      let message = db.map { String(cString: sqlite3_errmsg($0)) } ?? "cannot open"
      sqlite3_close(db)
      throw LibraryError.load("\(path): \(message)")
    }
    handle = db

    let key = String(decoding: Self.obfuscatedKey.map { $0 ^ Self.keyMask }, as: UTF8.self)
    // A wrong passphrase is not reported here — SQLCipher only fails once it has to read a
    // page — so the first query is what actually proves the database opened.
    try exec("PRAGMA key = '\(key)'")
    try exec("PRAGMA busy_timeout = 5000")
  }

  deinit { sqlite3_close(handle) }

  private func exec(_ sql: String) throws {
    var error: UnsafeMutablePointer<CChar>?
    guard sqlite3_exec(handle, sql, nil, nil, &error) == SQLITE_OK else {
      let message = error.map { String(cString: $0) } ?? "unknown error"
      sqlite3_free(error)
      throw LibraryError.load(message)
    }
  }

  /// Runs `sql` and hands each row to `row`. Values are read through `Row`, which is only
  /// valid for the duration of the call.
  func query(_ sql: String, _ parameters: [String] = [], _ row: (Row) -> Void) throws {
    var statement: OpaquePointer?
    guard sqlite3_prepare_v2(handle, sql, -1, &statement, nil) == SQLITE_OK, let statement else {
      throw LibraryError.load(String(cString: sqlite3_errmsg(handle)))
    }
    defer { sqlite3_finalize(statement) }
    for (index, parameter) in parameters.enumerated() {
      sqlite3_bind_text(statement, Int32(index + 1), parameter, -1, Self.transient)
    }
    while true {
      switch sqlite3_step(statement) {
      case SQLITE_ROW: row(Row(statement: statement))
      case SQLITE_DONE: return
      default: throw LibraryError.load(String(cString: sqlite3_errmsg(handle)))
      }
    }
  }

  /// SQLITE_TRANSIENT: the bound Swift string does not outlive the bind call.
  private static let transient = unsafeBitCast(
    -1, to: (@convention(c) (UnsafeMutableRawPointer?) -> Void).self)

  /// One row of a result set, valid only inside the `query` callback.
  struct Row {
    let statement: OpaquePointer

    func text(_ index: Int32) -> String? {
      guard let value = sqlite3_column_text(statement, index) else { return nil }
      return String(cString: value)
    }

    /// Empty strings are rekordbox's way of saying "unset" in most text columns, so they
    /// are folded into nil here rather than at every call site.
    func nonEmptyText(_ index: Int32) -> String? {
      guard let value = text(index), !value.isEmpty else { return nil }
      return value
    }

    func int(_ index: Int32) -> Int32? {
      guard sqlite3_column_type(statement, index) != SQLITE_NULL else { return nil }
      return sqlite3_column_int(statement, index)
    }
  }
}
