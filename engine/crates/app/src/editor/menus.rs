//! Game-owned pages in the shell's F10 editor. IDs from GAME_IDS upwards
//! stay stable when a game changes labels, visibility, or the ordering of rows.
use render::debug_ui::{DebugControl, DebugMenu, DebugValue};

#[derive(Default)]
pub struct EditorMenus {
    pages: Vec<DebugMenu>,
    controls: Vec<DebugControl>,
}

impl EditorMenus {
    pub fn page(&mut self, id: u32, parent: Option<u32>, label: &str, help: &str) {
        assert!(
            id >= crate::debug::GAME_IDS && self.pages.iter().all(|p| p.id != id),
            "editor menu needs a unique game ID"
        );
        self.pages.push(DebugMenu {
            id,
            parent,
            label: label.into(),
            help: help.into(),
        });
    }

    pub fn control(
        &mut self,
        page: u32,
        id: u32,
        label: &str,
        value: DebugValue,
        help: &str,
    ) -> &mut DebugControl {
        assert!(
            id >= crate::debug::GAME_IDS && self.controls.iter().all(|c| c.id != id),
            "editor control needs a unique game ID"
        );
        self.controls.push(DebugControl {
            id,
            menu: Some(page),
            label: label.into(),
            value,
            help: help.into(),
            enabled: true,
            advanced: false,
        });
        self.controls.last_mut().unwrap()
    }

    pub fn finish(self) -> (Vec<DebugMenu>, Vec<DebugControl>) {
        (self.pages, self.controls)
    }
}
