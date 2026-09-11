import SwiftUI

/// The centre column: master tempo and the mixer section (EQ kills, deck gain,
/// level meters, cue monitors).
struct MixerView: View {
  @Environment(ConsoleModel.self) private var model

  var body: some View {
    GroupBox {
      VStack(spacing: 12) {
        tempo
        Divider()
        // Two rows so the kills, faders and meters share one height, with the
        // gain readouts and cue monitors on a row of their own beneath.
        Grid(horizontalSpacing: 8, verticalSpacing: 6) {
          GridRow {
            EQKillColumn(index: 0)
            GainSlider(index: 0, gain: model.deck(0).gain)
            LevelMeter(peak: model.deck(0).peak, hold: model.peakHold[0])
              .frame(width: 10)
            LevelMeter(peak: model.deck(1).peak, hold: model.peakHold[1])
              .frame(width: 10)
            GainSlider(index: 1, gain: model.deck(1).gain)
            EQKillColumn(index: 1)
          }
          .frame(height: Self.columnHeight)
          GridRow {
            Color.clear.gridCellUnsizedAxes([.horizontal, .vertical])
            gainReadout(index: 0)
            CueButton(index: 0)
            CueButton(index: 1)
            gainReadout(index: 1)
            Color.clear.gridCellUnsizedAxes([.horizontal, .vertical])
          }
        }
        Spacer(minLength: 0)
      }
      .padding(4)
      // Fill the row so the box is as tall as the decks beside it.
      .frame(maxHeight: .infinity)
    } label: {
      Label("Mixer", systemImage: "slider.horizontal.3")
    }
  }

  private var tempo: some View {
    let binding = Binding<Int>(
      get: { Int(model.snapshot.master_tempo.rounded()) },
      set: { model.setMasterTempo(Float($0)) })
    return LabeledContent {
      Stepper(value: binding, in: 60...200) {
        Text("\(binding.wrappedValue) BPM")
          .font(.title3.monospacedDigit().weight(.semibold))
      }
    } label: {
      Text("Tempo")
    }
  }

  /// Shared height of the kills, faders and meters.
  static let columnHeight: CGFloat = 72

  private func gainReadout(index: Int) -> some View {
    Text("\(Int((model.deck(index).gain * 100).rounded()))%")
      .font(.caption2.monospacedDigit())
      .foregroundStyle(.secondary)
      .lineLimit(1)
      .fixedSize()
  }
}

/// Pioneer-style 15-segment LED meter, -24 dB to +13 dB with an +8 dB offset.
struct LevelMeter: View {
  let peak: Float
  let hold: Float

  private static func db(_ peak: Float) -> Float {
    guard peak > 0 else { return -.infinity }
    return min(20 * log10(peak) + 8, 13)
  }

  var body: some View {
    Canvas { context, size in
      let minDb: Float = -24
      let maxDb: Float = 13
      let segments = 15
      let segmentHeight = size.height / CGFloat(segments)
      let stepDb = (maxDb - minDb) / Float(segments - 1)
      let current = Self.db(peak)
      let held = Self.db(hold)
      for i in 0..<segments {
        let segmentDb = minDb + Float(i) * stepDb
        let color: Color =
          i >= 13 ? Theme.meterRed : (i >= 9 ? Theme.meterOrange : Theme.meterGreen)
        let lit = current >= segmentDb || (held >= segmentDb && held < segmentDb + stepDb)
        let y = size.height - CGFloat(i + 1) * segmentHeight
        context.fill(
          Path(CGRect(x: 0, y: y, width: size.width, height: segmentHeight - 1)),
          with: .color(lit ? color : color.opacity(0.15)))
      }
    }
  }
}

/// Deck gain as a standard slider, stood on end.
struct GainSlider: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int
  let gain: Float

  var body: some View {
    let binding = Binding<Double>(
      get: { Double(gain) },
      set: { model.setDeckGain(index, Float($0)) })
    Slider(value: binding, in: 0...1)
      .labelsHidden()
      .controlSize(.small)
      .frame(width: MixerView.columnHeight)
      .rotationEffect(.degrees(-90))
      .frame(width: 20, height: MixerView.columnHeight)
  }
}

struct EQKillColumn: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int

  var body: some View {
    let deck = model.deck(index)
    VStack(spacing: 0) {
      killButton("H", .high, active: deck.eq_high != 0)
      Spacer(minLength: 2)
      killButton("M", .mid, active: deck.eq_mid != 0)
      Spacer(minLength: 2)
      killButton("L", .low, active: deck.eq_low != 0)
    }
    .frame(height: MixerView.columnHeight)
  }

  private func killButton(_ label: String, _ band: EQBand, active: Bool) -> some View {
    ActiveButton(
      active: active, tint: Theme.kill, action: { model.setEQ(index, band, kill: !active) }
    ) {
      Text(label).font(.caption.weight(.semibold)).frame(width: 18)
    }
    .controlSize(.small)
    .help("Kill \(band)")
  }
}

extension EQBand: CustomStringConvertible {
  var description: String {
    switch self {
    case .low: "low"
    case .mid: "mid"
    case .high: "high"
    }
  }
}

struct CueButton: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int

  var body: some View {
    let enabled = model.deck(index).cue_enabled != 0
    ActiveButton(active: enabled, tint: Theme.cue, action: { model.toggleCue(index) }) {
      Image(systemName: "headphones").frame(width: 18)
    }
    .help("Cue monitor")
  }
}
