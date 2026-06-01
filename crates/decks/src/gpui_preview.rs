use gpui::{Context, Render, Window, div, prelude::*, px, rgb};

struct GpuiPreview;

impl Render for GpuiPreview {
  fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
    div()
      .size_full()
      .flex()
      .flex_col()
      .justify_center()
      .items_center()
      .gap_4()
      .bg(rgb(0x0b0d10))
      .text_color(rgb(0xd9dde3))
      .child(
        div()
          .text_2xl()
          .font_weight(gpui::FontWeight::BOLD)
          .child("Sujay GPUI Preview"),
      )
      .child(
        div()
          .text_sm()
          .text_color(rgb(0x00d4ff))
          .child("GPUI backend initialized successfully"),
      )
      .child(
        div()
          .w(px(680.0))
          .h(px(140.0))
          .rounded_md()
          .bg(rgb(0x14181e))
          .border_1()
          .border_color(rgb(0x2f3640))
          .shadow_lg(),
      )
  }
}

pub fn launch_preview() -> bool {
  let _ = GpuiPreview;
  eprintln!(
    "[native-ui][gpui] preview disabled: running gpui in-process requires main-thread ownership and a dedicated app event loop"
  );
  false
}
