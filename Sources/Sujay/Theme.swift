import SwiftUI

/// Console colours. Everything structural comes from the system appearance;
/// these are the few hues that carry meaning on a DJ console: which deck is
/// which, that a loop is running, how hot a meter is.
enum Theme {
  /// Deck A is warm, deck B is cool — the two ends of the crossfader.
  static let deckA = Color.orange
  static let deckB = Color.cyan
  static let loop = Color.green
  static let cue = Color.cyan
  static let kill = Color.orange
  static let meterGreen = Color.green
  static let meterOrange = Color.orange
  static let meterRed = Color.red
  static let waveformPlayed = Color.accentColor
  static let waveformUnplayed = Color.secondary
  static let beatMarker = Color.red.opacity(0.6)

  static func deckTint(_ index: Int) -> Color { index == 0 ? deckA : deckB }
}

/// A bordered button that turns prominent, in `tint`, while `active`.
struct ActiveButton<Label: View>: View {
  let active: Bool
  let tint: Color
  let action: () -> Void
  @ViewBuilder let label: () -> Label

  var body: some View {
    if active {
      Button(action: action, label: label)
        .buttonStyle(.borderedProminent)
        .tint(tint)
    } else {
      Button(action: action, label: label)
        .buttonStyle(.bordered)
    }
  }
}
