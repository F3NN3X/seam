//! "Layout for screen N": the built-in and library tiles in one wrapping row.

use super::*;
use crate::theme::Palette;

/// The layout tiles: built in, a divider, then yours. Also gives the tree of a layout that is
/// none of them, so it can be saved.
pub(super) fn section_of(
    panel: &Panel,
    mon: &Mon,
    saved: Option<&ZoneLayout>,
    number: usize,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> (AnyElement, Option<Node>) {
    // Built in: the templates. Your layouts: the library layouts for this screen, or
    // all of them, then New layout. A layout that is none of them shows as Changed,
    // selected, before New layout.
    let library = library::entries(&panel.config.custom_layouts);
    let own = saved.filter(|l| {
        l.template == Template::Custom && !l.custom.as_ref().is_some_and(|id| library.iter().any(|e| e.id == *id))
    });
    let own = own.map(|l| l.tree.clone());
    let mut row = div().flex().flex_wrap().items_center().gap(px(TILE_GAP));
    let templates = Template::ALL.into_iter().filter(|t| *t != Template::Custom).map(Pick::Template);
    let mut at = 0;
    for choice in templates {
        let selected = choice.selected(saved);
        row = row.child(tile(at, choice, selected, None, window, cx));
        at += 1;
    }
    row = row.child(div().flex_none().w(px(1.)).h(px(tokens::SIZE_ROW)).bg(p.hairline_strong));
    // The layout the screen uses always shows, so the picker never hides what's in use.
    let in_use = saved.and_then(|l| l.custom.clone());
    let fits: Vec<&str> = library::for_screen(&library, &mon.id).map(|e| e.id.as_str()).collect();
    let all = panel.all_layouts;
    let shown: Vec<(library::Entry, Option<SharedString>)> = library
        .iter()
        .filter(|e| all || fits.contains(&e.id.as_str()) || in_use.as_ref() == Some(&e.id))
        .map(|e| (e.clone(), (!fits.contains(&e.id.as_str())).then(|| for_screens(e, panel))))
        .collect();
    let picks = shown
        .into_iter()
        .map(|(e, note)| (Pick::Library(e), note))
        .chain(own.clone().map(|tree| (Pick::Changed(tree), None)))
        .chain([(Pick::New, None)]);
    for (choice, note) in picks {
        let selected = choice.selected(saved);
        row = row.child(tile(at, choice, selected, note, window, cx));
        at += 1;
    }
    // Only offered when it would show more, or to go back.
    let toggle = (all || library.len() > fits.len()).then(|| {
        let label = if all { format!("Only screen {number}") } else { "Show all screens".to_string() };
        ui::secondary("zone-all-screens", label, cx)
            .ghost()
            .small()
            .on_click(cx.listener(|panel, _, _, cx| {
                panel.all_layouts = !panel.all_layouts;
                cx.notify();
            }))
            .into_any_element()
    });
    (ui::section_with(format!("Layout for screen {number}"), None, toggle, row, p), own)
}
