import SwiftUI
import UniformTypeIdentifiers

/// The rekordbox browse list: playlist picker and a sortable table. Right-click
/// or drag a row onto a deck to load it.
struct LibraryView: View {
  @Environment(ConsoleModel.self) private var model
  @State private var selectedPlaylist: String? = nil
  @State private var selection = Set<Track.ID>()
  @State private var sortOrder = [KeyPathComparator(\Track.title)]

  private var visibleTracks: [Track] {
    let library = model.library
    var tracks = library.tracks.filter(\.isLocalFile)
    if let id = selectedPlaylist, let playlist = library.playlists.first(where: { $0.id == id }) {
      let byId = Dictionary(uniqueKeysWithValues: tracks.map { ($0.id, $0) })
      tracks = playlist.trackIds.compactMap { byId[$0] }
    }
    return tracks.sorted(using: sortOrder)
  }

  private func depth(of playlist: Playlist) -> Int {
    var depth = 0
    var parent = playlist.parentId
    let playlists = model.library.playlists
    while let next = playlists.first(where: { $0.id == parent }) {
      depth += 1
      parent = next.parentId
      if depth > 16 { break }
    }
    return depth
  }

  var body: some View {
    let library = model.library
    VStack(alignment: .leading, spacing: 6) {
      HStack(spacing: 8) {
        Text("REKORDBOX LIBRARY")
          .font(Theme.pixel(12)).bold().foregroundStyle(Theme.cyan)
        if !library.sourceLabel.isEmpty {
          Text("• \(library.sourceLabel)")
            .font(Theme.pixel(11)).foregroundStyle(Theme.textDim)
            .lineLimit(1).truncationMode(.middle)
        }
        Spacer()
        Text("\(library.tracks.filter(\.isLocalFile).count) tracks")
          .font(Theme.pixel(10)).foregroundStyle(Theme.textDim)
      }

      Picker("Playlist", selection: $selectedPlaylist) {
        Text("Collection").tag(String?.none)
        ForEach(library.playlists.filter { !$0.isFolder }) { playlist in
          Text(String(repeating: "    ", count: depth(of: playlist)) + playlist.name)
            .tag(String?.some(playlist.id))
        }
      }
      .labelsHidden()
      .frame(width: 240)

      Divider()

      if library.tracks.isEmpty {
        Text(library.sourceLabel.isEmpty ? "No Rekordbox tracks found" : library.sourceLabel)
          .font(Theme.pixel(11)).foregroundStyle(Theme.textDim)
          .frame(maxWidth: .infinity, maxHeight: .infinity)
      } else {
        Table(of: Track.self, selection: $selection, sortOrder: $sortOrder) {
          TableColumn("TITLE", value: \.title)
          TableColumn("ARTIST", value: \.artist)
          TableColumn("ALBUM", value: \.album)
          TableColumn("BPM", value: \.bpmSortKey) { track in
            Text(track.bpm.map { String(format: "%.1f", $0) } ?? "—")
          }
          .width(60)
          TableColumn("TIME", value: \.durationSortKey) { track in
            Text(track.durationText)
          }
          .width(50)
          TableColumn("RATING", value: \.ratingSortKey) { track in
            Text(track.ratingStars)
          }
          .width(70)
          TableColumn("TAGS", value: \.tagsText)
          TableColumn("RELEASE", value: \.releaseText)
            .width(90)
        } rows: {
          ForEach(visibleTracks) { track in
            // Row drag: a file URL, so a deck's drop target and Finder both take it.
            TableRow(track).itemProvider {
              NSItemProvider(object: track.fileURL as NSURL)
            }
          }
        }
        .contextMenu(forSelectionType: Track.ID.self) { ids in
          if let id = ids.first, let track = library.tracks.first(where: { $0.id == id }) {
            Button("Load to Deck A") { model.loadFile(0, track.fileURL) }
            Button("Load to Deck B") { model.loadFile(1, track.fileURL) }
          }
        }
        .font(Theme.pixel(11))
      }
    }
    .padding(8)
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .background(Theme.gradient135(Theme.rgb(24, 24, 24), Theme.rgb(14, 14, 14)))
    .overlay(Rectangle().stroke(Theme.borderDim, lineWidth: 1))
  }
}

extension Track {
  var bpmSortKey: Float { bpm ?? -1 }
  var durationSortKey: Float { durationSeconds ?? -1 }
  var ratingSortKey: Int32 { rating ?? -1 }

  var durationText: String {
    guard let seconds = durationSeconds else { return "—" }
    let total = Int(max(seconds, 0))
    return String(format: "%d:%02d", total / 60, total % 60)
  }

  var ratingStars: String {
    let count = Int(min(max(rating ?? 0, 0), 5))
    return String(repeating: "★", count: count) + String(repeating: "☆", count: 5 - count)
  }

  var tagsText: String {
    let parts = (tags ?? "")
      .split(whereSeparator: { $0 == "," || $0 == ";" })
      .map { $0.trimmingCharacters(in: .whitespaces) }
      .filter { !$0.isEmpty }
      .prefix(3)
    return parts.isEmpty ? "#-" : "#" + parts.joined(separator: " #")
  }

  var releaseText: String {
    guard let value = releaseDate else { return "" }
    let digits = value.replacingOccurrences(of: "/", with: "-")
      .filter { $0.isNumber || $0 == "-" }
    return digits.count >= 10 ? String(digits.prefix(10)) : value
  }
}
