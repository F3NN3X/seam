//! The tests for which picture each screen shows, the fits, the cache and the checks.

use super::*;
use crate::config::ScreenInUse;

fn picture(path: &str) -> Wallpaper {
    Wallpaper { path: path.into(), fit: Fit::Fill }
}

fn set(pairs: &[(&str, &str)]) -> WallpaperSet {
    WallpaperSet { screens: pairs.iter().map(|(s, p)| (s.to_string(), picture(p))).collect() }
}

fn ws(id: &str, scope: Option<&str>) -> Workspace {
    Workspace { id: id.into(), scope: scope.map(Into::into), ..Workspace::default() }
}

/// Two screens, `a` in use on both, with pictures for "dev" (all screens) and "notes" (screen s2 only).
fn desk() -> Desk {
    Desk {
        active_workspace: Some("dev".into()),
        wallpaper: set(&[("s1", "desk1.jpg"), ("s2", "desk2.jpg")]),
        workspace_wallpapers: BTreeMap::from([
            ("dev".to_string(), set(&[("s1", "dev1.jpg")])),
            ("notes".to_string(), set(&[("s2", "notes2.jpg")])),
        ]),
        ..Desk::default()
    }
}

fn paths(wanted: &[(String, Option<Wallpaper>, Origin)]) -> Vec<(&str, Option<&str>, Origin)> {
    wanted.iter().map(|(s, w, o)| (s.as_str(), w.as_ref().map(|w| w.path.as_str()), *o)).collect()
}

#[test]
fn resolves_through_in_use_on_then_desk_then_own() {
    let mut d = desk();
    d.wallpaper.screens.remove("s2");
    let got = wanted(&d, &["s1", "s2"]);
    assert_eq!(paths(&got), [("s1", Some("dev1.jpg"), Origin::Workspace), ("s2", None, Origin::Own)]);
    // The desk default stands in where the workspace sets nothing.
    let got = wanted(&desk(), &["s1", "s2"]);
    assert_eq!(paths(&got)[1], ("s2", Some("desk2.jpg"), Origin::Desk));
    // With nothing in use, only the desk default counts.
    let none = Desk { active_workspace: None, ..desk() };
    assert_eq!(paths(&wanted(&none, &["s1"])), [("s1", Some("desk1.jpg"), Origin::Desk)]);
    // The choices go on past a first one, for a picture that is gone.
    assert_eq!(choices(&desk(), "s1").len(), 2);
}

#[test]
fn one_screen_workspace_without_a_picture_shows_the_desk_default() {
    let mut d = desk();
    // "solo" is in use on s1 and sets nothing: the desk default, not dev's picture (Q6).
    d.screen_in_use.insert("s1".into(), ScreenInUse { workspace: Some("solo".into()), previous: None });
    assert_eq!(paths(&wanted(&d, &["s1"])), [("s1", Some("desk1.jpg"), Origin::Desk)]);
}

#[test]
fn one_screen_switch_changes_only_its_screen() {
    let d = desk();
    let before = wanted(&d, &["s1", "s2"]);
    let next = after_switch(&d, "notes", Some("s2"), &["s1", "s2"], &[]);
    let after = wanted(&next, &["s1", "s2"]);
    assert_eq!(before[0], after[0]);
    assert_eq!(paths(&after)[1], ("s2", Some("notes2.jpg"), Origin::Workspace));
    // And an All-screens switch can change both.
    let all = wanted(&after_switch(&next, "other", None, &["s1", "s2"], &[]), &["s1", "s2"]);
    assert_eq!(paths(&all), [("s1", Some("desk1.jpg"), Origin::Desk), ("s2", Some("desk2.jpg"), Origin::Desk)]);
}

#[test]
fn snap_back_of_the_desk_wide_workspace_leaves_the_away_screens_pictures() {
    // dev is desk-wide, notes is in use on s2; Snap back of dev leaves s2 to notes.
    let d = after_switch(&desk(), "notes", Some("s2"), &["s1", "s2"], &[]);
    let before = wanted(&d, &["s1", "s2"]);
    let back = after_switch(&d, "dev", None, &["s1", "s2"], &["s2".to_string()]);
    assert_eq!(wanted(&back, &["s1", "s2"]), before);
    assert_eq!(paths(&before)[1], ("s2", Some("notes2.jpg"), Origin::Workspace));
    // Without away, the same switch takes s2 too.
    let all = after_switch(&d, "dev", None, &["s1", "s2"], &[]);
    assert_eq!(paths(&wanted(&all, &["s2"]))[0].2, Origin::Desk);
}

#[test]
fn one_screen_workspace_sets_only_its_scope_screen() {
    let solo = ws("solo", Some("s2"));
    assert_eq!(may_set(Some(&solo), "s2"), Ok(()));
    assert_eq!(may_set(Some(&solo), "s1"), Err(ONE_SCREEN));
    assert_eq!(may_set(Some(&ws("dev", None)), "s1"), Ok(()));
    assert_eq!(may_set(None, "s1"), Ok(()));
}

#[test]
fn scope_change_drops_the_old_screens_picture() {
    let mut d = desk();
    d.workspace_wallpapers.insert("solo".into(), set(&[("s1", "a.jpg"), ("s2", "b.jpg")]));
    // "notes" moved from s2 to s1, "solo" is for s2 and "dev" was deleted.
    let list = [ws("notes", Some("s1")), ws("solo", Some("s2"))];
    assert!(keep_valid(&mut d, &list));
    assert_eq!(d.workspace_wallpapers, BTreeMap::from([("solo".to_string(), set(&[("s2", "b.jpg")]))]));
    // Nothing left to drop.
    assert!(!keep_valid(&mut d, &list));
}

#[test]
fn only_a_choice_a_screen_shows_now_ends_the_hold() {
    let mut d = desk();
    d.screen_in_use.insert("s2".into(), ScreenInUse { workspace: Some("notes".into()), previous: None });
    // The desk default, and the workspace in use on the screen.
    assert!(shows_now(&d, None, "s1"));
    assert!(shows_now(&d, Some("dev"), "s1"));
    assert!(shows_now(&d, Some("notes"), "s2"));
    // A workspace that isn't in use there changes nothing on screen.
    assert!(!shows_now(&d, Some("dev"), "s2"));
    assert!(!shows_now(&d, Some("other"), "s1"));
}

#[test]
fn retry_forgets_a_screens_failures_or_all() {
    let mut failures: Failures = BTreeMap::from([
        ("k1".to_string(), ("s1".to_string(), UNREADABLE)),
        ("k2".to_string(), ("s2".to_string(), NO_CODEC)),
    ]);
    retry(&mut failures, Some("s1"));
    assert_eq!(failures.keys().collect::<Vec<_>>(), ["k2"]);
    retry(&mut failures, Some("s3"));
    assert_eq!(failures.len(), 1);
    retry(&mut failures, None);
    assert!(failures.is_empty());
}

#[test]
fn only_changed_screens_are_applied() {
    let applied = BTreeMap::from([("s1".to_string(), "a.png".to_string()), ("s2".to_string(), "b.png".to_string())]);
    let wanted = [
        ("s1".to_string(), Show::Picture("a.png".into())),
        ("s2".to_string(), Show::Picture("c.png".into())),
        ("s3".to_string(), Show::Own),
    ];
    let got: Vec<&str> = to_apply(&applied, &wanted).iter().map(|(s, _)| s.as_str()).collect();
    // s1 shows it already, and s3 was never changed, so it still shows mine.
    assert_eq!(got, ["s2"]);
    let back = [("s1".to_string(), Show::Own)];
    assert_eq!(to_apply(&applied, &back).len(), 1);
}

fn area(x: u32, y: u32, w: u32, h: u32) -> Area {
    Area { x, y, w, h }
}

#[test]
fn fill_covers_and_crops_centred() {
    // A 4:3 picture on a 16:9 screen: as wide as the screen, the top and bottom cut off evenly.
    let p = fit_rect((4000, 3000), (1920, 1080), Fit::Fill);
    assert_eq!(p.scaled, (1920, 1440));
    assert_eq!(p.piece, Piece { from: area(0, 180, 1920, 1080), to: (0, 0) });
    // A wide one: as tall as the screen, the sides cut off.
    let p = fit_rect((3000, 1000), (1920, 1080), Fit::Fill);
    assert_eq!(p.scaled, (3240, 1080));
    assert_eq!(p.piece.from, area(660, 0, 1920, 1080));
}

#[test]
fn fit_letterboxes() {
    let p = fit_rect((4000, 3000), (1920, 1080), Fit::Fit);
    assert_eq!(p.scaled, (1440, 1080));
    assert_eq!(p.piece, Piece { from: area(0, 0, 1440, 1080), to: (240, 0) });
    let p = fit_rect((3000, 1000), (1920, 1080), Fit::Fit);
    assert_eq!(p.scaled, (1920, 640));
    assert_eq!(p.piece.to, (0, 220));
}

#[test]
fn center_crops_when_bigger() {
    let p = fit_rect((800, 600), (1920, 1080), Fit::Center);
    assert_eq!(p.scaled, (800, 600));
    assert_eq!(p.piece, Piece { from: area(0, 0, 800, 600), to: (560, 240) });
    let p = fit_rect((4000, 600), (1920, 1080), Fit::Center);
    assert_eq!(p.piece, Piece { from: area(1040, 0, 1920, 600), to: (0, 240) });
}

#[test]
fn stretch_is_exact() {
    let p = fit_rect((4000, 3000), (1920, 1080), Fit::Stretch);
    assert_eq!(p.scaled, (1920, 1080));
    assert_eq!(p.piece, Piece { from: area(0, 0, 1920, 1080), to: (0, 0) });
}

#[test]
fn tile_starts_top_left() {
    let p = fit_rect((800, 600), (1920, 1080), Fit::Tile);
    let pieces: Vec<Piece> = p.pieces().collect();
    assert_eq!(pieces.len(), 6);
    assert_eq!(pieces[0], Piece { from: area(0, 0, 800, 600), to: (0, 0) });
    assert_eq!(pieces[2], Piece { from: area(0, 0, 320, 600), to: (1600, 0) });
    assert_eq!(pieces[5], Piece { from: area(0, 0, 320, 480), to: (1600, 600) });
    // Bigger than the screen: one piece, its top left.
    assert_eq!(fit_rect((3000, 2000), (1920, 1080), Fit::Tile).pieces().count(), 1);
}

#[test]
fn portrait_screen_sizes() {
    // A landscape picture on a portrait screen: Fill keeps the middle, Fit leaves bars above and below.
    let fill = fit_rect((3840, 2160), (1080, 1920), Fit::Fill);
    assert_eq!(fill.scaled, (3413, 1920));
    assert_eq!(fill.piece.from, area(1166, 0, 1080, 1920));
    let fit = fit_rect((3840, 2160), (1080, 1920), Fit::Fit);
    assert_eq!(fit.scaled, (1080, 608));
    assert_eq!(fit.piece.to, (0, 656));
    // Every piece of every fit stays on the screen.
    for fit in [Fit::Fill, Fit::Fit, Fit::Stretch, Fit::Center, Fit::Tile] {
        let p = fit_rect((3840, 2160), (1080, 1920), fit);
        assert!(p.pieces().all(|q| q.to.0 + q.from.w <= 1080 && q.to.1 + q.from.h <= 1920), "{fit:?}");
        assert!(p.piece.from.x + p.piece.from.w <= p.scaled.0 && p.piece.from.y + p.piece.from.h <= p.scaled.1);
    }
}

#[test]
fn compose_puts_bars_round_a_fitted_picture() {
    let p = fit_rect((1, 1), (3, 1), Fit::Center);
    // One half see-through red pixel, over blue bars.
    let canvas = compose(&p, &[0, 0, 128, 128], [255, 0, 0]);
    assert_eq!(canvas, [255, 0, 0, 255, 127, 0, 128, 255, 255, 0, 0, 255]);
}

#[test]
fn cache_key_changes_with_file_fit_and_size() {
    let key = |path: &str, len, written, fit, size| cache_key(path, len, written, fit, 0, size);
    let base = key(r"C:\a.jpg", 10, 1, Fit::Fill, (1920, 1080));
    assert_eq!(base, key(r"c:\A.JPG", 10, 1, Fit::Fill, (1920, 1080)));
    for other in [
        key(r"C:\b.jpg", 10, 1, Fit::Fill, (1920, 1080)),
        key(r"C:\a.jpg", 11, 1, Fit::Fill, (1920, 1080)),
        key(r"C:\a.jpg", 10, 2, Fit::Fill, (1920, 1080)),
        key(r"C:\a.jpg", 10, 1, Fit::Tile, (1920, 1080)),
        key(r"C:\a.jpg", 10, 1, Fit::Fill, (1080, 1920)),
        cache_key(r"C:\a.jpg", 10, 1, Fit::Fill, 0x00ff_ffff, (1920, 1080)),
    ] {
        assert_ne!(base, other);
    }
    // A changed file keeps the prefix, so its last render is found once the file is gone.
    assert!(base.starts_with(&cache_prefix(r"C:\a.jpg", Fit::Fill, 0, (1920, 1080))));
    assert!(key(r"C:\a.jpg", 99, 9, Fit::Fill, (1920, 1080)).starts_with(&base[..16]));
}

#[test]
fn evict_oldest_first_and_never_the_kept() {
    let file = |name: &str, len, used| Cached { path: PathBuf::from(name), len, used };
    let files = [file("a", 300, 1), file("b", 300, 2), file("c", 300, 3), file("d", 300, 4)];
    let keep = |p: &Path| p == Path::new("a");
    // 1200 over a cap of 700: a is kept, so b and c go.
    assert_eq!(evict(&files, 700, &keep), [PathBuf::from("b"), PathBuf::from("c")]);
    assert!(evict(&files, 1200, &keep).is_empty());
    // Nothing but kept files left: over the cap, and nothing goes.
    assert!(evict(&files[..1], 0, &keep).is_empty());
}

#[test]
fn extensions_accepted_and_refused() {
    for ok in [r"C:\a.JPG", "b.jpeg", "c.jfif", "d.png", "e.bmp", "f.dib", "g.gif", "h.TIFF", "i.webp", "j.heic"] {
        assert!(accepted_extension(ok), "{ok}");
    }
    for no in ["a.svg", "b.mp4", "c", r"C:\d.png\e", "jpg"] {
        assert!(!accepted_extension(no), "{no}");
    }
    assert_eq!(check_picture("a.svg", 10, b"<svg"), Err(NOT_A_PICTURE));
    assert_eq!(check_picture("a.png", 10, b""), Err(UNREADABLE));
    // An extension Seam takes but whose header it doesn't read goes to the worker.
    assert_eq!(check_picture("a.heic", 10, b"....ftypheic"), Ok(()));
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
    b.extend(b"IHDR");
    b.extend(w.to_be_bytes());
    b.extend(h.to_be_bytes());
    b
}

#[test]
fn header_size_over_limits_is_refused() {
    assert_eq!(header_size(&png(3840, 2160)), Some((3840, 2160)));
    let gif = [b"GIF89a".as_slice(), &[0x80, 0x07, 0x38, 0x04]].concat();
    assert_eq!(header_size(&gif), Some((1920, 1080)));
    let mut bmp = vec![b'B', b'M'];
    bmp.extend([0; 12]);
    bmp.extend(40i32.to_le_bytes());
    bmp.extend(640i32.to_le_bytes());
    bmp.extend((-480i32).to_le_bytes());
    assert_eq!(header_size(&bmp), Some((640, 480)));
    // A JPEG with an APP0 segment before its frame.
    let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0, 4, 0, 0, 0xff, 0xc0, 0, 17, 8, 0x04, 0x38, 0x07, 0x80];
    assert_eq!(header_size(&jpeg), Some((1920, 1080)));
    assert_eq!(header_size(b"RIFF....WEBP"), None);
    assert_eq!(check_picture("a.png", 10, &png(30_000, 100)), Err(TOO_BIG));
    assert_eq!(check_picture("a.png", 10, &png(15_000, 15_000)), Err(TOO_BIG));
    assert_eq!(check_picture("a.png", 10, &png(20_000, 10_000)), Ok(()));
    assert_eq!(check_picture("a.png", MAX_FILE + 1, &png(10, 10)), Err(TOO_BIG));
}
