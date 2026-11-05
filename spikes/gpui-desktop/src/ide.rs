//! IDE review view: agents, the attention-ordered change queue and a virtualized diff.
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::diff::{Diff, DiffFile, DiffState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::bench;
use crate::data::{AgentChanges, Api, Session};
use crate::theme::*;

const ROW_H: f32 = 28.0;

pub struct IdeView {
    api: Arc<Api>,
    agents: Vec<Session>,
    groups: Vec<AgentChanges>,
    selected: Option<(String, String)>,
    diff: Option<Entity<DiffState>>,
    status: SharedString,
}

impl IdeView {
    pub fn new(api: Arc<Api>, cx: &mut Context<Self>) -> Self {
        let this = Self { api, agents: vec![], groups: vec![], selected: None, diff: None, status: "".into() };
        this.load(cx);
        this
    }

    fn load(&self, cx: &mut Context<Self>) {
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_spawn(async move {
                    let agents = api.sessions()?;
                    let root = agents.iter().find(|s| s.worktree.is_none()).or(agents.first()).map(|s| s.cwd.clone()).unwrap_or_default();
                    let mut groups = api.changes(&root)?;
                    // Same rule as changesFromRoute.ts: files no agent claimed go to the most recently active shared-tree session.
                    let orphans: Vec<_> = groups.iter().filter(|g| g.session_id == "unattributed").flat_map(|g| g.files.clone()).collect();
                    groups.retain(|g| g.session_id != "unattributed");
                    if let Some(holder) = agents.iter().filter(|s| s.worktree.is_none()).max_by_key(|s| s.last_activity) {
                        let owned: std::collections::HashSet<_> = groups.iter().flat_map(|g| g.files.iter().map(|f| f.path.clone())).collect();
                        let extra: Vec<_> = orphans.into_iter().filter(|f| !owned.contains(&f.path)).collect();
                        match groups.iter_mut().find(|g| g.session_id == holder.id) {
                            Some(g) => g.files.extend(extra),
                            None => groups.push(AgentChanges { session_id: holder.id.clone(), title: holder.title.clone().unwrap_or_default(), waiting: holder.waiting, busy: holder.busy, files: extra }),
                        }
                    }
                    groups.retain(|g| !g.files.is_empty());
                    groups.sort_by_key(|g| (!g.waiting, !g.busy, std::cmp::Reverse(g.files.len())));
                    anyhow::Ok((agents, groups))
                })
                .await;
            if let Ok((mut agents, groups)) = loaded {
                agents.sort_by_key(|a| (!a.waiting, !a.busy, std::cmp::Reverse(a.last_activity)));
                this.update(cx, |this, cx| {
                    this.agents = agents;
                    this.groups = groups;
                    cx.notify();
                    bench::event("data_loaded");
                })
                .ok();
            }
        })
        .detach();
    }

    pub fn open_first_big(&mut self, cx: &mut Context<Self>) {
        let hit = self.groups.iter().flat_map(|g| g.files.iter().map(move |f| (g.session_id.clone(), f))).max_by_key(|(_, f)| f.adds);
        if let Some((session, file)) = hit {
            self.open(session, file.path.clone(), cx);
        }
    }

    fn open(&mut self, session: String, path: String, cx: &mut Context<Self>) {
        self.selected = Some((session.clone(), path.clone()));
        self.status = "loading diff...".into();
        cx.notify();
        let api = self.api.clone();
        let started = Instant::now();
        cx.spawn(async move |this, cx| {
            let p = path.clone();
            let patch = cx.background_spawn(async move { api.diff_patch(&session, &p) }).await;
            let fetched = started.elapsed();
            this.update(cx, |this, cx| {
                match patch.map_err(|e| e.to_string()).and_then(|p| DiffFile::parse(&p).map_err(|e| e.to_string())) {
                    Ok(files) => {
                        let parsed = started.elapsed();
                        this.diff = Some(cx.new(|cx| DiffState::new(files, cx)));
                        this.status = "".into();
                        bench::event(&format!("diff_fetch_ms={:.1}", fetched.as_secs_f64() * 1000.0));
                        bench::event(&format!("diff_fetch_parse_ms={:.1}", parsed.as_secs_f64() * 1000.0));
                        bench::event("diff_open");
                    }
                    Err(e) => this.status = e.into(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn agent_row(&self, a: &Session) -> impl IntoElement {
        let dot = if a.waiting { c(ACCENT) } else if a.busy { c(SUCCESS) } else { c(INK4) };
        h_flex()
            .h(px(ROW_H))
            .px_3()
            .gap_2()
            .items_center()
            .child(div().size(px(7.0)).rounded_full().bg(dot))
            .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(c(INK)).text_size(px(13.0)).child(a.title.clone().unwrap_or_default()))
            .child(if a.waiting {
                div().text_size(px(11.0)).text_color(c(ACCENT)).child("Needs you").into_any_element()
            } else {
                h_flex()
                    .gap_1()
                    .text_size(px(11.0))
                    .child(div().text_color(c(SUCCESS)).child(format!("+{}", a.diff_added.unwrap_or(0))))
                    .child(div().text_color(c(DANGER)).child(format!("-{}", a.diff_removed.unwrap_or(0))))
                    .into_any_element()
            })
    }

    fn group(&self, g: &AgentChanges, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = v_flex().child(
            v_flex().px_3().py_2().gap_0p5().child(
                div().text_size(px(13.0)).text_color(c(INK)).child(g.title.clone()),
            ).child(
                div()
                    .text_size(px(11.0))
                    .text_color(if g.waiting { c(ACCENT) } else { c(INK3) })
                    .child(if g.waiting { format!("Waiting for you · {} files", g.files.len()) } else { format!("{} to review · {} files", g.files.len(), g.files.len()) }),
            ),
        );
        for f in &g.files {
            let (session, path) = (g.session_id.clone(), f.path.clone());
            let selected = self.selected.as_ref() == Some(&(session.clone(), path.clone()));
            let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_string();
            let dir = f.path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
            col = col.child(
                h_flex()
                    .id(SharedString::from(format!("{}:{}", g.session_id, f.path)))
                    .h(px(ROW_H))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .when(selected, |d| d.bg(rgba(0xF4F4F613)))
                    .hover(|d| d.bg(rgba(0xF4F4F60B)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.open(session.clone(), path.clone(), cx)))
                    .child(div().text_size(px(13.0)).text_color(c(INK)).child(name))
                    .child(div().flex_1().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(11.0)).text_color(c(INK3)).child(dir))
                    .child(div().text_size(px(11.0)).text_color(c(SUCCESS)).child(format!("+{}", f.adds)))
                    .child(div().text_size(px(11.0)).text_color(c(DANGER)).child(format!("-{}", f.dels))),
            );
        }
        col
    }
}

fn section(label: &'static str, right: String) -> impl IntoElement {
    h_flex()
        .h(px(26.0))
        .px_3()
        .items_center()
        .justify_between()
        .text_size(px(10.0))
        .text_color(c(INK3))
        .child(label.to_uppercase())
        .child(right)
}

impl Render for IdeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let waiting = self.agents.iter().filter(|a| a.waiting).count();
        let agents: Vec<_> = self.agents.iter().map(|a| self.agent_row(a).into_any_element()).collect();
        let groups: Vec<_> = self.groups.clone().iter().map(|g| self.group(g, cx).into_any_element()).collect();
        let total: usize = self.groups.iter().map(|g| g.files.len()).sum();

        let rail = v_flex()
            .w(px(260.0))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(border())
            .child(section("Agents", if waiting > 0 { format!("{waiting} waiting") } else { self.agents.len().to_string() }))
            .children(agents)
            .child(div().h(px(1.0)).bg(border()).my_1())
            .child(section("Changes", total.to_string()))
            .child(div().id("queue").flex_1().min_h_0().overflow_y_scroll().children(groups));

        let name = self.selected.as_ref().map(|(_, p)| p.rsplit('/').next().unwrap_or(p).to_string());
        let center = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                h_flex()
                    .h(px(34.0))
                    .bg(c(BG1))
                    .border_b_1()
                    .border_color(border())
                    .items_center()
                    .px_3()
                    .gap_2()
                    .text_size(px(12.0))
                    .child(div().text_color(c(INK)).child(name.clone().unwrap_or_default()))
                    .child(div().text_color(c(INK4)).child(if name.is_some() { "diff" } else { "" })),
            )
            .child(match &self.diff {
                Some(state) => div().flex_1().min_h_0().child(Diff::new(state).size_full().into_any_element()).into_any_element(),
                None => div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(c(INK3))
                    .text_size(px(13.0))
                    .child(if self.status.is_empty() { SharedString::from(format!("{total} files to review")) } else { self.status.clone() })
                    .into_any_element(),
            });
        h_flex().size_full().bg(c(BG0)).child(rail).child(center)
    }
}
