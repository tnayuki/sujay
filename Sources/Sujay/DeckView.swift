import SwiftUI
import UniformTypeIdentifiers

/// One deck: header, full waveform, rekordbox cues, loop pads. Deck A
/// (index 0) mirrors deck B so the console is symmetric around the mixer.
struct DeckView: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int

  private var isLeft: Bool { index == 0 }
  private var tint: Color { Theme.deckTint(index) }

  var body: some View {
    let hasTrack = model.hasTrack(index)
    let text = model.deckText(index)
    let deck = model.deck(index)
    GroupBox {
      VStack(alignment: .leading, spacing: 8) {
        header(text: text, playing: deck.playing != 0, hasTrack: hasTrack)
        FullWaveformView(index: index, height: 50)
        cueRow(cues: text.cues, enabled: hasTrack)
        loopPads(
          loopEnabled: deck.loop_enabled != 0, loopBeats: deck.loop_beats, enabled: deck.bpm > 0)
      }
      .padding(4)
      .frame(maxHeight: .infinity, alignment: .top)
    } label: {
      Label(isLeft ? "Deck A" : "Deck B", systemImage: isLeft ? "a.circle.fill" : "b.circle.fill")
        .foregroundStyle(tint)
    }
    .dropDestination(for: URL.self) { urls, _ in
      guard let url = urls.first else { return false }
      model.loadFile(index, url)
      return true
    }
  }

  // MARK: Header

  @ViewBuilder
  private func header(text: DeckText, playing: Bool, hasTrack: Bool) -> some View {
    HStack(spacing: 10) {
      if isLeft {
        thumbnail
        info(text: text, hasTrack: hasTrack)
        playButton(playing: playing, enabled: hasTrack)
      } else {
        playButton(playing: playing, enabled: hasTrack)
        info(text: text, hasTrack: hasTrack)
        thumbnail
      }
    }
  }

  private var thumbnail: some View {
    Image(systemName: "music.note")
      .font(.title2)
      .foregroundStyle(.secondary)
      .frame(width: 40, height: 40)
      .background(.quaternary, in: RoundedRectangle(cornerRadius: 6))
  }

  private func info(text: DeckText, hasTrack: Bool) -> some View {
    VStack(alignment: isLeft ? .leading : .trailing, spacing: 2) {
      Text(hasTrack ? text.title : "No track loaded")
        .font(.headline)
        .foregroundStyle(hasTrack ? .primary : .secondary)
        .lineLimit(1)
        .truncationMode(.tail)
      HStack(spacing: 6) {
        Text(model.timeText(index)).monospacedDigit()
        if !text.bpmText.isEmpty {
          Text("\(text.bpmText) BPM").monospacedDigit()
        }
      }
      .font(.subheadline)
      .foregroundStyle(.secondary)
    }
    .frame(maxWidth: .infinity, alignment: isLeft ? .leading : .trailing)
  }

  private func playButton(playing: Bool, enabled: Bool) -> some View {
    Button {
      model.togglePlay(index)
    } label: {
      Image(systemName: playing ? "stop.fill" : "play.fill")
        .font(.title3)
        .frame(width: 28, height: 28)
    }
    .buttonStyle(.bordered)
    .tint(tint)
    .disabled(!enabled)
    .keyboardShortcut(isLeft ? "q" : "p", modifiers: [])
  }

  // MARK: Cues

  @ViewBuilder
  private func cueRow(cues: [CuePoint], enabled: Bool) -> some View {
    HStack(spacing: 4) {
      if isLeft { Spacer(minLength: 0) }
      if cues.isEmpty {
        Text("No cues").font(.caption).foregroundStyle(.tertiary)
      } else {
        ForEach(cues.prefix(8)) { cue in
          cueButton(cue, enabled: enabled)
        }
      }
      if !isLeft { Spacer(minLength: 0) }
    }
    .frame(height: 22)
  }

  private func cueButton(_ cue: CuePoint, enabled: Bool) -> some View {
    let color =
      cue.colorRgb.map { rgb in
        rgb.count == 3
          ? Color(
            red: Double(rgb[0]) / 255, green: Double(rgb[1]) / 255, blue: Double(rgb[2]) / 255)
          : Theme.cue
      } ?? Theme.cue
    return Button {
      model.recallCue(index, cue)
    } label: {
      Text(cue.loopEnd != nil ? "L\(cue.label)" : cue.label)
        .font(.caption.weight(.semibold))
        .frame(minWidth: 22)
    }
    .buttonStyle(.bordered)
    .tint(color)
    .controlSize(.small)
    .disabled(!enabled)
  }

  // MARK: Loops

  private static let loopRows: [[Float]] = [
    [1.0 / 16, 1.0 / 8, 1.0 / 4, 1.0 / 2, 1],
    [2, 4, 8, 16, 32],
  ]

  @ViewBuilder
  private func loopPads(loopEnabled: Bool, loopBeats: Float, enabled: Bool) -> some View {
    VStack(alignment: isLeft ? .trailing : .leading, spacing: 4) {
      ForEach(Self.loopRows, id: \.self) { row in
        HStack(spacing: 4) {
          ForEach(row, id: \.self) { beats in
            let active = loopEnabled && abs(loopBeats - beats) < 0.001
            ActiveButton(
              active: active, tint: Theme.loop,
              action: { model.toggleLoop(index, beats: active ? 0 : beats) }
            ) {
              Text(beats >= 1 ? "\(Int(beats))" : "1/\(Int(1 / beats))")
                .font(.caption.monospacedDigit())
                .frame(width: 26)
            }
            .controlSize(.small)
            .disabled(!enabled)
          }
        }
      }
    }
    .frame(maxWidth: .infinity, alignment: isLeft ? .trailing : .leading)
  }
}
