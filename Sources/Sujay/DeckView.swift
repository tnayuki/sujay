import SwiftUI
import UniformTypeIdentifiers

/// One deck panel: header, full waveform, rekordbox cues, loop pads. Deck A
/// (index 0) mirrors deck B so the console is symmetric around the tempo column.
struct DeckView: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int

  private var isLeft: Bool { index == 0 }

  var body: some View {
    let hasTrack = model.hasTrack(index)
    let text = model.deckText(index)
    let deck = model.deck(index)
    VStack(alignment: .leading, spacing: 0) {
      header(text: text, playing: deck.playing != 0)
      Spacer().frame(height: 5)
      FullWaveformView(index: index, height: 50)
      Spacer().frame(height: 5)
      cueRow(cues: text.cues, enabled: hasTrack)
      Spacer().frame(height: 10)
      loopPads(
        loopEnabled: deck.loop_enabled != 0, loopBeats: deck.loop_beats, enabled: deck.bpm > 0)
    }
    .padding(8)
    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
    .background(Theme.gradient135(Theme.panel, Theme.bgDark))
    .overlay(Rectangle().stroke(hasTrack ? Theme.cyan : Theme.borderDim, lineWidth: 1))
    .shadow(color: hasTrack ? Theme.cyan.opacity(0.35) : .clear, radius: 15)
    .dropDestination(for: URL.self) { urls, _ in
      guard let url = urls.first else { return false }
      model.loadFile(index, url)
      return true
    }
  }

  // MARK: Header

  @ViewBuilder
  private func header(text: DeckText, playing: Bool) -> some View {
    HStack(spacing: 10) {
      if isLeft {
        deckNumber("1")
        thumbnail
        info(text: text)
        playButton(playing: playing)
      } else {
        playButton(playing: playing)
        info(text: text)
        thumbnail
        deckNumber("2")
      }
    }
    .frame(height: 40)
  }

  private func deckNumber(_ number: String) -> some View {
    Text(number)
      .font(Theme.pixel(18))
      .foregroundStyle(Theme.cyan)
      .shadow(color: Theme.cyan.opacity(0.6), radius: 5)
      .frame(width: 18, height: 18)
  }

  private var thumbnail: some View {
    RoundedRectangle(cornerRadius: 4)
      .fill(Theme.gradient135(Theme.panel, Theme.bgDark))
      .overlay(RoundedRectangle(cornerRadius: 4).stroke(Theme.borderDim, lineWidth: 1))
      .overlay(Text("🎵").font(.system(size: 20)))
      .frame(width: 40, height: 40)
  }

  private func info(text: DeckText) -> some View {
    VStack(alignment: .leading, spacing: 3) {
      Text(model.hasTrack(index) ? text.title : "No track loaded")
        .font(Theme.pixel(13)).bold()
        .foregroundStyle(Theme.textPrimary)
        .lineLimit(1)
        .truncationMode(.tail)
      HStack(spacing: 4) {
        Text(model.timeText(index))
          .font(Theme.pixel(11)).foregroundStyle(Theme.cyan)
        if !text.bpmText.isEmpty {
          Text("• \(text.bpmText) BPM")
            .font(Theme.pixel(11)).foregroundStyle(Theme.orangeSec)
        }
      }
    }
    .frame(maxWidth: .infinity, alignment: .leading)
  }

  private func playButton(playing: Bool) -> some View {
    Button {
      model.togglePlay(index)
    } label: {
      Text(playing ? "■" : "▶")
        .font(.system(size: 16))
        .foregroundStyle(Theme.orangeSec)
        .frame(width: 32, height: 32)
    }
    .buttonStyle(ConsoleButtonStyle(cornerRadius: 4))
  }

  // MARK: Cues

  @ViewBuilder
  private func cueRow(cues: [CuePoint], enabled: Bool) -> some View {
    HStack(spacing: 3) {
      if isLeft { Spacer(minLength: 0) }
      if !cues.isEmpty {
        if isLeft { cueLabel }
        ForEach(cues.prefix(8)) { cue in
          cueButton(cue, enabled: enabled)
        }
        if !isLeft { cueLabel }
      }
      if !isLeft { Spacer(minLength: 0) }
    }
    .frame(height: 20)
  }

  private var cueLabel: some View {
    Text("CUE").font(Theme.pixel(9)).foregroundStyle(Theme.textDim)
  }

  private func cueButton(_ cue: CuePoint, enabled: Bool) -> some View {
    let color =
      cue.colorRgb.map { rgb in
        rgb.count == 3 ? Theme.rgb(Double(rgb[0]), Double(rgb[1]), Double(rgb[2])) : Theme.orangeSec
      } ?? Theme.orangeSec
    return Button {
      model.recallCue(index, cue)
    } label: {
      Text(cue.loopEnd != nil ? "L\(cue.label)" : cue.label)
        .font(Theme.pixel(9))
        .foregroundStyle(color)
        .frame(minWidth: 30)
        .frame(height: 20)
    }
    .buttonStyle(ConsoleButtonStyle())
    .disabled(!enabled)
    .opacity(enabled ? 1 : 0.5)
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
        HStack(spacing: 2) {
          ForEach(row, id: \.self) { beats in
            let active = loopEnabled && abs(loopBeats - beats) < 0.001
            LoopPad(
              label: beats >= 1 ? "\(Int(beats))" : "1/\(Int(1 / beats))",
              active: active, enabled: enabled
            ) {
              model.toggleLoop(index, beats: active ? 0 : beats)
            }
          }
        }
      }
    }
    .frame(maxWidth: .infinity, alignment: isLeft ? .trailing : .leading)
  }
}

private struct LoopPad: View {
  let label: String
  let active: Bool
  let enabled: Bool
  let action: () -> Void
  @State private var pulse = false

  var body: some View {
    Button(action: action) {
      Text(label)
        .font(Theme.pixel(9))
        .foregroundStyle(enabled ? (active ? .white : Theme.textDim) : Theme.textDim.opacity(0.55))
        .frame(width: 24, height: 22)
    }
    .buttonStyle(
      ConsoleButtonStyle(
        active: active, activeTop: Theme.green, activeBottom: Theme.rgb(0, 153, 68),
        activeBorder: Theme.green, glow: Theme.green, glowRadius: pulse ? 12 : 4)
    )
    .disabled(!enabled)
    .animation(
      active ? .easeInOut(duration: 0.5).repeatForever(autoreverses: true) : .default,
      value: pulse
    )
    .onChange(of: active, initial: true) { _, isActive in
      pulse = isActive
    }
  }
}
