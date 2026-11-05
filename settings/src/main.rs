//! Seam's settings panel, a separate process so its UI can never delay the
//! mouse hook. Every change goes straight to config.json, then the tray reloads.

#![cfg_attr(windows, windows_subsystem = "windows")]

use gpui_kit::component::button::Button;
use gpui_kit::component::setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings};
use gpui_kit::component::{ActiveTheme, Theme};
use gpui_kit::*;
use seam::config::Config;
use seam::{autostart, tray};

fn seam_exe() -> Option<std::path::PathBuf> {
    Some(std::env::current_exe().ok()?.with_file_name("seam.exe"))
}

fn config_switch(
    title: &'static str,
    about: &'static str,
    read: fn(&Config) -> bool,
    write: fn(&mut Config, bool),
) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::switch(
            move |_: &App| read(&Config::load()),
            move |on: bool, _: &mut App| {
                Config::edit(|c| write(c, on));
                tray::reload();
            },
        ),
    )
    .description(about)
}

fn start_with_windows() -> SettingItem {
    SettingItem::new(
        "Start with Windows",
        SettingField::switch(
            |_: &App| autostart::is_enabled(),
            |on: bool, _: &mut App| {
                if let Some(exe) = seam_exe() {
                    autostart::set(on, &exe);
                }
            },
        ),
    )
    .description("Launch Seam when you sign in.")
}

fn open_folder() -> SettingItem {
    SettingItem::new(
        "Settings folder",
        SettingField::render(|_, _, _| {
            Button::new("open-folder").label("Open").on_click(|_, _, _| {
                if let Some(dir) = Config::path().parent() {
                    let _ = std::fs::create_dir_all(dir);
                    let _ = std::process::Command::new("explorer").arg(dir).spawn();
                }
            })
        }),
    )
    .description("Where config.json and the debug log live.")
}

fn general() -> SettingPage {
    SettingPage::new("General")
        .group(SettingGroup::new().title("Startup").item(start_with_windows()))
        .group(
            SettingGroup::new()
                .title("Pointer")
                .item(config_switch(
                    "Remap pointer at borders",
                    "Cross where the screens physically meet.",
                    |c| c.enabled,
                    |c, on| c.enabled = on,
                ))
                .item(config_switch(
                    "Also align Windows arrangement",
                    "Move monitors in Display settings after lining up.",
                    |c| c.align_arrangement,
                    |c, on| c.align_arrangement = on,
                )),
        )
        .group(SettingGroup::new().title("Files").item(open_folder()))
}

struct Panel;

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(Settings::new("seam-settings").page(general()))
    }
}

fn main() {
    if tray::raise_open_panel() {
        return;
    }
    application().with_assets(assets::Assets).run(|cx| {
        init(cx);
        Theme::sync_system_appearance(None, cx);
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions { title: Some(tray::PANEL_TITLE.into()), ..Default::default() }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(720.), px(480.)), cx))),
            ..Default::default()
        };
        open_window(options, cx, |_, cx| cx.new(|_| Panel)).expect("failed to open the settings window");
    });
}
