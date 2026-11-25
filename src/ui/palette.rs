//! The command palette (Ctrl+K): a text box over a short list of commands. Typing narrows the
//! list, Up and Down move the highlight, Enter runs it. A date typed in works as "go to".

use gpui::prelude::*;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, MouseButton,
    Subscription, Window, div, px, rgb,
};
use jiff::Zoned;

use super::compose::{ComposeEvent, backdrop, hint, panel};
use super::text_input::{Submit, TextInput};
use super::theme::{CAPTION, SECONDARY, SELECTED_BORDER, TEXT};
use super::{PaletteDown, PaletteUp};
use crate::palette::{self, CalendarRef, Entry};
use crate::store::Store;

pub struct PaletteView {
    store: Entity<Store>,
    input: Entity<TextInput>,
    all: Vec<Entry>,
    selected: usize,
    _subscription: Subscription,
}

impl EventEmitter<ComposeEvent> for PaletteView {}

impl PaletteView {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| TextInput::new("Type a command, or a date to jump to", false, 1, cx));
        let subscription = cx.observe(&input, |this, _, cx| {
            this.selected = 0;
            cx.notify();
        });
        let all = {
            let s = store.read(cx);
            let sets: Vec<String> = s.prefs.calendar_sets.keys().cloned().collect();
            let calendars: Vec<CalendarRef> = s
                .calendars
                .iter()
                .map(|c| CalendarRef {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    visible: s.is_visible(c),
                })
                .collect();
            palette::entries(&sets, &calendars)
        };
        Self {
            store,
            input,
            all,
            selected: 0,
            _subscription: subscription,
        }
    }

    pub fn first_focus(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }

    fn matches(&self, cx: &App) -> Vec<Entry> {
        palette::search(self.input.read(cx).text(), &self.all, &Zoned::now())
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = self.matches(cx).len() as isize;
        if n > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
            cx.notify();
        }
    }

    fn run(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(entry) = self.matches(cx).into_iter().nth(index) {
            cx.emit(ComposeEvent::Run(entry.action));
        }
    }
}

impl Focusable for PaletteView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for PaletteView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = &self.store;
        let matches = self.matches(cx);
        let selected = self.selected.min(matches.len().saturating_sub(1));
        let empty = matches.is_empty();
        backdrop()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::Close)),
            )
            .child(
                panel()
                    .key_context("Palette")
                    .on_action(cx.listener(|this, _: &Submit, _, cx| {
                        let i = this.selected;
                        this.run(i, cx)
                    }))
                    .on_action(cx.listener(|this, _: &PaletteUp, _, cx| this.step(-1, cx)))
                    .on_action(cx.listener(|this, _: &PaletteDown, _, cx| this.step(1, cx)))
                    .child(self.input.clone())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .children(matches.into_iter().enumerate().map(|(i, entry)| {
                                div()
                                    .id(("palette-row", i))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_size(px(13.0))
                                    .text_color(rgb(TEXT))
                                    .when(i == selected, |d| {
                                        d.bg(rgb(SELECTED_BORDER)).font_weight(FontWeight::SEMIBOLD)
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| this.run(i, cx)))
                                    .child(entry.label)
                                    .child(
                                        div()
                                            .text_size(px(11.0))
                                            .text_color(rgb(if i == selected {
                                                SECONDARY
                                            } else {
                                                CAPTION
                                            }))
                                            .child(entry.detail),
                                    )
                            })),
                    )
                    .when(empty, |d| d.child(hint("No command or date matches")))
                    .child(hint("↑↓ to choose · Enter to run · Esc to close")),
            )
    }
}
