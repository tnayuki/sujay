import SwiftUI

struct CrossfaderView: View {
  @Environment(ConsoleModel.self) private var model

  var body: some View {
    let position = CGFloat(min(max(model.snapshot.crossfader, 0), 1))
    GeometryReader { geometry in
      let width = geometry.size.width
      ZStack(alignment: .leading) {
        LinearGradient(
          colors: [Theme.orangeSec, Theme.rgb(51, 51, 51), Theme.cyan],
          startPoint: .leading, endPoint: .trailing
        )
        .frame(height: 6)
        .clipShape(RoundedRectangle(cornerRadius: 3))
        .overlay(
          RoundedRectangle(cornerRadius: 3).stroke(Color.black.opacity(0.3), lineWidth: 0.5)
        )
        .frame(maxHeight: .infinity)

        RoundedRectangle(cornerRadius: 2)
          .fill(Theme.gradient135(Theme.borderMed, Theme.panel))
          .overlay(RoundedRectangle(cornerRadius: 2).stroke(Theme.rgb(119, 119, 119), lineWidth: 1))
          .frame(width: 10, height: 24)
          .offset(x: position * width - 5)
      }
      .contentShape(Rectangle())
      .gesture(
        DragGesture(minimumDistance: 0).onChanged { value in
          model.setCrossfader(Float(min(max(value.location.x / width, 0), 1)))
        })
    }
    .frame(height: 36)
  }
}
