//! General's Assistants section: the two MCP switches, whether an assistant is
//! connected, and where `seam-mcp.exe` is, to paste into an assistant's settings.

use crate::panel::Panel;
use crate::theme::{Palette, palette};
use crate::ui::{self, card, row, row_with, section, switch};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::WindowExt;
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use seam::api::MCP_CLIENT;
use serde_json::{Value, json};

/// Whether an assistant has a connection to Seam open.
pub fn connected(panel: &Panel) -> bool {
    panel.state.as_ref().is_some_and(|s| s.clients.iter().any(|c| c.starts_with(MCP_CLIENT)))
}

/// `seam-mcp.exe` next to the exe running, where the installer and the zip put it.
fn mcp_path() -> String {
    let beside = std::env::current_exe().ok().map(|exe| exe.with_file_name("seam-mcp.exe"));
    beside.map_or_else(|| "seam-mcp.exe".into(), |path| path.display().to_string())
}

fn patching(view: &Entity<Panel>, patch: fn(bool) -> Value) -> impl Fn(bool, &mut Window, &mut App) + 'static + use<> {
    let view = view.clone();
    move |on: bool, window: &mut Window, cx: &mut App| {
        view.update(cx, |panel, cx| panel.patch(patch(on), window, cx));
    }
}

pub fn render(panel: &Panel, view: &Entity<Panel>, window: &mut Window, cx: &mut Context<Panel>) -> AnyElement {
    let p: Palette = palette(cx);
    let (on, edit) = (panel.config.api.mcp, panel.config.api.assistants_edit);
    let status = if !on {
        "Off".to_string()
    } else if connected(panel) {
        "An assistant is connected".to_string()
    } else {
        "None connected".to_string()
    };
    let master = switch(
        "mcp-enabled",
        "Let assistants use Seam (MCP)",
        on,
        window,
        cx,
        patching(view, |on| json!({ "api": { "mcp": on } })),
    );
    // Under the first switch: it does nothing while that one is off, so it looks and acts unavailable.
    let edits = div().opacity(if on { 1. } else { 0.5 }).child(switch(
        "assistants-edit",
        "Let assistants change workspaces",
        edit,
        window,
        cx,
        {
            let set = patching(view, |on| json!({ "api": { "assistants_edit": on } }));
            move |next, window, cx| {
                if on {
                    set(next, window, cx);
                }
            }
        },
    ));
    let path = mcp_path();
    let shown = div()
        .font_family(cx.theme().mono_font_family.clone())
        .font_weight(FontWeight::NORMAL)
        .text_size(px(13.))
        .text_color(p.secondary)
        .child(path.clone());
    let copy = ui::secondary("copy-mcp-path", "Copy", cx).on_click(cx.listener(move |_, _, window, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
        window.push_notification(Notification::new().message("Copied the path to seam-mcp.exe"), cx);
    }));
    section(
        "Assistants",
        card(
            vec![
                row(
                    "Let assistants use Seam (MCP)",
                    Some(format!("Claude and other assistants can read status and switch workspaces. {status}").into()),
                    master,
                    p,
                ),
                row(
                    "Let assistants change workspaces",
                    Some("Rename, edit and remove rows, each with an Undo notice. Needs the switch above".into()),
                    edits,
                    p,
                ),
                row_with("seam-mcp.exe", Some(shown.into_any_element()), copy),
            ],
            p,
        ),
        p,
    )
}
