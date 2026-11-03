//! The window: title bar, the sidebar of pages by category, the open page.
//!
//! A page is a group's generic page (page.rs), or a custom one standing for
//! several groups: Hashes for the five `*sum` commands (app.md).

use gpui::{
    AnyView, AppContext as _, Context, Entity, IntoElement, ParentElement, Render, Styled, Window,
    div,
};
use gpui_component::sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem};
use gpui_component::{ActiveTheme as _, TitleBar, h_flex, v_flex};
use toolwing_core::{Category, Group, Tool as _};
use toolwing_tools::{GROUPS, group, hash};

use crate::hashes::HashesPage;
use crate::page::ToolPage;

/// What the sidebar lists.
#[derive(Clone, Copy)]
pub enum Entry {
    Group(&'static Group),
    Hashes,
}

impl Entry {
    pub fn title(self) -> &'static str {
        match self {
            Entry::Group(g) => g.title,
            Entry::Hashes => "Hashes",
        }
    }

    fn category(self) -> Option<Category> {
        match self {
            Entry::Group(g) => group(g.name).first().map(|t| t.meta().category),
            Entry::Hashes => Some(Category::Crypto),
        }
    }
}

/// The pages in the registry's order: the hash groups as one, where the
/// first of them is.
pub fn entries() -> Vec<Entry> {
    let mut entries = Vec::new();
    for g in GROUPS {
        let is_sum = hash::SUMS.iter().any(|s| s.meta().group == g.name);
        if !is_sum {
            entries.push(Entry::Group(g));
        } else if !entries.iter().any(|e| matches!(e, Entry::Hashes)) {
            entries.push(Entry::Hashes);
        }
    }
    entries
}

/// The open page.
#[derive(Clone)]
pub enum Page {
    Tool(Entity<ToolPage>),
    Hashes(Entity<HashesPage>),
}

impl Page {
    fn view(&self) -> AnyView {
        match self {
            Page::Tool(p) => p.clone().into(),
            Page::Hashes(p) => p.clone().into(),
        }
    }
}

pub struct Workspace {
    entries: Vec<Entry>,
    /// Index into `entries`.
    selected: usize,
    page: Page,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let entries = entries();
        let page = open(entries[0], window, cx);
        Workspace {
            entries,
            selected: 0,
            page,
        }
    }

    /// Opens the page titled `title`, as clicking it in the sidebar would.
    pub fn select_title(&mut self, title: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.entries.iter().position(|e| e.title() == title) {
            self.select(i, window, cx);
        }
    }

    pub fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index == self.selected || index >= self.entries.len() {
            return;
        }
        self.selected = index;
        self.page = open(self.entries[index], window, cx);
        cx.notify();
    }

    pub fn page(&self) -> &Page {
        &self.page
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> Sidebar<SidebarGroup<SidebarMenu>> {
        let mut sidebar = Sidebar::new("tools").collapsible(false);
        for category in Category::ALL {
            let items: Vec<SidebarMenuItem> = self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.category() == Some(*category))
                .map(|(i, e)| {
                    SidebarMenuItem::new(e.title())
                        .active(i == self.selected)
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.select(i, window, cx)),
                        )
                })
                .collect();
            if !items.is_empty() {
                sidebar = sidebar.child(
                    SidebarGroup::new(category.title()).child(SidebarMenu::new().children(items)),
                );
            }
        }
        sidebar
    }
}

fn open(entry: Entry, window: &mut Window, cx: &mut Context<Workspace>) -> Page {
    match entry {
        Entry::Group(g) => Page::Tool(cx.new(|cx| ToolPage::new(g, window, cx))),
        Entry::Hashes => Page::Hashes(cx.new(|cx| HashesPage::new(window, cx))),
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(TitleBar::new().child(div().text_sm().child("toolwing")))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.sidebar(cx))
                    .child(div().flex_1().min_w_0().h_full().child(self.page.view())),
            )
    }
}
