//! The headless screenshot hook used by the UI test harness (`--shot PATH`).

use super::*;

impl App {
    /// `--shot PATH`: render for a moment, capture the framebuffer, exit.
    /// Used by the headless UI harness; exit code 3 means the capture timed out.
    pub(super) fn handle_shot(&mut self, ctx: &egui::Context) {
        if !self.shot_requested && self.shot_started.elapsed() > Duration::from_secs(3) {
            self.shot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                "kubetitan-shot",
            )));
        }
        if !self.shot_requested {
            return;
        }
        let Some(path) = self.shot.clone() else {
            return;
        };
        let mut saved = false;
        ctx.input(|i| {
            for ev in &i.raw.events {
                if let egui::Event::Screenshot { image, .. } = ev {
                    let img = image::RgbaImage::from_fn(
                        image.width() as u32,
                        image.height() as u32,
                        |x, y| {
                            let c = image.pixels[y as usize * image.width() + x as usize];
                            image::Rgba([c.r(), c.g(), c.b(), c.a()])
                        },
                    );
                    match img.save(&path) {
                        Ok(_) => println!("screenshot saved: {path}"),
                        Err(e) => eprintln!("screenshot save failed: {e}"),
                    }
                    saved = true;
                }
            }
        });
        if saved {
            std::process::exit(0);
        }
        if self.shot_started.elapsed() > Duration::from_secs(9) {
            eprintln!("screenshot event never arrived, path={path}");
            std::process::exit(3);
        }
    }
}
