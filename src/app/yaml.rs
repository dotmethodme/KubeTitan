//! The YAML tab: fetching a resource's manifest, editing it, and applying the
//! result with a server-side apply.

use super::*;

impl App {
    pub(super) fn fetch_yaml(&mut self) {
        let Some(sel) = self.selection.as_ref() else {
            return;
        };
        let (kind, ns, name) = (sel.kind, sel.namespace.clone(), sel.name.clone());
        self.yaml = None;
        let (tx, rx) = mpsc::channel::<Result<String, String>>();
        let msg_tx = self.tx.clone();
        std::thread::spawn(move || {
            while let Ok(res) = rx.recv() {
                let _ = msg_tx.send(Msg::Yaml(res));
            }
        });
        cluster::fetch_yaml(&self.rt, self.client.clone(), kind, ns, name, tx);
    }

    // -----------------------------------------------------------------------
    // YAML editing (Vue's `useResourceYamlEditor`)
    // -----------------------------------------------------------------------

    /// Starts editing the YAML tab — the draft is seeded from what is shown.
    pub fn start_yaml_edit(&mut self) {
        if let Some(Ok(text)) = &self.yaml {
            self.yaml_draft = Some(text.clone());
            self.yaml_error = None;
        }
    }

    /// Leaves the editor, discarding the draft.
    pub fn cancel_yaml_edit(&mut self) {
        self.yaml_draft = None;
        self.yaml_review = false;
        self.yaml_applying = false;
        self.yaml_error = None;
    }

    /// `hasYamlChanges` — "Review & Apply" stays disabled until the draft differs.
    pub fn yaml_changed(&self) -> bool {
        matches!((&self.yaml, &self.yaml_draft), (Some(Ok(shown)), Some(draft)) if shown != draft)
    }

    /// Server-side applies the draft; the outcome arrives as `Msg::YamlApplied`.
    pub fn apply_yaml(&mut self, force: bool) {
        let (Some(Ok(_)), Some(draft), Some(selection)) =
            (&self.yaml, &self.yaml_draft, &self.selection)
        else {
            return;
        };
        let (kind, draft) = (selection.kind, draft.clone());
        self.yaml_applying = true;
        self.yaml_error = None;

        let (tx, rx) = mpsc::channel::<Result<String, String>>();
        let msg_tx = self.tx.clone();
        std::thread::spawn(move || {
            while let Ok(res) = rx.recv() {
                let _ = msg_tx.send(Msg::YamlApplied(res));
            }
        });
        cluster::apply_yaml(&self.rt, self.client.clone(), kind, draft, force, tx);
    }

    pub(super) fn on_yaml_applied(&mut self, result: Result<String, String>) {
        self.yaml_applying = false;
        match result {
            Ok(message) => {
                // like Vue: the draft becomes what the tab shows
                if let Some(draft) = self.yaml_draft.take() {
                    self.yaml = Some(Ok(draft));
                }
                self.yaml_review = false;
                self.flash_status(message);
            }
            Err(error) => self.yaml_error = Some(error),
        }
    }
}
