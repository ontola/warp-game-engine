//! Declarative F10 pages exported by a Luau game's entry module.
use render::debug_ui::{DebugControl, DebugMenu, DebugValue};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
pub(crate) struct Page {
    id: u32,
    label: String,
    parent: Option<u32>,
    #[serde(default)]
    help: String,
    #[serde(default)]
    buttons: Vec<Button>,
}

#[derive(Deserialize)]
struct Button {
    id: u32,
    label: String,
    #[serde(default)]
    help: String,
}

pub(crate) fn build(pages: Vec<Page>) -> Result<(Vec<DebugMenu>, Vec<DebugControl>), String> {
    let mut menus = app::editor::EditorMenus::default();
    let mut parents = HashSet::new();
    let mut actions = HashSet::new();
    for page in pages {
        if page.parent.is_some_and(|p| !parents.contains(&p)) {
            return Err(format!("declare the parent of page {} first", page.id));
        }
        if page.id < app::debug::GAME_IDS || !parents.insert(page.id) {
            return Err(format!("invalid or duplicate page ID {}", page.id));
        }
        menus.page(page.id, page.parent, &page.label, &page.help);
        for button in page.buttons {
            if button.id < app::debug::GAME_IDS || !actions.insert(button.id) {
                return Err(format!("invalid or duplicate button ID {}", button.id));
            }
            menus.control(
                page.id,
                button.id,
                &button.label,
                DebugValue::Button,
                &button.help,
            );
        }
    }
    Ok(menus.finish())
}
