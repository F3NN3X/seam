//! The tests for the workspace logic.

use super::*;

fn rule(exe: &str, aumid: Option<&str>, class: &str, title: &str) -> Rule {
    Rule {
        exe: exe.into(),
        aumid: aumid.map(Into::into),
        class: class.into(),
        title_hint: title.into(),
        monitor: "m".into(),
        ..Rule::default()
    }
}

fn win(hwnd: isize, exe: &str, aumid: Option<&str>, class: &str, title: &str, z: usize) -> Candidate {
    Candidate { hwnd, exe: exe.into(), aumid: aumid.map(Into::into), class: class.into(), title: title.into(), z }
}

#[test]
fn app_rules_need_a_place_or_a_pin() {
    let keep = Target::State { state: WinState::Keep };
    let saved = |exe: &str, monitor: &str, target: Target, pin: Option<Pin>| {
        let rule = Rule { exe: exe.into(), monitor: monitor.into(), target, pin, ..Rule::default() };
        serde_json::to_value(rule).unwrap()
    };
    let zone = || Target::Zone { zone: 1, span_to: None };
    let list = [
        saved("a.exe", "m", zone(), None),
        saved("b.exe", "", keep.clone(), Some(Pin::Top)),
        saved("c.exe", "m", keep.clone(), None),
        saved("d.exe", "m", zone(), Some(Pin::Both)),
        saved("", "m", zone(), Some(Pin::Top)),
    ];
    let usable: Vec<String> = app_rules(&list).into_iter().map(|r| r.exe).collect();
    assert_eq!(usable, ["a.exe", "b.exe", "d.exe"]);
}

#[test]
fn rules_get_the_right_windows() {
    let ff = r"C:\Program Files\Mozilla Firefox\firefox.exe";
    let terminal = Some("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App");
    let rules = [
        rule(ff, None, "MozillaWindowClass", "GitHub"),
        rule(ff, None, "MozillaWindowClass", "Docs"),
        rule("x", terminal, "CASCADIA_HOSTING_WINDOW_CLASS", ""),
        rule(r"C:\Spotify.exe", None, "Chrome_WidgetWin_1", ""),
    ];
    let wins = [
        win(1, &ff.to_uppercase(), None, "MozillaWindowClass", "Docs — Mozilla Firefox", 0),
        win(2, ff, None, "MozillaWindowClass", "GitHub — Mozilla Firefox", 1),
        win(3, "other.exe", terminal, "CASCADIA_HOSTING_WINDOW_CLASS", "pwsh", 2),
    ];
    // The title picks which Firefox window each rule gets, the AUMID beats the
    // exe, and a rule with no window gets none.
    assert_eq!(assign(&rules, &wins), vec![Some(2), Some(1), Some(3), None]);

    // A window is claimed once: three rules, two windows.
    let same = [rule(ff, None, "", ""), rule(ff, None, "", ""), rule(ff, None, "", "")];
    let two = [win(1, ff, None, "", "a", 0), win(2, ff, None, "", "b", 1)];
    assert_eq!(assign(&same, &two), vec![Some(1), Some(2), None]);

    // The class breaks a tie before the title or the z-order does.
    let rules = [rule(ff, None, "Right", "")];
    let wins = [win(1, ff, None, "Wrong", "", 0), win(2, ff, None, "Right", "", 5)];
    assert_eq!(assign(&rules, &wins), vec![Some(2)]);

    // A rule with an AUMID never takes a window that has none.
    let rules = [rule(ff, terminal, "", "")];
    assert_eq!(assign(&rules, &[win(1, ff, None, "", "", 0)]), vec![None]);

    // A recapture matches the same way. A matched row keeps its launch details and
    // place, a row with no window stays, and a new window goes at the end.
    let spotify = r"C:\Spotify.exe";
    let mut docs = rule(ff, None, "MozillaWindowClass", "Docs");
    (docs.launch, docs.delay_ms, docs.wait_for_window) = (false, 1500, true);
    let old = Workspace { windows: vec![docs, rule(spotify, None, "", "")], ..Workspace::default() };
    let mut moved = rule(ff, None, "MozillaWindowClass", "Docs — Mozilla Firefox");
    moved.target = Target::Zone { zone: 2, span_to: None };
    let fresh = vec![rule("notepad.exe", None, "", ""), moved];
    let none = HashSet::new();
    let merged = merge_capture(&old, fresh.clone(), &[], &none, &[], false).windows;
    let rows: Vec<(&str, bool, u32, bool)> =
        merged.iter().map(|r| (r.exe.as_str(), r.launch, r.delay_ms, r.wait_for_window)).collect();
    assert_eq!(rows, [(ff, false, 1500, true), (spotify, true, 0, false), ("notepad.exe", true, 0, false)]);
    assert_eq!(merged[0].target, Target::Zone { zone: 2, span_to: None });

    // With the setting on, the row with no window goes, and the others say where they were.
    let on = merge_capture(&old, fresh.clone(), &[], &none, &[], true);
    let exes: Vec<&str> = on.windows.iter().map(|r| r.exe.as_str()).collect();
    assert_eq!((exes, on.removed), ([ff, "notepad.exe"].to_vec(), 1));
    assert_eq!((on.from, on.rows), (vec![Some(0), None], vec![1, 0]));
    // It stays when its app has a window the capture made no row of, or nothing else is left.
    let other =
        Candidate { hwnd: 9, exe: spotify.into(), aumid: None, class: String::new(), title: String::new(), z: 0 };
    assert_eq!(merge_capture(&old, fresh.clone(), &[], &none, &[other], true).removed, 0);
    assert_eq!(merge_capture(&old, vec![rule("notepad.exe", None, "", "")], &[], &none, &[], true).removed, 0);

    // Two Explorer place rows, the first one's window closed: it goes, the second keeps its
    // binding at row 0, and Undo's map gives the old row back.
    let explorer = r"C:\Windows\explorer.exe";
    let (mut a, mut b) = (rule(explorer, None, "CabinetWClass", "A"), rule(explorer, None, "CabinetWClass", "B"));
    a.place = Some(Place::Folder(r"C:\A".into()));
    b.place = Some(Place::Folder(r"C:\B".into()));
    let old = Workspace { windows: vec![a, b], ..Workspace::default() };
    let open = rule(explorer, None, "CabinetWClass", "B");
    let bound = [Some(Bound::Gone), Some(Bound::Alive(0))];
    let merged = merge_capture(&old, vec![open], &bound, &none, &[], true);
    assert_eq!((merged.windows.len(), merged.removed, merged.from, merged.rows), (1, 1, vec![Some(1)], vec![0]));
    assert_eq!(merged.windows[0].place, old.windows[1].place);
    // Another Explorer window nobody holds doesn't keep the closed row.
    let extra = rule(explorer, None, "CabinetWClass", "C");
    let both = merge_capture(&old, vec![rule(explorer, None, "CabinetWClass", "B"), extra], &bound, &none, &[], true);
    assert_eq!((both.removed, both.from), (1, vec![Some(1), None]));

    // Stages end at a row that waits for its window.
    let mut waits: Vec<Rule> = (0..5).map(|_| rule(ff, None, "", "")).collect();
    (waits[1].wait_for_window, waits[4].wait_for_window) = (true, true);
    assert_eq!(stages(&waits, &[0, 1, 2, 3, 4]), vec![vec![0, 1], vec![2, 3, 4]]);
    assert_eq!(stages(&waits, &[0, 2]), vec![vec![0, 2]]);
    assert_eq!(stages(&waits, &[1, 4]), vec![vec![1], vec![4]]);
    assert_eq!((tame_delay(1200), tame_delay(99_000)), (1000, MAX_DELAY_MS));
}

// "No zones" and "Keep as is" must not be confused, or a switch takes a screen's zones away.
#[test]
fn a_capture_records_no_zones_and_keep_as_is_stays_apart() {
    let rows = ZoneLayout::of(Template::Rows).unwrap();
    let old = BTreeMap::from([("gone".to_string(), json!(rows)), ("a".to_string(), json!(rows))]);
    let captured = capture_zones(&old, &[("a".into(), None), ("b".into(), Some(rows.clone()))]);
    assert_eq!(screen_layout(&captured["a"]), Some(None));
    assert_eq!(screen_layout(&captured["b"]), Some(Some(rows.clone())));
    // A screen the capture didn't see keeps its entry.
    assert_eq!(captured["gone"], json!(rows));
    // An entry this build can't read keeps the screen as it is.
    assert_eq!(screen_layout(&json!({ "template": "spiral" })), None);
    assert!(zones_match(&json!("?"), None));
    assert!(zones_match(&no_zones(), None) && !zones_match(&no_zones(), Some(&rows)));
    assert!(!zones_match(&captured["b"], None));
    // A tree deeper than the editor can make keeps the screen as it is, and an
    // import leaves it out.
    let deep = (0..=zones::MAX_DEPTH)
        .fold(json!("leaf"), |a, _| json!({ "split": "vertical", "ratio": 0.5, "a": a, "b": "leaf" }));
    let deep = json!({ "template": "custom", "tree": deep });
    assert_eq!(screen_layout(&deep), None);
    let mut ws = Workspace { id: "w".into(), ..Workspace::default() };
    ws.zones = BTreeMap::from([("m".to_string(), deep), ("n".to_string(), json!(rows))]);
    let here = BTreeMap::from(
        [("m", 1), ("n", 2)]
            .map(|(id, number)| (id.to_string(), ScreenInfo { number, name: String::new(), width: 0, height: 0 })),
    );
    let file = Export { seam_workspace: Export::VERSION, workspace: ws, screens: BTreeMap::new() };
    let imported = import(file, &here, &[], &[]);
    assert_eq!(imported.zones.keys().collect::<Vec<_>>(), ["n"]);
}

// What a place row matches decides which window is moved and which app is started
// (D336): its own window while open, none once closed, by app when not bound yet,
// and no other row takes a window bound to one.
#[test]
fn place_rows_keep_their_own_windows() {
    let chrome = r"C:\Chrome\chrome.exe";
    // Window 2 is the topmost, so plain matching would give it to the first row.
    let wins: Vec<Candidate> = [(1, 1), (2, 0), (3, 2), (4, 3)].map(|(h, z)| win(h, chrome, None, "", "", z)).into();
    let rules: Vec<Rule> = (0..5).map(|_| rule(chrome, None, "", "")).collect();
    let bound = [Some(Bound::Unbound), Some(Bound::Alive(2)), Some(Bound::Gone), None, Some(Bound::Alive(9))];
    let elsewhere = HashSet::from([1]);
    // The unbound row goes by app but skips window 2, bound here, and window 1, bound
    // in another workspace. Window 2 stays with its bound row. The closed one gets
    // nothing, so it's started. The plain row doesn't steal window 2 either. One bound
    // to a window that isn't a candidate gets none.
    assert_eq!(assign_bound(&rules, &wins, &bound, &elsewhere), vec![Some(3), Some(2), None, Some(1), None]);
    // With nothing bound, it's plain matching.
    assert_eq!(assign_bound(&rules, &wins, &[None; 5], &HashSet::new()), assign(&rules, &wins));
}

// A saved place is opened when a switch starts the app, so the editor and the API
// can keep one on its own app's row, but not make one up or move it to another app.
#[test]
fn an_edit_never_adds_a_place() {
    let (explorer, chrome) = (r"C:\Windows\explorer.exe", r"C:\Chrome\chrome.exe");
    let docs = Some(Place::Folder(r"C:\Docs".into()));
    let mut saved = rule(explorer, None, "CabinetWClass", "Docs");
    saved.place = docs.clone();
    let old =
        Workspace { id: "w".into(), windows: vec![saved.clone(), rule(chrome, None, "", "")], ..Workspace::default() };
    let mut moved = rule(chrome, None, "", "");
    moved.place = docs.clone();
    let mut other_class = rule(explorer, None, "Other", "");
    other_class.place = docs.clone();
    let mut made_up = saved.clone();
    made_up.place = Some(Place::Url("https://a.com".into()));
    let new = Workspace { windows: vec![saved, moved, other_class, made_up], ..old.clone() };
    let places: Vec<_> = checked_update(&old, new, &|_| 0).unwrap().windows.into_iter().map(|r| r.place).collect();
    assert_eq!(places, vec![docs, None, None, None]);
}

// A remembered folder or page is handed to Explorer or a browser, so only a local
// folder and an http or https page may get through, each as one argument.
#[test]
fn only_local_folders_and_web_pages_are_opened() {
    assert!(safe_folder(r"C:\Users\me\Documents") && safe_folder(r"D:\"));
    let refused = [r"\\server\share", r"\\?\C:\x", r"C:\a\..\b", "C:relative", "::{20D04FE0}", r#"C:\a"b"#, "C:/a"];
    assert!(refused.iter().all(|f| !safe_folder(f)), "{refused:?}");
    assert_eq!(explorer_arg(r"C:\My Files\"), r#""C:\My Files""#);
    assert_eq!(explorer_arg(r"C:\"), r"C:\");

    assert!(safe_url("https://example.com/a?b=c#d") && safe_url("HTTP://localhost:8080"));
    let refused = ["file:///C:/x", "javascript:alert(1)", "https://", "https:///x", "https://a b", "https://a\"b"];
    assert!(refused.iter().all(|u| !safe_url(u)), "{refused:?}");
    assert!(!safe_url(&format!("https://a.com/{}", "x".repeat(MAX_URL))));
    assert!(!safe_url("https://me:pw@a.com/") && safe_url("https://a.com/@me"));

    assert_eq!(page_of("https://github.com/x").as_deref(), Some("https://github.com/x"));
    assert_eq!(page_of("github.com/x?q=1#top").as_deref(), Some("https://github.com/x?q=1#top"));
    assert_eq!(page_of("localhost:3000/").as_deref(), Some("http://localhost:3000/"));
    assert_eq!(page_of("192.168.1.2/a").as_deref(), Some("http://192.168.1.2/a"));
    assert_eq!(page_of("intranet:8080/wiki").as_deref(), Some("http://intranet:8080/wiki"));
    // A query that could hold a secret goes as a whole, and a fragment with a value.
    assert_eq!(page_of("https://a.com/cb?x=1&Code=9#t=2").as_deref(), Some("https://a.com/cb"));
    assert_eq!(page_of("https://a.com/?q=code#access").as_deref(), Some("https://a.com/?q=code#access"));
    for bar in [
        "about:blank",
        "edge://settings",
        "chrome://newtab",
        "rust lang",
        "rust",
        "intranet/wiki",
        "javascript:a.b",
        "",
        "x:y.z",
    ] {
        assert_eq!(page_of(bar), None, "{bar}");
    }
}
