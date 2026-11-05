//! Chat view: a virtualized markdown transcript, a composer, and simulated token streaming.
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::bench;
use crate::data::Api;
use crate::theme::*;

pub struct Msg {
    user: bool,
    text: SharedString,
}

const STREAM_TEXT: &str = "## Streaming reply\n\nThe matching engine keeps the **hot loop** allocation-free. Here is the shape of it:\n\n```rust\nfn match_orders(book: &mut Book, incoming: Order) -> Vec<Fill> {\n    let mut fills = Vec::new();\n    while let Some(best) = book.best_opposite(&incoming) {\n        if !incoming.crosses(best.price) { break; }\n        let qty = incoming.remaining().min(best.remaining());\n        fills.push(Fill::new(best.id, incoming.id, best.price, qty));\n    }\n    fills\n}\n```\n\nAnd the checklist for the sequencer hand-off:\n\n- reserve the fill vector up front and reuse it between calls\n- assert price-time priority only in `debug_assertions` builds\n- never reorder the sequencing writes, even when the fills are empty\n\nShort answer: yes, the p99 holds under ten thousand levels of depth, as long as snapshots are taken off the hot path. ";

pub struct ChatView {
    msgs: Rc<RefCell<Vec<Msg>>>,
    scroller: Entity<MessageScrollerState>,
    composer: Entity<TextareaState>,
    streaming: bool,
}

impl ChatView {
    pub fn new(api: Arc<Api>, session: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let msgs = Rc::new(RefCell::new(Vec::<Msg>::new()));
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let composer = cx.new(|cx| TextareaState::new(window, cx).auto_grow(1, 8));
        cx.subscribe_in(&composer, window, |this: &mut Self, state, ev: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { shift: false, .. } = ev {
                let text = state.read(cx).value();
                if !text.trim().is_empty() {
                    state.update(cx, |s, cx| s.set_value("", window, cx));
                    this.push(true, text.trim().to_string().into(), cx);
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let rows = cx.background_spawn(async move { api.history(&session) }).await;
            if let Ok(rows) = rows {
                this.update(cx, |this, cx| {
                    let n = rows.len();
                    *this.msgs.borrow_mut() = rows.into_iter().map(|r| Msg { user: r.role == "user", text: r.content.into() }).collect();
                    this.scroller.update(cx, |s, cx| s.reset(n, cx));
                    cx.notify();
                    bench::event("chat_loaded");
                })
                .ok();
            }
        })
        .detach();
        Self { msgs, scroller, composer, streaming: false }
    }

    pub fn focus_composer(&self, window: &mut Window, cx: &mut App) {
        self.composer.focus_handle(cx).focus(window, cx);
    }

    fn push(&mut self, user: bool, text: SharedString, cx: &mut Context<Self>) {
        self.msgs.borrow_mut().push(Msg { user, text });
        self.scroller.update(cx, |s, cx| {
            s.append(1, cx);
            s.scroll_to_end(cx);
        });
        cx.notify();
    }

    pub fn start_stream(&mut self, tokens_per_sec: u64, total: usize, cx: &mut Context<Self>) {
        if self.streaming {
            return;
        }
        self.streaming = true;
        self.push(false, "".into(), cx);
        bench::event("stream_start");
        let words: Vec<&'static str> = STREAM_TEXT.split_inclusive(' ').collect();
        cx.spawn(async move |this, cx| {
            let mut text = String::new();
            for i in 0..total {
                cx.background_executor().timer(Duration::from_millis(1000 / tokens_per_sec)).await;
                text.push_str(words[i % words.len()]);
                let snapshot: SharedString = text.clone().into();
                let alive = this
                    .update(cx, |this, cx| {
                        let t0 = Instant::now();
                        let last = this.msgs.borrow().len() - 1;
                        this.msgs.borrow_mut()[last].text = snapshot;
                        this.scroller.update(cx, |s, cx| {
                            s.remeasure_items(last..last + 1, cx);
                        });
                        cx.notify();
                        bench::event(&format!("tick_ms={:.3}", t0.elapsed().as_secs_f64() * 1000.0));
                    })
                    .is_ok();
                if !alive {
                    return;
                }
            }
            this.update(cx, |this, _| {
                this.streaming = false;
                bench::event("stream_end");
            })
            .ok();
        })
        .detach();
    }
}

impl Render for ChatView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let msgs = self.msgs.clone();
        let list = MessageScroller::new("transcript", self.scroller.clone(), move |ix, _, _| {
            let msgs = msgs.borrow();
            let Some(m) = msgs.get(ix) else { return div().into_any_element() };
            if m.user {
                h_flex()
                    .w_full()
                    .justify_end()
                    .px_6()
                    .py_2()
                    .child(
                        div()
                            .max_w(px(620.0))
                            .px_3()
                            .py_2()
                            .rounded_lg()
                            .bg(c(BG3))
                            .text_size(px(13.0))
                            .text_color(c(INK))
                            .child(m.text.clone()),
                    )
                    .into_any_element()
            } else {
                div()
                    .w_full()
                    .px_6()
                    .py_2()
                    .child(div().max_w(px(760.0)).text_size(px(13.0)).child(TextView::markdown(("msg", ix), m.text.clone())))
                    .into_any_element()
            }
        });
        let _ = cx;
        v_flex()
            .size_full()
            .bg(c(BG0))
            .child(div().flex_1().min_h_0().child(list.size_full()))
            .child(
                div()
                    .p_3()
                    .border_t_1()
                    .border_color(border())
                    .child(Textarea::new(&self.composer)),
            )
    }
}
