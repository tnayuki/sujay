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
        HStack(alignment: .top, spacing: 8) {
          EQKillColumn(index: 0)
          gainColumn(index: 0)
          meterColumn(index: 0)
          meterColumn(index: 1)
          gainColumn(index: 1)
          EQKillColumn(index: 1)
        }
      }
      .padding(4)
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

  private func gainColumn(index: Int) -> some View {
    let gain = model.deck(index).gain
    return VStack(spacing: 4) {
      GainSlider(index: index, gain: gain)
      Text("\(Int((gain * 100).rounded()))%")
        .font(.caption2.monospacedDigit())
        .foregroundStyle(.secondary)
        .lineLimit(1)
        .fixedSize()
    }
    .frame(width: 32)
  }

  private func meterColumn(index: Int) -> some View {
    VStack(spacing: 6) {
      LevelMeter(peak: model.deck(index).peak, hold: model.peakHold[index])
        .frame(width: 10, height: 64)
      CueButton(index: index)
    }
    .frame(width: 32)
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
      .frame(width: 64)
      .rotationEffect(.degrees(-90))
      .frame(width: 20, height: 64)
  }
}

struct EQKillColumn: View {
  @Environment(ConsoleModel.self) private var model
  let index: Int

  var body: some View {
    let deck = model.deck(index)
    VStack(spacing: 4) {
      killButton("H", .high, active: deck.eq_high != 0)
      killButton("M", .mid, active: deck.eq_mid != 0)
      killButton("L", .low, active: deck.eq_low != 0)
    }
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
