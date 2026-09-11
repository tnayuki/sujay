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
    GroupBox {
      VStack(alignment: .leading, spacing: 8) {
        HStack {
          Picker("Playlist", selection: $selectedPlaylist) {
            Text("Collection").tag(String?.none)
            ForEach(library.playlists.filter { !$0.isFolder }) { playlist in
              Text(String(repeating: "    ", count: depth(of: playlist)) + playlist.name)
                .tag(String?.some(playlist.id))
            }
          }
          .frame(maxWidth: 280)
          Spacer()
          Text("\(visibleTracks.count) tracks")
            .font(.callout.monospacedDigit())
            .foregroundStyle(.secondary)
        }

        if library.tracks.isEmpty {
          ContentUnavailableView(
            library.sourceLabel.isEmpty ? "No rekordbox tracks" : library.sourceLabel,
            systemImage: "music.note.list")
        } else {
          Table(of: Track.self, selection: $selection, sortOrder: $sortOrder) {
            TableColumn("Title", value: \.title)
            TableColumn("Artist", value: \.artist)
            TableColumn("Album", value: \.album)
            TableColumn("BPM", value: \.bpmSortKey) { track in
              Text(track.bpm.map { String(format: "%.1f", $0) } ?? "—").monospacedDigit()
            }
            .width(60)
            TableColumn("Time", value: \.durationSortKey) { track in
              Text(track.durationText).monospacedDigit()
            }
            .width(50)
            TableColumn("Rating", value: \.ratingSortKey) { track in
              Text(track.ratingStars)
            }
            .width(70)
            TableColumn("Tags", value: \.tagsText)
            TableColumn("Release", value: \.releaseText)
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
        }
      }
      .padding(4)
    } label: {
      Label("Rekordbox Library", systemImage: "music.note.list")
    }
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
