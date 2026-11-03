//! The window: title bar, the sidebar of tools by category, the open tool's page.

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div,
};
use gpui_component::sidebar::{Sidebar, SidebarGroup, SidebarMenu, SidebarMenuItem};
use gpui_component::{ActiveTheme as _, TitleBar, h_flex, v_flex};
use toolwing_core::Category;
use toolwing_tools::{GROUPS, group};

use crate::page::ToolPage;

pub struct Workspace {
    /// Index into `GROUPS`.
    selected: usize,
    page: Entity<ToolPage>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let page = cx.new(|cx| ToolPage::new(&GROUPS[0], window, cx));
        Workspace { selected: 0, page }
    }

    /// Opens the page of `GROUPS[index]`.
    pub fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index == self.selected {
            return;
        }
        self.selected = index;
        self.page = cx.new(|cx| ToolPage::new(&GROUPS[index], window, cx));
        cx.notify();
    }

    pub fn page(&self) -> &Entity<ToolPage> {
        &self.page
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> Sidebar<SidebarGroup<SidebarMenu>> {
        let mut sidebar = Sidebar::new("tools").collapsible(false);
        for category in Category::ALL {
            let items: Vec<SidebarMenuItem> = GROUPS
                .iter()
                .enumerate()
                .filter(|(_, g)| {
                    group(g.name)
                        .first()
                        .is_some_and(|t| t.meta().category == *category)
                })
                .map(|(i, g)| {
                    SidebarMenuItem::new(g.title)
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
                    .child(div().flex_1().min_w_0().h_full().child(self.page.clone())),
            )
    }
}
