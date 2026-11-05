mod bench;
mod chat;
mod data;
mod ide;
mod theme;

use std::borrow::Cow;
use std::sync::Arc;

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::chat::ChatView;
use crate::data::Api;
use crate::ide::IdeView;
use crate::theme::*;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Ide,
    Chat,
}

struct Workbench {
    mode: Mode,
    ide: Entity<IdeView>,
    chat: Entity<ChatView>,
    printed_gpu: bool,
    focus: FocusHandle,
    animate: bool,
}

impl Workbench {
    fn tab(&self, label: &'static str, mode: Mode, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.mode == mode;
        div()
            .id(label)
            .px_3()
            .h(px(22.0))
            .flex()
            .items_center()
            .rounded_sm()
            .text_size(px(11.0))
            .text_color(if active { c(INK) } else { c(INK2) })
            .when(active, |d| d.bg(rgba(0xF4F4F61A)))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.mode = mode;
                if mode == Mode::Chat {
                    this.chat.update(cx, |chat, cx| chat.focus_composer(window, cx));
                }
                cx.notify();
            }))
            .child(label)
    }
}

impl Render for Workbench {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        bench::frame();
        if self.animate {
            window.request_animation_frame();
        }
        if !self.printed_gpu {
            self.printed_gpu = true;
            if let Some(specs) = window.gpu_specs() {
                eprintln!("gpu: {} / {} / software={}", specs.device_name, specs.driver_name, specs.is_software_emulated);
            }
        }
        let body = match self.mode {
            Mode::Ide => self.ide.clone().into_any_element(),
            Mode::Chat => self.chat.clone().into_any_element(),
        };
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if this.mode == Mode::Ide && ev.keystroke.key == "o" {
                    this.ide.update(cx, |ide, cx| ide.open_first_big(cx));
                }
            }))
            .bg(c(BG0))
            .text_color(c(INK))
            .child(
                h_flex()
                    .h(px(36.0))
                    .flex_none()
                    .px_3()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(border())
                    .child(div().text_size(px(13.0)).child("Forge"))
                    .child(h_flex().gap_1().child(self.tab("Chat", Mode::Chat, cx)).child(self.tab("IDE", Mode::Ide, cx))),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

fn main() {
    bench::init();
    bench::watch();
    let api = Arc::new(Api::from_env());
    let view = std::env::var("SPIKE_VIEW").unwrap_or_else(|_| "ide".into());
    let session = std::env::var("SPIKE_CHAT_SESSION").unwrap_or_default();
    let (w, h) = std::env::var("SPIKE_SIZE").ok().and_then(|v| v.split_once('x').map(|(a, b)| (a.parse().unwrap_or(1440.0), b.parse().unwrap_or(900.0)))).unwrap_or((1440.0, 900.0));

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../../../mobile/assets/Geist-Regular.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../../../mobile/assets/Geist-Medium.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../../../mobile/assets/GeistMono-Regular.ttf").as_slice()),
            ])
            .expect("fonts");
        theme::apply(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.0), px(0.0)), size(px(w), px(h))))),
            titlebar: None,
            ..Default::default()
        };
        let (window, wb) = gpui_kit::open_window(options, cx, |window, cx| {
            let ide = cx.new(|cx| IdeView::new(api.clone(), cx));
            let chat = cx.new(|cx| ChatView::new(api.clone(), session.clone(), window, cx));
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            cx.new(|_| Workbench { mode: if view == "chat" { Mode::Chat } else { Mode::Ide }, ide, chat, printed_gpu: false, focus, animate: std::env::var("SPIKE_ANIM").is_ok() })
        })
        .expect("window");
        let _ = (window, &wb);
        if std::env::var("SPIKE_OPEN_DIFF").is_ok() {
            let ide = wb.read(cx).ide.clone();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(1500)).await;
                ide.update(cx, |ide, cx| ide.open_first_big(cx));
            })
            .detach();
        }
        if let Ok(rate) = std::env::var("SPIKE_STREAM") {
            let chat = wb.read(cx).chat.clone();
            cx.spawn(async move |cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(3000)).await;
                chat.update(cx, |chat, cx| chat.start_stream(rate.parse().unwrap_or(50), 600, cx));
            })
            .detach();
        }
        cx.activate(true);
    });
}
