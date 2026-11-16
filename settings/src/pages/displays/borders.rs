//! The Borders section, the strip under the map, and the words every place uses for a border.

use super::physical_map::sample;
use super::*;
use desks::short_date;

/// KEY of the Borders section's own disclosure.
const BORDERS: &str = "Borders";
const TOPIC: &str = "Pointer and borders";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    LinedUp,
    Glued,
    NotOnDesk,
    /// Not lined up: the pointer may jump at it.
    Loose,
}

pub(super) fn kind(b: &Border) -> Kind {
    match (b.not_on_desk, b.glued, b.state) {
        (true, ..) => Kind::NotOnDesk,
        (_, true, _) => Kind::Glued,
        (_, _, BorderState::LinedUp) => Kind::LinedUp,
        _ => Kind::Loose,
    }
}

/// A screen the desk has no place for: the far side of borders that are all marked not on the desk.
pub(super) fn off_desk(state: &State, number: usize) -> bool {
    let mine = |b: &&Border| b.a as usize == number || b.b as usize == number;
    state.borders.iter().any(|b| b.b as usize == number && b.not_on_desk)
        && state.borders.iter().filter(mine).all(|b| b.not_on_desk)
}

fn lower(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
        Side::Top => "top",
        Side::Bottom => "bottom",
    }
}

fn capital(side: Side) -> &'static str {
    match side {
        Side::Left => "Left",
        Side::Right => "Right",
        Side::Top => "Top",
        Side::Bottom => "Bottom",
    }
}

fn title(b: &Border) -> String {
    format!("Screen {} and screen {}", b.a, b.b)
}

/// The border in plain words.
fn plain(b: &Border) -> String {
    let (a, other, side) = (b.a, b.b, b.side);
    match kind(b) {
        Kind::NotOnDesk => format!("{other} is not on your desk, so the pointer stops at the {} of {a}", lower(side)),
        Kind::Glued => format!("{} of {a} is glued to {} of {other}", capital(side), lower(side.opposite())),
        Kind::LinedUp | Kind::Loose => {
            format!("{} of {a} meets {} of {other}", capital(side), lower(side.opposite()))
        }
    }
}

/// When the desk in use was lined up, as a short date.
fn lined_up_on(panel: &Panel, state: &State) -> Option<String> {
    panel.config.desks.get(&state.desk.sig).and_then(|d| d.lined_up_at).and_then(short_date)
}

/// How the border stands, and what was measured. Scale and offset are for the strip only.
fn measured(panel: &Panel, state: &State, b: &Border) -> String {
    let link = panel.links.iter().find(|l| (l.a + 1, l.b + 1) == (b.a as usize, b.b as usize));
    let no_size = [b.a, b.b].into_iter().find(|n| panel.mons.get(*n as usize - 1).is_some_and(|m| m.raw_dpi <= 0.));
    let head = match (kind(b), lined_up_on(panel, state)) {
        (Kind::LinedUp, Some(when)) => format!("Lined up {when}"),
        (Kind::LinedUp, None) => "Lined up".to_string(),
        (Kind::Glued, _) => "Glued".to_string(),
        _ => "Not lined up".to_string(),
    };
    match (kind(b), link, no_size) {
        (Kind::NotOnDesk, ..) => "The pointer stops at this edge".to_string(),
        (_, Some(l), _) => format!("{head}. Scale {:.2}, offset {:+.0} px", l.map.s, l.map.o),
        (_, None, Some(n)) => format!("Screen {n} reports no size. Line up guesses a scale of 1"),
        (_, None, None) => format!("{head}. Line up this border to set it"),
    }
}

/// What a border's menu and the strip offer.
#[derive(Clone, Copy)]
enum Act {
    LineUp,
    LineUpAgain,
    LineUpInstead,
    Show,
    Unglue,
    PutOnDesk,
    OffDesk,
    Learn,
}

impl Act {
    fn label(self, b: &Border) -> String {
        match self {
            Act::LineUp => "Line up these screens".into(),
            Act::LineUpAgain => "Line up again".into(),
            Act::LineUpInstead => "Line up instead".into(),
            Act::Show => "Show on screens".into(),
            Act::Unglue => "Unglue".into(),
            Act::PutOnDesk => format!("Put {} on my desk", b.b),
            Act::OffDesk => "Not on my desk".into(),
            Act::Learn => "Learn more".into(),
        }
    }

    fn run(self, b: &Border, panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>) {
        match self {
            Act::LineUp | Act::LineUpAgain | Act::LineUpInstead => panel.command("lineup.start", json!({}), window, cx),
            Act::Show => panel.command("layout.show", json!({ "view": "seam" }), window, cx),
            Act::Unglue => unglue(panel, b, window, cx),
            Act::PutOnDesk | Act::OffDesk => toggle_not_on_desk(panel, b, window, cx),
            Act::Learn => panel.open_topic(TOPIC, window, cx),
        }
    }
}

/// The menu of a border, the first being what the strip offers. A border that touches in Windows
/// can be marked as not on the desk.
fn acts(b: &Border, touching_now: bool) -> Vec<Act> {
    let off = touching_now.then_some(Act::OffDesk);
    match kind(b) {
        Kind::LinedUp => [Act::LineUpAgain, Act::Show].into_iter().chain(off).chain([Act::Learn]).collect(),
        Kind::Glued => vec![Act::LineUpInstead, Act::Unglue, Act::Learn],
        Kind::NotOnDesk => vec![Act::PutOnDesk, Act::Learn],
        Kind::Loose => [Act::LineUp, Act::Show].into_iter().chain(off).chain([Act::Learn]).collect(),
    }
}

fn touching_now(panel: &Panel, b: &Border) -> bool {
    let touch = |n: u32| panel.mons.get(n as usize - 1).map(|m| m.rect);
    touch(b.a).zip(touch(b.b)).is_some_and(|(a, other)| touching(&a, &other).is_some())
}

/// The "..." of a border.
fn border_menu(panel: &Panel, b: &Border, i: usize, cx: &mut Context<Panel>) -> AnyElement {
    let view = cx.entity().downgrade();
    let (border, offered) = (b.clone(), acts(b, touching_now(panel, b)));
    ui::secondary(("border-more", i), "", cx)
        .ghost()
        .icon(Icon::new(assets::IconName::Ellipsis))
        .accessibility_label(format!("More for screen {} and screen {}", b.a, b.b))
        .dropdown_menu(move |mut menu, _, _| {
            for act in &offered {
                let (view, border, act) = (view.clone(), border.clone(), *act);
                menu = menu.item(PopupMenuItem::new(act.label(&border)).on_click(move |_, window, cx| {
                    let border = border.clone();
                    let _ = view.update(cx, |panel, cx| act.run(&border, panel, window, cx));
                }));
            }
            menu
        })
        .into_any_element()
}

/// The ids of the two screens of a border, which saved borders are known by.
fn screen_ids(state: &State, b: &Border) -> Option<(String, String)> {
    let id = |n: u32| state.screens.iter().find(|s| s.number == n).map(|s| s.id.clone());
    id(b.a).zip(id(b.b))
}

fn same_pair(saved: &SavedBorder, (a, b): &(String, String)) -> bool {
    (&saved.a, &saved.b) == (a, b) || (&saved.a, &saved.b) == (b, a)
}

/// Mark a border as not on the desk, or back. A border that was never lined up gets a
/// stand-in map, which "Put on my desk" takes away again, because nothing was measured.
fn toggle_not_on_desk(panel: &mut Panel, border: &Border, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(state) = panel.state.as_ref() else { return };
    let Some(ids) = screen_ids(state, border) else { return };
    let sig = state.desk.sig.clone();
    let known = panel.config.desks.get(&sig);
    let mut saved = known.map(|d| d.borders.clone()).unwrap_or_default();
    let stand_in = EdgeMap { s: 1., o: 0. };
    match saved.iter().position(|s| same_pair(s, &ids)) {
        Some(at) if saved[at].not_on_desk && saved[at].map == stand_in => {
            saved.remove(at);
        }
        Some(at) => saved[at].not_on_desk = !saved[at].not_on_desk,
        None => saved.push(SavedBorder {
            a: ids.0,
            b: ids.1,
            side: border.side,
            map: stand_in,
            glued: false,
            not_on_desk: true,
        }),
    }
    let mut desk = json!({ "borders": saved });
    if known.is_none() {
        desk["name"] = json!(state.desk.name);
    }
    panel.patch(json!({ "desks": { sig: desk } }), window, cx);
}

/// Stop a border being glued. Undo puts the desk's borders back as they were.
fn unglue(panel: &mut Panel, border: &Border, window: &mut Window, cx: &mut Context<Panel>) {
    let Some(state) = panel.state.as_ref() else { return };
    let Some(ids) = screen_ids(state, border) else { return };
    let sig = state.desk.sig.clone();
    let Some(before) = panel.config.desks.get(&sig).map(|d| d.borders.clone()) else { return };
    let mut saved = before.clone();
    for s in saved.iter_mut().filter(|s| same_pair(s, &ids)) {
        s.glued = false;
    }
    panel.patch(json!({ "desks": { sig.clone(): { "borders": saved } } }), window, cx);
    let message = format!("Unglued {}", title(border).to_lowercase());
    let undo = move |panel: &mut Panel, window: &mut Window, cx: &mut Context<Panel>| {
        panel.patch(json!({ "desks": { sig.clone(): { "borders": before.clone() } } }), window, cx);
    };
    panel.offer_undo(message, undo, window, cx);
}

/// How many borders are of each kind.
struct Tally {
    lined_up: usize,
    glued: usize,
    off: usize,
    loose: usize,
}

fn tally(state: &State) -> Tally {
    let count = |which: Kind| state.borders.iter().filter(|b| kind(b) == which).count();
    Tally {
        lined_up: count(Kind::LinedUp),
        glued: count(Kind::Glued),
        off: count(Kind::NotOnDesk),
        loose: count(Kind::Loose),
    }
}

/// "a", "a and b" or "a, b, and c".
fn listed(mut parts: Vec<String>) -> String {
    match parts.len() {
        0 | 1 => parts.pop().unwrap_or_default(),
        2 => parts.join(" and "),
        n => format!("{}, and {}", parts[..n - 1].join(", "), parts[n - 1]),
    }
}

fn summary_title(t: &Tally, total: usize) -> String {
    match (t.loose, total) {
        (0, 1) => "The border works".to_string(),
        (0, n) => format!("All {n} borders work"),
        (1, _) => "1 border needs lining up".to_string(),
        (n, _) => format!("{n} borders need lining up"),
    }
}

fn summary_about(t: &Tally, lined_up_on: Option<String>) -> String {
    let mut parts = vec![];
    if t.lined_up > 0 {
        let when = lined_up_on.map(|d| format!(" {d}")).unwrap_or_default();
        parts.push(format!("{} lined up{when}", t.lined_up));
    }
    if t.glued > 0 {
        parts.push(format!("{} glued", t.glued));
    }
    if t.off > 0 {
        parts.push(format!("{} stops the pointer at a screen that's not on your desk", t.off));
    }
    if t.loose > 0 {
        parts.push(format!("{} not lined up", t.loose));
    }
    listed(parts)
}

/// The first row of the card: how the borders are doing, and "Show" or "Hide" for the rest.
fn summary_row(
    panel: &Panel,
    state: &State,
    open: bool,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Row {
    let t = tally(state);
    let (heading, about) = (summary_title(&t, state.borders.len()), summary_about(&t, lined_up_on(panel, state)));
    let (colour, filled) = if t.loose > 0 { (p.negative, false) } else { (p.positive, true) };
    let row = div()
        .flex()
        .items_center()
        .gap(px(tokens::SPACE_12))
        .min_h(px(tokens::SIZE_ROW_2))
        .px(px(tokens::SPACE_16))
        .py(px(tokens::SPACE_8))
        .child(ui::dot(colour, filled))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(tokens::SPACE_2))
                .flex_1()
                .min_w_0()
                .child(text_style(div(), tokens::BODY).child(heading.clone()))
                .child(ui::description(about, p)),
        )
        .child(text_style(div(), tokens::LABEL).text_color(p.text_secondary).child(if open { "Hide" } else { "Show" }))
        .child(ui::chevron(open, p));
    let view = cx.entity();
    let toggle = move |_: &mut Window, cx: &mut App| {
        view.update(cx, |panel, cx| {
            panel.toggle_disclosure(BORDERS);
            cx.notify();
        });
    };
    Row::from(ui::disclose(row, "borders-summary", heading, open, window, cx, toggle))
}

/// One border: its screens, in plain words, how it stands and its menu. Clicking it selects it on the map.
fn border_row(
    panel: &Panel,
    state: &State,
    b: &Border,
    (i, last): (usize, bool),
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> Row {
    let selected = panel.border_pick == Some((b.a, b.b));
    let (colour, filled, status) = match kind(b) {
        Kind::LinedUp => (
            p.positive,
            true,
            lined_up_on(panel, state).map_or("Lined up".to_string(), |when| format!("Lined up {when}")),
        ),
        Kind::Glued => (p.positive, true, "Glued".to_string()),
        Kind::NotOnDesk => (p.text_muted, false, "Pointer stops".to_string()),
        Kind::Loose => (p.negative, false, "Not lined up".to_string()),
    };
    let status = div()
        .flex()
        .items_center()
        .gap(px(tokens::SPACE_8))
        .child(ui::dot(colour, filled))
        .child(text_style(div(), tokens::LABEL).text_color(p.text_secondary).child(status));
    // Over the row, so a click on the menu stays there.
    let control = div()
        .flex()
        .items_center()
        .gap(px(tokens::SPACE_8))
        .occlude()
        .child(status)
        .child(border_menu(panel, b, i, cx));
    let (a, other) = (b.a, b.b);
    let heading = title(b);
    let view = cx.entity();
    ui::row_with(heading.clone(), Some(ui::description(plain(b), p).into_any_element()), control).map(move |row| {
        let pick = move |_: &mut Window, cx: &mut App| {
            view.update(cx, |panel, cx| {
                panel.border_pick = Some((a, other));
                panel.layout_view = LayoutView::Seam;
                cx.notify();
            })
        };
        ui::selectable(("border-row", i), heading, selected, window, cx, pick)
            .when(selected, |d| d.bg(p.accent_tint))
            .when(!selected, |d| d.hover(|d| d.bg(p.surface_raised)).active(|d| d.bg(p.surface_card)))
            .when(last, |d| d.rounded_b(px(tokens::RADIUS_12 - 1.)))
            .child(row)
            .into_any_element()
    })
}

/// The Borders section: a summary, and a row for each border once it's opened.
pub(super) fn section(
    panel: &Panel,
    state: &State,
    p: Palette,
    window: &mut Window,
    cx: &mut Context<Panel>,
) -> AnyElement {
    if state.borders.is_empty() {
        return div().hidden().into_any_element();
    }
    // A border that needs lining up opens the list, and a click on the summary closes it.
    let open = panel.disclosed.contains(BORDERS) != (tally(state).loose > 0);
    let mut rows = vec![summary_row(panel, state, open, p, window, cx)];
    if open {
        let last = state.borders.len() - 1;
        rows.extend(
            state.borders.iter().enumerate().map(|(i, b)| border_row(panel, state, b, (i, i == last), p, window, cx)),
        );
    }
    ui::section(BORDERS, card(rows, p), p)
}

/// The selected border, under the map: how it looks, what it is and what can be done about it.
pub(super) fn strip(panel: &Panel, state: &State, p: Palette, cx: &mut Context<Panel>) -> Option<Div> {
    let picked = panel.border_pick?;
    let b = state.borders.iter().find(|b| (b.a, b.b) == picked)?;
    let offered = acts(b, touching_now(panel, b));
    let button = |act: Act, id: &'static str, cx: &mut Context<Panel>| {
        let border = b.clone();
        ui::secondary(id, act.label(b), cx)
            .on_click(cx.listener(move |panel, _, window, cx| act.run(&border, panel, window, cx)))
    };
    let learn = button(Act::Learn, "border-learn", cx).ghost();
    let clear = ui::secondary("border-clear", "", cx)
        .ghost()
        .icon(Icon::new(assets::IconName::X))
        .tooltip("Clear selection")
        .accessibility_label("Clear selection")
        .on_click(cx.listener(|panel, _, _, cx| {
            panel.border_pick = None;
            cx.notify();
        }));
    Some(
        div()
            .flex()
            .items_center()
            .gap(px(tokens::SPACE_12))
            .px(px(tokens::SPACE_12))
            .py(px(tokens::SPACE_8))
            .border_1()
            .border_color(p.hairline)
            .rounded(px(tokens::RADIUS_8))
            .child(sample(kind(b), p))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(text_style(div(), tokens::BODY).child(plain(b)))
                    .child(ui::tabular(ui::description(measured(panel, state, b), p))),
            )
            .children(offered.first().map(|act| button(*act, "border-act", cx)))
            .child(learn)
            .child(clear),
    )
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it would bring in gpui's own `test` attribute.
    use super::{Border, BorderState, Kind, Side, Tally, kind, plain, summary_about, summary_title};

    fn border(side: Side, state: BorderState, glued: bool, not_on_desk: bool) -> Border {
        Border { a: 2, b: 1, side, state, glued, not_on_desk }
    }

    #[test]
    fn a_border_is_said_in_plain_words() {
        let lined = border(Side::Bottom, BorderState::LinedUp, false, false);
        assert_eq!(plain(&lined), "Bottom of 2 meets top of 1");
        let glued = border(Side::Right, BorderState::LinedUp, true, false);
        assert_eq!(plain(&glued), "Right of 2 is glued to left of 1");
        let off = border(Side::Bottom, BorderState::LinedUp, false, true);
        assert_eq!(plain(&off), "1 is not on your desk, so the pointer stops at the bottom of 2");
    }

    #[test]
    fn not_on_the_desk_beats_glued_and_lined_up() {
        assert!(kind(&border(Side::Left, BorderState::LinedUp, true, true)) == Kind::NotOnDesk);
        assert!(kind(&border(Side::Left, BorderState::NotLinedUp, true, false)) == Kind::Glued);
        assert!(kind(&border(Side::Left, BorderState::NotLinedUp, false, false)) == Kind::Loose);
    }

    #[test]
    fn the_summary_counts_each_kind() {
        let t = Tally { lined_up: 2, glued: 2, off: 1, loose: 0 };
        assert_eq!(summary_title(&t, 5), "All 5 borders work");
        assert_eq!(
            summary_about(&t, Some("9 Nov".into())),
            "2 lined up 9 Nov, 2 glued, and 1 stops the pointer at a screen that's not on your desk"
        );
        let loose = Tally { lined_up: 0, glued: 0, off: 0, loose: 1 };
        assert_eq!(summary_title(&loose, 1), "1 border needs lining up");
    }
}
