import SwiftUI

struct ContentView: View {
  @Environment(ConsoleModel.self) private var model

  var body: some View {
    VStack(spacing: 0) {
      TitlebarView()
      VStack(spacing: 0) {
        ZoomWaveformView(index: 0)
        Spacer().frame(height: 5)
        ZoomWaveformView(index: 1)
        Spacer().frame(height: 12)

        HStack(alignment: .top, spacing: 8) {
          DeckView(index: 0)
          TempoView()
            .frame(width: 200)
          DeckView(index: 1)
        }
        .frame(height: 176)

        Spacer().frame(height: 8)
        CrossfaderView()
        Spacer().frame(height: 8)
        LibraryView()
      }
      .padding(10)
      .background(Theme.gradient135(Theme.bgDark, Theme.bgDeep))
      .overlay(Rectangle().stroke(Theme.rgb(51, 51, 51), lineWidth: 1))
    }
    .background(Theme.bgDark)
    .font(Theme.pixel(12))
    .foregroundStyle(Theme.textPrimary)
  }
}
