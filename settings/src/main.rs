//! Spike: Seam's settings panel on GPUI Kit. Toggles aren't wired to config yet.
//! `--measure` prints the time to the first painted frame, then quits.

use gpui_kit::component::button::Button;
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings};
use gpui_kit::*;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::Instant;

static REMAP: AtomicBool = AtomicBool::new(true);
static ALIGN: AtomicBool = AtomicBool::new(true);
static AUTOSTART: AtomicBool = AtomicBool::new(false);

fn toggle(title: &'static str, about: &'static str, flag: &'static AtomicBool) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::switch(move |_: &App| flag.load(Relaxed), move |on: bool, _: &mut App| flag.store(on, Relaxed)),
    )
    .description(about)
}

fn general() -> SettingPage {
    SettingPage::new("General")
        .group(
            SettingGroup::new()
                .title("Startup")
                .item(toggle("Start with Windows", "Launch Seam when you sign in.", &AUTOSTART)),
        )
        .group(
            SettingGroup::new()
                .title("Pointer")
                .item(toggle("Remap pointer at borders", "Cross where the screens physically meet.", &REMAP))
                .item(toggle(
                    "Also align Windows arrangement",
                    "Move monitors in Display settings after lining up.",
                    &ALIGN,
                )),
        )
        .group(
            SettingGroup::new().title("Files").item(SettingItem::new(
                "Settings folder",
                SettingField::render(|_, _, _| Button::new("open-folder").label("Open").on_click(|_, _, _| {})),
            )),
        )
}

struct Panel {
    started: Option<Instant>,
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        if let Some(t0) = self.started.take() {
            window.on_next_frame(move |_, cx| {
                println!("first frame after {} ms", t0.elapsed().as_millis());
                cx.quit();
            });
        }
        div().size_full().child(Settings::new("seam-settings").page(general()))
    }
}

fn main() {
    let t0 = Instant::now();
    let measure = std::env::args().any(|a| a == "--measure");
    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        open_window(WindowOptions::default(), cx, move |_, cx| {
            cx.new(|_| Panel { started: measure.then_some(t0) })
        })
        .expect("failed to open the settings window");
    });
}
