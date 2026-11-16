//! General's assistant rows: the two MCP switches, and where `seam-mcp.exe` is, to paste into
//! an assistant's settings.

use crate::panel::Panel;
use crate::theme::tokens::{self, text_style};
use crate::theme::{Palette, palette};
use crate::ui::{self, Row, dimmed, row, row_with, switch};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use serde_json::{Value, json};

/// `seam-mcp.exe` next to the exe running, where the installer and the zip put it, if
/// it is there. The Store copy doesn't have it.
fn mcp_path() -> Option<String> {
    let beside = std::env::current_exe().ok()?.with_file_name("seam-mcp.exe");
    beside.is_file().then(|| beside.display().to_string())
}

fn patching(view: &Entity<Panel>, patch: fn(bool) -> Value) -> impl Fn(bool, &mut Window, &mut App) + 'static + use<> {
    let view = view.clone();
    move |on: bool, window: &mut Window, cx: &mut App| {
        view.update(cx, |panel, cx| panel.patch(patch(on), window, cx));
    }
}

/// The rows after "Let other apps control Seam": MCP, what it may change, and where its exe is
/// while it's on.
pub fn rows(panel: &Panel, view: &Entity<Panel>, window: &mut Window, cx: &mut Context<Panel>) -> Vec<Row> {
    let p: Palette = palette(cx);
    let (on, edit) = (panel.config.api.mcp, panel.config.api.assistants_edit);
    let master = switch(
        "mcp-enabled",
        "Let assistants use Seam (MCP)",
        on,
        window,
        cx,
        patching(view, |on| json!({ "api": { "mcp": on } })),
    );
    // It does nothing while the switch above is off, so it looks and acts unavailable.
    let edits = switch("assistants-edit", "Let assistants change workspaces", edit, window, cx, {
        let set = patching(view, |on| json!({ "api": { "assistants_edit": on } }));
        move |next, window, cx| {
            if on {
                set(next, window, cx);
            }
        }
    });
    let mut rows = vec![
        row(
            "Let assistants use Seam (MCP)",
            Some("Claude and other assistants can read status and switch workspaces".into()),
            master,
            p,
        ),
        dimmed(
            row(
                "Let assistants change workspaces",
                Some("Rename, edit and remove rows, each with an Undo notice. Needs the switch above".into()),
                edits,
                p,
            ),
            "assistants-edit-row",
            (!on).then_some("Turn on Let assistants use Seam (MCP) first"),
        ),
    ];
    if on {
        let path = mcp_path();
        let shown = text_style(div(), tokens::MONO)
            .font_family(cx.theme().mono_font_family.clone())
            .text_color(p.text_secondary)
            .child(path.clone().unwrap_or_else(|| "Not included in this build".into()));
        let copy = path.map(|path| {
            ui::secondary("copy-mcp-path", "Copy", cx).on_click(cx.listener(move |_, _, window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                window.push_notification(Notification::new().message("Copied the path to seam-mcp.exe"), cx);
            }))
        });
        rows.push(row_with("seam-mcp.exe", Some(shown.into_any_element()), div().children(copy)));
    }
    rows
}
