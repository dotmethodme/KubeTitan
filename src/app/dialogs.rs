//! Modal dialogs owned by the app: the action confirmation and the YAML apply
//! preview.

use super::*;

impl App {
    /// Confirm/scale dialogs, drawn above everything else.
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending.as_ref() else {
            return;
        };
        let (title, message, confirm_label, danger) = match pending.action {
            Action::Delete => (
                format!("Delete {}", pending.kind.kind),
                format!(
                    "Are you sure you want to delete \u{201c}{}\u{201d}? This action cannot be undone.",
                    pending.name
                ),
                "Delete".to_string(),
                true,
            ),
            Action::Restart => (
                format!("Restart {}", pending.kind.kind),
                format!(
                    "Are you sure you want to restart \u{201c}{}\u{201d}? This will trigger a rolling restart.",
                    pending.name
                ),
                "Restart".to_string(),
                true,
            ),
            // never pending: one opens its own dialog, the other a shell
            Action::Forward | Action::Terminal => return,
            Action::Scale => (
                "Scale Deployment".to_string(),
                pending.name.clone(),
                "Scale".to_string(),
                false,
            ),
        };
        let name = pending.name.clone();
        let action = pending.action;
        let current = pending.current_replicas;
        let mut value = pending.replicas;

        let outcome = match action {
            Action::Scale => widgets::scale_dialog(ctx, &name, current, &mut value),
            _ => widgets::confirm_dialog(ctx, true, &title, &message, &confirm_label, danger),
        };
        if action == Action::Scale {
            self.set_scale_target(value);
        }

        match outcome {
            widgets::Confirm::Accept => self.confirm_pending(),
            widgets::Confirm::Cancel => self.cancel_pending(),
            widgets::Confirm::None => {}
        }
    }

    /// Vue's `PortForwardDialog`: what to forward, and which ports. The dialog
    /// owns its own error, so a rejected port does not close the form.
    pub(super) fn forward_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.forward_dialog.take() else {
            return;
        };
        let mut keep = true;
        match widgets::forward_dialog(ctx, &mut dialog) {
            widgets::ForwardChoice::Start => match dialog.request() {
                Ok(request) => match crate::portforward::start(self, request) {
                    Ok(id) => {
                        keep = false;
                        self.set_view(View::PortForwards);
                        self.flash_status(format!("Port forward {id} started"));
                    }
                    Err(error) => dialog.error = Some(error),
                },
                Err(error) => dialog.error = Some(error),
            },
            widgets::ForwardChoice::Cancel => keep = false,
            widgets::ForwardChoice::None => {}
        }
        if keep {
            self.forward_dialog = Some(dialog);
        }
    }

    /// Vue's `ResourceYamlApplyDialog`, driven by `widgets::yaml_apply_dialog`:
    /// the diff of the draft, then a server-side apply.
    pub(super) fn yaml_apply_dialog(&mut self, ctx: &egui::Context) {
        let (Some(Ok(shown)), Some(draft)) = (self.yaml.clone(), self.yaml_draft.clone()) else {
            // the drawer or the editor went away while the dialog was open
            self.yaml_review = false;
            return;
        };
        match widgets::yaml_apply_dialog(
            ctx,
            self.yaml_review,
            &shown,
            &draft,
            self.yaml_applying,
            self.yaml_error.as_deref(),
        ) {
            widgets::YamlChoice::Apply(force) => self.apply_yaml(force),
            widgets::YamlChoice::Cancel => self.cancel_yaml_edit(),
            widgets::YamlChoice::None => {}
        }
    }
}
