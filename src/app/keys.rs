//! Keyboard handling, in one place.
//!
//! Escape and the global shortcuts used to be handled by whoever happened to be
//! drawing: the palette owned Ctrl+K and its own Escape, the picker and the
//! shortcuts sheet each caught Escape separately. Two modals open at once could
//! both react to the same key, and a hotkey could live in a different file from
//! the rest of its shortcut family.
//!
//! Now the update loop asks this module once per frame, before anything paints,
//! and the answer is unambiguous: the topmost modal wins.

use super::*;

impl App {
    /// Global shortcuts and modal dismissal. Runs before the frame is painted.
    pub(super) fn handle_keys(&mut self, ctx: &egui::Context) {
        // A shell owns the keyboard while it has focus: Esc belongs to vim, and
        // Ctrl+C to the running command. Clicking outside the grid hands the
        // keys back, which is how the app's shortcuts return.
        if self.terminal_keys {
            return;
        }
        let cmd = |key: egui::Key| ctx.input(|i| i.modifiers.command && i.key_pressed(key));

        // Ctrl+K / Ctrl+P: the command palette, always in its plain mode.
        if cmd(egui::Key::K) || cmd(egui::Key::P) {
            self.palette_open = !self.palette_open;
            self.palette_query.clear();
            self.palette_sel = 0;
            self.palette_mode = PaletteMode::Commands;
        }
        // Ctrl+.: the same palette as a namespace switcher; pressing it again
        // closes, as the Vue palette does.
        if cmd(egui::Key::Period) {
            self.open_palette_mode(PaletteMode::Namespaces);
        }
        // Ctrl+,: the same palette as a context switcher (Vue's Ctrl+,)
        if cmd(egui::Key::Comma) {
            self.open_palette_mode(PaletteMode::Contexts);
        }
        if cmd(egui::Key::Slash) {
            self.shortcuts_open = !self.shortcuts_open;
        }
        if cmd(egui::Key::F) {
            self.focus_search = true;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.dismiss_topmost();
        }

        // Quick switch, the way the Vue's shortcut sheet has it: Alt+1-9 the Nth
        // context, Ctrl+1-9 the Nth view in sidebar order. The Vue checks the
        // physical key, because Option+number is not a number on macOS.
        const DIGITS: [egui::Key; 9] = [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        let digit = ctx.input(|i| {
            DIGITS
                .iter()
                .position(|key| i.key_pressed(*key))
                .map(|index| (index, i.modifiers))
        });
        if let Some((index, modifiers)) = digit {
            if modifiers.alt && !modifiers.command && !modifiers.ctrl {
                if let Some(name) = self.contexts.get(index).cloned() {
                    let _ = self.switch_context(name);
                }
            } else if modifiers.command
                && let Some((view, _, _, _)) = crate::palette::nav_entries().get(index)
            {
                self.set_view(view.clone());
            }
        }
    }

    /// Ctrl+. / Ctrl+,: open the palette already in that mode; pressing the same
    /// shortcut again closes it, the way the Vue palette behaves.
    pub(crate) fn open_palette_mode(&mut self, mode: PaletteMode) {
        let closing = self.palette_open && self.palette_mode == mode;
        self.palette_open = !closing;
        self.palette_mode = if closing { PaletteMode::Commands } else { mode };
        self.palette_query.clear();
        self.palette_sel = 0;
    }

    /// Escape closes the topmost thing only — palette, resource picker,
    /// confirmation dialog, then the shortcuts sheet, then the drawer.
    pub(crate) fn dismiss_topmost(&mut self) {
        if self.palette_open {
            self.palette_open = false;
            self.palette_mode = PaletteMode::Commands;
        } else if self.picker_open {
            self.picker_open = false;
        } else if self.pending.is_some() {
            self.cancel_pending();
        } else if self.forward_dialog.is_some() {
            self.forward_dialog = None;
        } else if self.shortcuts_open {
            self.shortcuts_open = false;
        } else if self.data_edit.is_some() {
            // an edit in progress is the first thing Escape backs out of
            self.cancel_data_edit();
        } else if self.selection.is_some() {
            // the drawer is the lowest thing on the stack: Escape closes it only
            // once nothing above it is open
            self.close_drawer();
        }
    }
}
