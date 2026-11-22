//! The tests for the workspace logic.

use super::*;
use crate::config::ClosedApps;

fn keep(_: usize) -> ClosedApps {
    ClosedApps::Keep
}
fn remove(_: usize) -> ClosedApps {
    ClosedApps::Remove
}

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
    let merged = merge_capture(&old, fresh.clone(), &[], &none, &[], &keep).windows;
    let rows: Vec<(&str, bool, u32, bool)> =
        merged.iter().map(|r| (r.exe.as_str(), r.launch, r.delay_ms, r.wait_for_window)).collect();
    assert_eq!(rows, [(ff, false, 1500, true), (spotify, true, 0, false), ("notepad.exe", true, 0, false)]);
    assert_eq!(merged[0].target, Target::Zone { zone: 2, span_to: None });

    // With the setting on, the row with no window goes, and the others say where they were.
    let on = merge_capture(&old, fresh.clone(), &[], &none, &[], &remove);
    let exes: Vec<&str> = on.windows.iter().map(|r| r.exe.as_str()).collect();
    assert_eq!((exes, on.removed), ([ff, "notepad.exe"].to_vec(), 1));
    assert_eq!((on.from, on.rows), (vec![Some(0), None], vec![1, 0]));
    // It stays when its app has a window the capture made no row of, or nothing else is left.
    let other =
        Candidate { hwnd: 9, exe: spotify.into(), aumid: None, class: String::new(), title: String::new(), z: 0 };
    assert_eq!(merge_capture(&old, fresh.clone(), &[], &none, &[other], &remove).removed, 0);
    assert_eq!(merge_capture(&old, vec![rule("notepad.exe", None, "", "")], &[], &none, &[], &remove).removed, 0);

    // Two Explorer place rows, the first one's window closed: it goes, the second keeps its
    // binding at row 0, and Undo's map gives the old row back.
    let explorer = r"C:\Windows\explorer.exe";
    let (mut a, mut b) = (rule(explorer, None, "CabinetWClass", "A"), rule(explorer, None, "CabinetWClass", "B"));
    a.place = Some(Place::Folder(r"C:\A".into()));
    b.place = Some(Place::Folder(r"C:\B".into()));
    let old = Workspace { windows: vec![a, b], ..Workspace::default() };
    let open = rule(explorer, None, "CabinetWClass", "B");
    let bound = [Some(Bound::Gone), Some(Bound::Alive(0))];
    let merged = merge_capture(&old, vec![open], &bound, &none, &[], &remove);
    assert_eq!((merged.windows.len(), merged.removed, merged.from, merged.rows), (1, 1, vec![Some(1)], vec![0]));
    assert_eq!(merged.windows[0].place, old.windows[1].place);
    // Another Explorer window nobody holds doesn't keep the closed row.
    let extra = rule(explorer, None, "CabinetWClass", "C");
    let both =
        merge_capture(&old, vec![rule(explorer, None, "CabinetWClass", "B"), extra], &bound, &none, &[], &remove);
    assert_eq!((both.removed, both.from), (1, vec![Some(1), None]));

    // Stages end at a row that waits for its window.
    let mut waits: Vec<Rule> = (0..5).map(|_| rule(ff, None, "", "")).collect();
    (waits[1].wait_for_window, waits[4].wait_for_window) = (true, true);
    assert_eq!(stages(&waits, &[0, 1, 2, 3, 4]), vec![vec![0, 1], vec![2, 3, 4]]);
    assert_eq!(stages(&waits, &[0, 2]), vec![vec![0, 2]]);
    assert_eq!(stages(&waits, &[1, 4]), vec![vec![1], vec![4]]);
    assert_eq!((tame_delay(1200), tame_delay(99_000)), (1000, MAX_DELAY_MS));
}

/// Notepad is open; Spotify and Slack are not.
fn three_apps() -> (Workspace, Vec<Rule>) {
    let rules = vec![
        rule("notepad.exe", None, "", "Notes"),
        rule(r"C:\Spotify.exe", None, "", ""),
        rule(r"C:\Slack.exe", None, "", ""),
    ];
    let old = Workspace { windows: rules.clone(), ..Workspace::default() };
    (old, vec![rule("notepad.exe", None, "", "Notes")])
}

fn turn_off(_: usize) -> ClosedApps {
    ClosedApps::TurnOff
}

#[test]
fn turn_off_keeps_the_row_and_sets_off() {
    let (old, fresh) = three_apps();
    let merged = merge_capture(&old, fresh, &[], &HashSet::new(), &[], &turn_off);
    assert_eq!((merged.removed, merged.turned_off), (0, 2));
    let off: Vec<bool> = merged.windows.iter().map(|r| r.off).collect();
    assert_eq!(off, [false, true, true]);
    assert_eq!(merged.from, [Some(0), Some(1), Some(2)]);
    // A row already off isn't counted again.
    let mut old = old;
    old.windows[1].off = true;
    let (_, fresh) = three_apps();
    assert_eq!(merge_capture(&old, fresh, &[], &HashSet::new(), &[], &turn_off).turned_off, 1);
}

#[test]
fn per_row_choices_apply_to_their_rows() {
    let (old, fresh) = three_apps();
    let pick = |row: usize| if row == 1 { ClosedApps::TurnOff } else { ClosedApps::Remove };
    let merged = merge_capture(&old, fresh, &[], &HashSet::new(), &[], &pick);
    assert_eq!((merged.removed, merged.turned_off), (1, 1));
    let rows: Vec<(&str, bool)> = merged.windows.iter().map(|r| (r.exe.as_str(), r.off)).collect();
    assert_eq!(rows, [("notepad.exe", false), (r"C:\Spotify.exe", true)]);
    assert_eq!(merged.from, [Some(0), Some(1)]);
    // The choice for a row whose app is open is never asked for.
    let (old, fresh) = three_apps();
    let asked = std::cell::RefCell::new(Vec::new());
    let note = |row: usize| {
        asked.borrow_mut().push(row);
        ClosedApps::Keep
    };
    merge_capture(&old, fresh, &[], &HashSet::new(), &[], &note);
    assert_eq!(*asked.borrow(), [1, 2]);
}

#[test]
fn none_open_changes_nothing_whatever_the_choice() {
    let (old, _) = three_apps();
    for choice in [remove, turn_off, keep] {
        let merged = merge_capture(&old, vec![rule("calc.exe", None, "", "")], &[], &HashSet::new(), &[], &choice);
        assert_eq!((merged.removed, merged.turned_off), (0, 0));
        assert!(merged.windows[..3].iter().all(|r| !r.off));
        assert_eq!(merged.windows.len(), 4);
    }
}

#[test]
fn a_row_of_a_running_app_is_told_from_one_of_a_closed_app() {
    // Two Chrome windows saved, one open now, and Slack closed.
    let chrome = r"C:\Chrome.exe";
    let old = Workspace {
        windows: vec![
            rule(chrome, None, "", "Docs"),
            rule(chrome, None, "", "Hidden one"),
            rule(r"C:\Slack.exe", None, "", ""),
        ],
        ..Workspace::default()
    };
    let fresh = [rule(chrome, None, "", "Docs")];
    let none = HashSet::new();
    let rows = review::not_open(&old, &fresh, &[], &none, &[]);
    assert_eq!(rows, [1, 2]);
    assert_eq!(review::running(&old, &fresh, &[], &none, &rows), [true, false]);
}

#[test]
fn not_open_agrees_with_the_merge() {
    let (old, fresh) = three_apps();
    let none = HashSet::new();
    assert_eq!(review::not_open(&old, &fresh, &[], &none, &[]), [1, 2]);
    let removed = merge_capture(&old, fresh.clone(), &[], &none, &[], &remove);
    assert_eq!(removed.removed, 2);
    // A window of the app the capture made no row of keeps its row, in both.
    let slack = Candidate {
        hwnd: 9,
        exe: r"C:\Slack.exe".into(),
        aumid: None,
        class: String::new(),
        title: String::new(),
        z: 0,
    };
    assert_eq!(review::not_open(&old, &fresh, &[], &none, std::slice::from_ref(&slack)), [1]);
    assert_eq!(merge_capture(&old, fresh, &[], &none, &[slack], &remove).removed, 1);
    // With every app closed there is nothing to ask and nothing to change.
    let nothing = [rule("calc.exe", None, "", "")];
    assert!(review::not_open(&old, &nothing, &[], &none, &[]).is_empty());
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

// "Open with" reaches a program's command line or the shell, so what I type must be
// one full local path or an allowlisted link, and nothing that adds an argument.
#[test]
fn open_with_refuses_arguments_and_dangerous_schemes() {
    let path = |p: &str| Ok(OpenWith::Path(p.into()));
    let link = |l: &str| Ok(OpenWith::Link(l.into()));
    assert_eq!(safe_open_with(r"C:\Users\me\My Docs\a.md"), path(r"C:\Users\me\My Docs\a.md"));
    assert_eq!(safe_open_with(r"  D:\"), path(r"D:\"));
    assert_eq!(safe_open_with("HTTPS://Example.com/a?b=c#d"), link("https://Example.com/a?b=c#d"));
    assert_eq!(safe_open_with("obsidian://open?vault=Notes"), link("obsidian://open?vault=Notes"));
    assert_eq!(safe_open_with("vscode://file/C:/src/seam"), link("vscode://file/C:/src/seam"));
    assert_eq!(safe_open_with("vscode-insiders://file/C:/x"), link("vscode-insiders://file/C:/x"));
    assert_eq!(safe_open_with("spotify:playlist:37i9dQ"), link("spotify:playlist:37i9dQ"));
    // A user name or password goes, and a `@` later in the link stays.
    assert_eq!(safe_open_with("https://me:pw@a.com/x"), link("https://a.com/x"));
    assert_eq!(safe_open_with("http://a@b@c.com"), link("http://c.com"));
    assert_eq!(safe_open_with("https://a.com/@me"), link("https://a.com/@me"));

    let refused = [
        // Arguments and options.
        "",
        "   ",
        "-flag",
        "--new-window",
        "/c calc",
        r#"C:\a" --evil "b"#,
        r#"C:\a""#,
        "C:\\a\nb",
        "C:\\a\0b",
        // Paths that aren't one full local path.
        r"\\server\share\x",
        r"\\?\C:\x",
        r"\\.\pipe\x",
        r"C:\a\..\b",
        r"C:\..",
        "C:relative",
        "C:/a/b",
        "C:",
        "relative\\path.txt",
        "notepad.exe",
        "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}",
        // Schemes off the allowlist.
        "file:///C:/Windows/System32/calc.exe",
        "javascript:alert(1)",
        "data:text/html,<b>x</b>",
        "ms-msdt:/id PCWDiagnostic",
        "search-ms:query=x",
        "shell:startup",
        "cmd:/c calc",
        "ftp://a.com/x",
        "mailto:a@b.c",
        "x:y",
        // Allowlisted schemes, malformed.
        "https://",
        "https:///x",
        "https://me@",
        "https:a.com",
        "https://a b.com",
        "https://a.com/\"x",
        r"https://a.com\@evil.com",
        "obsidian://",
        "spotify:",
        // % expands in the shell, and a stream, a device or a trailing dot or space isn't a plain file.
        "%APPDATA%",
        r"C:\%TEMP%\x",
        "https://x/?%USERNAME%",
        "https://x/?%2",
        "https://x/?%zz",
        "https://x/%",
        r"C:\a\100%",
        r"C:\a\%41",
        r"C:\a\COM0",
        r"C:\a\LPT0.txt",
        "C:\\a\\COM\u{b9}",
        "C:\\a\\lpt\u{b2}.txt",
        "C:\\a\\COM\u{b3}",
        "https://a.com/\u{200b}x",
        "https://a.com/x\u{202e}gpj",
        "https://a.com/\u{ad}x",
        "https://a.com/\u{feff}",
        "https://a.com/\u{2066}x",
        "obsidian://open?vault=%USERPROFILE%",
        r"C:\a\b.txt:evil",
        r"C:\a:b",
        r"C:\a\CON",
        r"C:\a\nul.txt",
        r"C:\COM1\x",
        r"C:\lpt9.log",
        r"C:\a\Aux .txt",
        r"C:\a\CONIN$",
        r"C:\a\conout$",
        r"C:\a\dir.\x",
        r"C:\a\dir \x",
        r"C:\a\file.",
        // Whitespace that isn't a space.
        "https://a.com/\tx",
        "https://a.com/\u{a0}x",
        "https://a.com/\u{2003}x",
        "https://a.com/\u{3000}x",
    ];
    for text in refused {
        assert!(safe_open_with(text).is_err(), "{text:?} was let through");
    }
    assert!(safe_open_with(&format!("https://a.com/{}", "x".repeat(1024))).is_err());
    assert!(safe_open_with(&format!(r"C:\{}", "x".repeat(1024))).is_err());
    assert!(safe_open_with(&format!(r"C:\{}", "x".repeat(1000))).is_ok());
    // A % is fine in a link only as an escape.
    assert!(safe_open_with("obsidian://open?vault=My%20Notes").is_ok());
    assert!(safe_open_with("https://a.com/%41%7e").is_ok());
    // Names that only look like a device are fine.
    assert!(safe_open_with(r"C:\a\console\com10\lpt\nulled.txt").is_ok());
}

// A path goes to the row's program as its argument, so a program that runs what it's
// given, one that starts by package id, and Explorer given a file are refused.
#[test]
fn open_with_a_path_needs_a_program_that_only_opens_it() {
    let on = |exe: &str, aumid: Option<&str>| rule(exe, aumid, "c", "t");
    let dir = std::env::temp_dir();
    let dir = dir.to_string_lossy().to_string();
    for exe in [
        r"C:\Windows\System32\cmd.exe",
        r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe",
        r"C:\Program Files\PowerShell\7\PWSH.EXE",
        r"C:\Windows\System32\wscript.exe",
        r"C:\Windows\System32\mshta.exe",
        r"C:\Windows\System32\rundll32.exe",
        r"C:\Python\python.exe",
        r"C:\Program Files\nodejs\node.exe",
        r"C:\Python312\python3.12.exe",
        r"C:\Windows\py.exe",
        r"C:\Tools\perl.exe",
        r"C:\Windows\System32\certutil.exe",
        r"C:\Program Files\PowerShell\7-preview\pwsh-preview.exe",
        r"C:\Windows\System32\OpenSSH\ssh.exe",
        r"C:\Program Files\Java\bin\javaw.exe",
    ] {
        assert!(open_with_for(&on(exe, None), r"C:\src\run.bat").is_err(), "{exe}");
        // A link isn't an argument to it.
        assert!(open_with_for(&on(exe, None), "https://example.com").is_ok(), "{exe}");
    }
    let code = on(r"C:\Program Files\Code\Code.exe", None);
    assert!(open_with_for(&code, r"C:\src\seam").is_ok());
    // Explorer opens a folder, and a file that exists is refused.
    let explorer = on(r"C:\Windows\explorer.exe", None);
    let file = std::env::temp_dir().join("seam-open-with-test.txt");
    std::fs::write(&file, "x").unwrap();
    assert!(open_with_for(&explorer, &file.to_string_lossy()).is_err());
    let _ = std::fs::remove_file(&file);
    assert!(open_with_for(&explorer, &dir).is_ok());
    // A package id row has no program to hand a path to, but a link works.
    let packaged = on("", Some("Pkg.App!App"));
    assert!(open_with_for(&packaged, r"C:\src\seam").is_err());
    assert!(open_with_for(&packaged, "spotify:track:x").is_ok());
}

#[test]
fn open_with_is_checked_on_edit_and_left_out_of_files() {
    let old = Workspace { id: "ws".into(), ..Workspace::default() };
    let mut typed = rule(r"C:\Program Files\Code\Code.exe", None, "Chrome_WidgetWin_1", "Code");
    typed.open_with = Some(r"  C:\src\seam  ".into());
    let mut blank = typed.clone();
    blank.open_with = Some("  ".into());
    let new = Workspace { windows: vec![typed.clone(), blank], ..old.clone() };
    let saved = checked_update(&old, new, &|_| 0).unwrap();
    assert_eq!(saved.windows[0].open_with.as_deref(), Some(r"C:\src\seam"));
    assert_eq!(saved.windows[1].open_with, None);

    typed.open_with = Some(r#"C:\src" --evil"#.into());
    let bad = Workspace { windows: vec![typed], ..old.clone() };
    assert!(checked_update(&old, bad, &|_| 0).is_err());

    let with = Workspace { windows: saved.windows.clone(), ..old };
    let value = serde_json::to_value(&with).unwrap();
    assert_eq!(value["windows"][0]["open_with"], r"C:\src\seam");
    let stripped = without_open_with(&[value]);
    assert!(stripped[0]["windows"][0].get("open_with").is_none());
    // An older file has none, and loads as none.
    let older: Rule = serde_json::from_str(r#"{"exe":"C:\\a.exe"}"#).unwrap();
    assert_eq!(older.open_with, None);
}

#[test]
fn a_row_out_of_a_stack_saves_no_stack_field() {
    // An older file has none and loads as none, which is no stack.
    let older: Rule = serde_json::from_str(r#"{"exe":"C:\\a.exe"}"#).unwrap();
    assert_eq!(older.stack, None);
    assert!(serde_json::to_value(&older).unwrap().get("stack").is_none());
    let stacked = Rule { stack: Some(1), ..older };
    let back: Rule = serde_json::from_value(serde_json::to_value(&stacked).unwrap()).unwrap();
    assert_eq!(back.stack, Some(1));
}

fn named(id: &str) -> Workspace {
    Workspace { id: id.into(), name: id.into(), ..Workspace::default() }
}

#[test]
fn workspace_n_is_counted_from_one_in_list_order() {
    let list = [named("a"), named("b"), named("c")];
    assert_eq!(numbered(&list, 1).map(|w| w.id.as_str()), Some("a"));
    assert_eq!(numbered(&list, 3).map(|w| w.id.as_str()), Some("c"));
    assert!(numbered(&list, 0).is_none());
    assert!(numbered(&list, 4).is_none());
    assert!(numbered(&[], 1).is_none());
}

#[test]
fn a_window_without_a_row_gets_one_at_the_end_with_launch_off() {
    let notepad = r"C:\Windows\notepad.exe";
    let old = Workspace { windows: vec![rule(r"C:\a.exe", None, "A", "a")], ..named("w") };
    let mut fresh = rule(notepad, None, "Notepad", "Untitled");
    fresh.launch = true;
    fresh.focused = true;
    fresh.open_with = Some(r"C:\x".into());
    fresh.target = Target::Zone { zone: 2, span_to: None };
    fresh.place = Some(Place::Folder(r"C:\src".into()));
    let added = add_window(&old, fresh, &[]);
    assert!(added.new);
    assert_eq!(added.row, 1);
    assert_eq!(added.workspace.windows.len(), 2);
    let row = &added.workspace.windows[1];
    assert_eq!(row.exe, notepad);
    assert_eq!(row.target, Target::Zone { zone: 2, span_to: None });
    // Launch off, no Open with, no focus taken from the workspace, and a place left for the tray to store after its checks.
    assert!(!row.launch && !row.focused);
    assert_eq!(row.open_with, None);
    assert_eq!(row.place, None);
    assert_eq!(added.place, Some(Place::Folder(r"C:\src".into())));
    // The rows that were there are untouched.
    assert_eq!(added.workspace.windows[0], old.windows[0]);
}

#[test]
fn a_window_that_has_a_row_updates_it_and_never_adds_a_duplicate() {
    let ff = r"C:\Mozilla Firefox\firefox.exe";
    let mut mine = rule(ff, None, "MozillaWindowClass", "GitHub");
    mine.launch = true;
    mine.delay_ms = 1500;
    mine.focused = true;
    mine.z = 0;
    mine.target = Target::Zone { zone: 3, span_to: None };
    mine.stack = Some(1);
    mine.open_with = Some(r"C:\x".into());
    mine.place = Some(Place::Url("https://a.example".into()));
    let other = rule(r"C:\b.exe", None, "B", "b");
    let old = Workspace { windows: vec![other.clone(), mine], ..named("w") };
    let mut fresh = rule(ff, None, "MozillaWindowClass", "GitHub - now");
    fresh.target = Target::Zone { zone: 3, span_to: None };
    fresh.z = 5;
    fresh.stack = Some(1);
    fresh.place = Some(Place::Url("https://b.example".into()));
    let added = add_window(&old, fresh, &[]);
    assert!(!added.new);
    assert_eq!(added.row, 1);
    assert_eq!(added.workspace.windows.len(), 2);
    let row = &added.workspace.windows[1];
    // Where it is now is taken ...
    assert_eq!(row.target, Target::Zone { zone: 3, span_to: None });
    assert_eq!(row.title_hint, "GitHub - now");
    assert_eq!(row.stack, Some(1));
    // ... and what the row was set up with stays, launch on included.
    assert!(row.launch && row.focused);
    assert_eq!((row.delay_ms, row.z), (1500, 0));
    assert_eq!(row.open_with.as_deref(), Some(r"C:\x"));
    assert_eq!(row.place, Some(Place::Url("https://a.example".into())));
    assert_eq!(added.place, Some(Place::Url("https://b.example".into())));
    assert_eq!(added.workspace.windows[0], other);
}

fn terminal_row(zone: u32) -> Rule {
    let mut r = rule(r"C:\wt.exe", None, "CASCADIA", "pwsh");
    r.target = Target::Zone { zone, span_to: None };
    r.stack = Some(1);
    r
}

#[test]
fn with_two_rows_of_one_app_the_row_for_this_window_is_the_one_updated() {
    let old = Workspace { windows: vec![terminal_row(1), terminal_row(2)], ..named("w") };
    // In zone 2, where the second row is: that row, not the first.
    let mut fresh = terminal_row(2);
    fresh.title_hint = "now".into();
    let added = add_window(&old, fresh, &[None, None]);
    assert!(!added.new);
    assert_eq!(added.row, 1);
    assert_eq!(added.workspace.windows[0], old.windows[0]);
    assert_eq!(added.workspace.windows[1].title_hint, "now");
    // The row bound to this exact window wins, even where the window moved to.
    let mut moved = terminal_row(3);
    moved.title_hint = "moved".into();
    let added = add_window(&old, moved, &[None, Some(Bound::Alive(0))]);
    assert_eq!((added.row, added.new), (1, false));
    assert_eq!(added.workspace.windows[1].target, Target::Zone { zone: 3, span_to: None });
    assert_eq!(added.workspace.windows[0], old.windows[0]);
}

#[test]
fn a_window_in_a_place_no_row_holds_gets_a_new_row_even_with_rows_of_its_app() {
    let old = Workspace { windows: vec![terminal_row(1), terminal_row(2)], ..named("w") };
    let added = add_window(&old, terminal_row(4), &[None, None]);
    assert!(added.new);
    assert_eq!(added.row, 2);
    assert_eq!(added.workspace.windows.len(), 3);
    assert_eq!(&added.workspace.windows[..2], &old.windows[..]);
}

#[test]
fn a_row_takes_the_stack_the_window_is_read_in_and_a_new_row_records_it() {
    let old = Workspace { windows: vec![terminal_row(1)], ..named("w") };
    let mut second = terminal_row(1);
    second.stack = Some(0);
    let same = add_window(&old, second, &[]);
    assert_eq!(same.workspace.windows[0].stack, Some(0));
    // Now in no stack: the row isn't in one either.
    let mut alone = terminal_row(1);
    alone.stack = None;
    assert_eq!(add_window(&old, alone, &[]).workspace.windows[0].stack, None);
    // A new row, in a place no row of two holds, records the stack too.
    let two = Workspace { windows: vec![terminal_row(1), terminal_row(2)], ..named("w") };
    let mut stacked = terminal_row(4);
    stacked.stack = Some(2);
    let added = add_window(&two, stacked, &[]);
    assert!(added.new);
    assert_eq!(added.workspace.windows[2].stack, Some(2));
}

#[test]
fn rows_are_matched_bound_then_only_row_then_same_place_then_new() {
    let at = |zone| Target::Zone { zone, span_to: None };
    // 1. Bound to this window wins over a row in the same place.
    let old = Workspace { windows: vec![terminal_row(1), terminal_row(2)], ..named("w") };
    let a = add_window(&old, terminal_row(2), &[Some(Bound::Alive(0)), None]);
    assert_eq!((a.row, a.new), (0, false));
    assert_eq!(a.workspace.windows[0].target, at(2));
    // 2. The only row of the app is updated, though the window moved.
    let one = Workspace { windows: vec![rule(r"C:\a.exe", None, "A", "a"), terminal_row(1)], ..named("w") };
    let b = add_window(&one, terminal_row(5), &[]);
    assert_eq!((b.row, b.new, b.workspace.windows.len()), (1, false, 2));
    assert_eq!(b.workspace.windows[1].target, at(5));
    // ... unless it is bound to another window.
    let c = add_window(&one, terminal_row(5), &[None, Some(Bound::Alive(-1))]);
    assert!(c.new);
    // 3. With several rows, the one in the same place.
    let d = add_window(&old, terminal_row(2), &[None, None]);
    assert_eq!((d.row, d.new), (1, false));
    // 4. Several rows and none in its place: a new row.
    let e = add_window(&old, terminal_row(7), &[None, None]);
    assert_eq!((e.row, e.new), (2, true));
}

#[test]
fn adding_is_not_refused_for_another_rows_open_with() {
    let mut stale = rule(r"C:\a.exe", None, "A", "a");
    stale.open_with = Some(r"C:\gone\nowhere.txt".into());
    let old = Workspace { windows: vec![stale], ..named("w") };
    let mut added = add_window(&old, rule(r"C:\b.exe", None, "B", "b"), &[]).workspace;
    // Open with, which the check clears when it can't be used, comes through untouched.
    let kept = checked_rows(&old, added.clone(), &|_| 0, Some(1)).expect("the add's own row is fine");
    assert_eq!(kept.windows[0].open_with, old.windows[0].open_with);
    assert_eq!(kept.windows.len(), 2);
    // The editor's own save still checks every row.
    added.windows[0].open_with = Some(r#"C:\src" --evil"#.into());
    assert!(checked_rows(&old, added.clone(), &|_| 0, None).is_err());
    assert!(checked_rows(&old, added, &|_| 0, Some(1)).is_ok());
}

#[test]
fn a_place_row_bound_to_another_window_leaves_this_one_a_row_of_its_own() {
    let explorer = r"C:\Windows\explorer.exe";
    let mut folder = rule(explorer, None, "CabinetWClass", "src");
    folder.place = Some(Place::Folder(r"C:\src".into()));
    let old = Workspace { windows: vec![folder], ..named("w") };
    let mut fresh = rule(explorer, None, "CabinetWClass", "docs");
    fresh.target = Target::Zone { zone: 2, span_to: None };
    // The row's own window is open, and it isn't the one being added.
    let bound = [Some(Bound::Alive(-1))];
    let added = add_window(&old, fresh, &bound);
    assert!(added.new);
    assert_eq!(added.workspace.windows.len(), 2);
}

#[test]
fn refusals_say_why_and_everything_else_means_no_taskbar_button() {
    assert!(why_not("seam").contains("Seam's own"));
    assert!(why_not("never include").contains("Left alone"));
    assert!(why_not("administrator").contains("administrator"));
    assert!(why_not("sticky").contains("out of workspace switches"));
    assert!(why_not("off screen").contains("screen"));
    for skip in ["tool window", "owned", "not in Alt+Tab", "no activate", "child"] {
        assert!(why_not(skip).contains("taskbar button"), "{skip}");
    }
    let all = ["seam", "never include", "administrator", "sticky", "off screen", "no exe", "tool window"];
    let texts: HashSet<&str> = all.iter().map(|s| why_not(s)).collect();
    assert_eq!(texts.len(), all.len());
}

#[test]
fn off_rows_and_open_windows_round_trip_and_old_files_are_unchanged() {
    // A workspace saved before either field existed serializes without them.
    let old = json!({
        "id": "w", "name": "w", "hotkey": null, "launch_missing": true, "other_windows": "leave",
        "show_all": false, "windows": [{"exe": "a.exe", "monitor": "m", "launch": true}]
    });
    let ws = parse(&old).unwrap();
    assert!(!ws.windows[0].off);
    assert_eq!(ws.open_windows, OpenWindows::Move);
    let written = to_value(&ws);
    assert!(written.get("open_windows").is_none());
    assert!(written["windows"][0].get("off").is_none());
    // Both are kept when set, and read back.
    let mut set = ws.clone();
    set.windows[0].off = true;
    set.open_windows = OpenWindows::Leave;
    let written = to_value(&set);
    assert_eq!(written["open_windows"], "leave");
    assert_eq!(written["windows"][0]["off"], true);
    assert_eq!(parse(&written), Some(set));
}

#[test]
fn an_off_row_is_neither_started_nor_placed() {
    let on = rule("a.exe", None, "", "");
    let off = Rule { off: true, ..on.clone() };
    assert_eq!(row_action(&on, None, OpenWindows::Move, false), RowAction::Start);
    assert_eq!(row_action(&off, None, OpenWindows::Move, false), RowAction::Leave);
    assert_eq!(row_action(&on, Some(1), OpenWindows::Move, false), RowAction::Place);
    assert_eq!(row_action(&off, Some(1), OpenWindows::Move, false), RowAction::Leave);
    // Not even when Snap back asks.
    assert_eq!(row_action(&off, Some(1), OpenWindows::Move, true), RowAction::Leave);
    // A row with launch off is placed but never started, which is not the same as off.
    let placed_only = Rule { launch: false, ..on };
    assert_eq!(row_action(&placed_only, None, OpenWindows::Move, false), RowAction::Leave);
    assert_eq!(row_action(&placed_only, Some(1), OpenWindows::Move, false), RowAction::Place);
    // An off row matches no window and takes none from another row.
    let wins = [win(1, "a.exe", None, "", "", 0)];
    let rules = [off.clone(), rule("a.exe", None, "", "")];
    assert_eq!(assign_bound(&rules, &wins, &[], &HashSet::new()), [None, Some(1)]);
    assert_eq!(assign_bound_all(&rules, &wins, &[], &HashSet::new()), [Some(1), None]);
}

#[test]
fn leave_where_they_are_places_only_started_windows() {
    let r = rule("a.exe", None, "", "");
    assert_eq!(row_action(&r, Some(1), OpenWindows::Leave, false), RowAction::Leave);
    assert_eq!(row_action(&r, None, OpenWindows::Leave, false), RowAction::Start);
}

#[test]
fn put_back_moves_open_windows_even_when_the_workspace_leaves_them() {
    let r = rule("a.exe", None, "", "");
    assert_eq!(row_action(&r, Some(1), OpenWindows::Leave, true), RowAction::Place);
}

#[test]
fn update_from_screen_keeps_off() {
    let off = Rule { off: true, ..rule("a.exe", None, "A", "a") };
    let old = Workspace { windows: vec![off], ..named("w") };
    let mut fresh = rule("a.exe", None, "A", "a");
    fresh.target = Target::Zone { zone: 2, span_to: None };
    let merged = merge_capture(&old, vec![fresh], &[], &HashSet::new(), &[], &keep);
    assert_eq!(merged.windows.len(), 1);
    // The row's place is updated though it is off, and it stays off.
    assert!(merged.windows[0].off);
    assert_eq!(merged.windows[0].target, Target::Zone { zone: 2, span_to: None });
}

#[test]
fn add_window_keeps_an_off_row_off() {
    let off = Rule { off: true, launch: false, ..rule("a.exe", None, "A", "a") };
    let old = Workspace { windows: vec![off], ..named("w") };
    let mut fresh = rule("a.exe", None, "A", "a");
    fresh.target = Target::Zone { zone: 3, span_to: None };
    let added = add_window(&old, fresh, &[]);
    assert!(!added.new);
    assert!(added.workspace.windows[0].off);
    assert_eq!(added.workspace.windows[0].target, Target::Zone { zone: 3, span_to: None });
}

#[test]
fn moving_a_workspace_changes_its_number() {
    let saved: Vec<Value> = ["a", "b", "c"].iter().map(|id| to_value(&named(id))).collect();
    let ids = |list: &[Value]| -> Vec<String> { crate::workspace::list(list).into_iter().map(|w| w.id).collect() };
    let moved = moved_to(&saved, "c", 1).unwrap();
    assert_eq!(ids(&moved), ["c", "a", "b"]);
    assert_eq!(numbered(&crate::workspace::list(&moved), 1).map(|w| w.id.as_str()), Some("c"));
    assert_eq!(ids(&moved_to(&saved, "a", 3).unwrap()), ["b", "c", "a"]);
    assert_eq!(ids(&moved_to(&saved, "b", 2).unwrap()), ["a", "b", "c"]);
    assert_eq!(moved_to(&saved, "x", 1), Err(MoveError::NotFound));
    assert_eq!(moved_to(&saved, "a", 0), Err(MoveError::OutOfRange));
    assert_eq!(moved_to(&saved, "a", 4), Err(MoveError::OutOfRange));
}

#[test]
fn moving_keeps_unknown_keys() {
    let mut first = to_value(&named("a"));
    first["from_the_future"] = json!({ "kept": [1, 2] });
    let saved = vec![first.clone(), json!("not a workspace"), to_value(&named("b"))];
    let moved = moved_to(&saved, "b", 1).unwrap();
    assert_eq!(moved.len(), 3);
    assert_eq!(moved[0]["id"], "b");
    // Moved as it was, not read and written again.
    assert!(moved.contains(&first));
    assert!(moved.contains(&json!("not a workspace")));
}

#[test]
fn all_in_place_ignores_off_rows() {
    let row = |rule_index, state| RowStatus { rule_index, state };
    // An off row neither makes the workspace drift nor stops it reading as in place.
    let mixed = [row(0, RowState::InPlace), row(1, RowState::Off)];
    assert!(all_in_place(&mixed));
    assert!(!drifted(&mixed));
    // With nothing but off rows there is nothing running, so it isn't taken for the one in use.
    let all_off = [row(0, RowState::Off), row(1, RowState::Off)];
    assert!(!all_in_place(&all_off));
    assert!(!drifted(&all_off));
}

#[test]
fn an_off_row_with_a_bound_window_frees_it_for_the_next_row() {
    let off = Rule { off: true, ..rule("a.exe", None, "", "") };
    let on = rule("a.exe", None, "", "");
    let wins = [win(7, "a.exe", None, "", "", 0)];
    let bound = [Some(Bound::Alive(7)), Some(Bound::Unbound)];
    let found = assign_bound(&[off, on], &wins, &bound, &HashSet::new());
    assert_eq!(found, [None, Some(7)]);
}

#[test]
fn moving_counts_only_readable_workspaces() {
    let saved = vec![to_value(&named("a")), json!("junk"), to_value(&named("b")), to_value(&named("c"))];
    let ids = |list: &[Value]| -> Vec<String> { list.iter().filter_map(parse).map(|w| w.id).collect() };
    // Number 2 is "b", the second readable one, though the junk entry sits between.
    assert_eq!(ids(&moved_to(&saved, "c", 2).unwrap()), ["a", "c", "b"]);
    assert_eq!(ids(&moved_to(&saved, "a", 3).unwrap()), ["b", "c", "a"]);
    assert_eq!(moved_to(&saved, "a", 4), Err(MoveError::OutOfRange));
}

#[test]
fn a_row_whose_screen_isnt_connected_is_reported() {
    let placed = Rule { target: Target::Zone { zone: 1, span_to: None }, ..rule("a.exe", None, "", "") };
    assert!(screen_gone(&placed, ["x", "y"].into_iter()));
    assert!(!screen_gone(&placed, ["x", "m"].into_iter()));
    // A row that sends its window nowhere has no screen to miss.
    let keeps = Rule { target: Target::State { state: WinState::Keep }, ..placed };
    assert!(!screen_gone(&keeps, ["x"].into_iter()));
}

const WT: &str = "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App";
const WT_EXE: &str = r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\wt.exe";

fn terminal(sealed: Option<&str>) -> Rule {
    let tab = Tab {
        profile: Some("PowerShell".into()),
        folder: Some(r"C:\Projects".into()),
        command_sealed: sealed.map(Into::into),
        ..Tab::default()
    };
    Rule {
        exe: WT_EXE.into(),
        aumid: Some(WT.into()),
        item: Some(Item::Terminal { tabs: vec![tab] }),
        ..Rule::default()
    }
}

fn folder_row() -> Rule {
    Rule {
        exe: r"C:\Windows\explorer.exe".into(),
        item: Some(Item::Folder { folder: r"C:\Projects\seam".into() }),
        ..Rule::default()
    }
}

#[test]
fn old_rows_without_item_are_unchanged() {
    let older: Rule = serde_json::from_str(r#"{"exe":"C:\\a.exe","open_with":"C:\\x"}"#).unwrap();
    assert_eq!(older.item, None);
    assert_eq!(older.open_with.as_deref(), Some(r"C:\x"));
    assert!(serde_json::to_value(&older).unwrap().get("item").is_none());
    let back: Rule = serde_json::from_value(serde_json::to_value(terminal(Some("c2Vh"))).unwrap()).unwrap();
    assert_eq!(back, terminal(Some("c2Vh")));
}

#[test]
fn unknown_item_kind_reads_as_none() {
    for item in [r#"{"kind":"hologram","tabs":3}"#, r#"{"kind":"folder"}"#, "3", "null"] {
        let row: Rule = serde_json::from_str(&format!(r#"{{"exe":"C:\\a.exe","item":{item}}}"#)).unwrap();
        assert_eq!(row.item, None, "{item}");
    }
    let row: Rule = serde_json::from_str(r#"{"exe":"C:\\a.exe","item":{"kind":"folder","folder":"C:\\p"}}"#).unwrap();
    assert_eq!(row.item, Some(Item::Folder { folder: r"C:\p".into() }));
}

#[test]
fn import_drops_startup_commands_and_keeps_checked_items() {
    let web = Rule {
        exe: r"C:\Windows\notepad.exe".into(),
        item: Some(Item::Web { urls: vec!["https://a.com".into()] }),
        ..Rule::default()
    };
    let windows = vec![terminal(Some("c2Vh")), folder_row(), web];
    let workspace = Workspace { id: "w".into(), name: "A".into(), windows, ..Workspace::default() };
    let file = Export { seam_workspace: Export::VERSION, workspace, screens: BTreeMap::new() };
    let got = import(file, &BTreeMap::new(), &[], &[]);
    let Some(Item::Terminal { tabs }) = &got.windows[0].item else { panic!("the terminal went") };
    assert_eq!((tabs[0].command_sealed.as_deref(), tabs[0].command.as_deref()), (None, None));
    assert_eq!(tabs[0].folder.as_deref(), Some(r"C:\Projects"));
    assert_eq!(got.windows[1].item, folder_row().item);
    // A browser item on a program that isn't a browser doesn't pass its check.
    assert_eq!(got.windows[2].item, None);
    assert!(got.windows.iter().all(|r| !r.launch));
    // The same for a whole settings file's list of workspaces.
    let ws = Workspace { id: "w".into(), windows: vec![terminal(Some("c2Vh")), folder_row()], ..Workspace::default() };
    let cleaned = clean_imported_items(&[to_value(&ws)]);
    assert!(cleaned[0]["windows"][0]["item"]["tabs"][0].get("command_sealed").is_none());
    assert_eq!(cleaned[0]["windows"][1]["item"]["kind"], "folder");
}

#[test]
fn exports_leave_out_startup_commands() {
    let mut clear = terminal(Some("c2Vh"));
    if let Some(Item::Terminal { tabs }) = &mut clear.item {
        tabs[0].command = Some("npm run dev".into());
    }
    let workspace = Workspace { id: "w".into(), windows: vec![clear.clone(), folder_row()], ..Workspace::default() };
    let settings = without_commands(&[to_value(&workspace)]);
    let text = serde_json::to_string(&settings).unwrap();
    assert!(!text.contains("command") && !text.contains("c2Vh") && !text.contains("npm"));
    assert!(text.contains("PowerShell") && text.contains(r"C:\\Projects\\seam"));
    // The single-workspace export strips the rows itself.
    let Some(Item::Terminal { tabs }) = clear.item.map(Item::without_commands) else { panic!() };
    assert_eq!(
        tabs[0],
        Tab { profile: Some("PowerShell".into()), folder: Some(r"C:\Projects".into()), ..Tab::default() }
    );
}

#[test]
fn update_clears_open_with_and_place_on_item_rows() {
    let old = Workspace { id: "ws".into(), ..Workspace::default() };
    let mut row = folder_row();
    row.open_with = Some(r"C:\src".into());
    row.place = Some(Place::Folder(r"C:\y".into()));
    let new = Workspace { windows: vec![row], ..old.clone() };
    let saved = checked_update(&old, new, &|_| 0).unwrap();
    assert_eq!((saved.windows[0].open_with.clone(), saved.windows[0].place.clone()), (None, None));
    assert!(saved.windows[0].item.is_some());
    // An item that fails its check refuses the save.
    let mut bad = folder_row();
    bad.item = Some(Item::Folder { folder: r"\\server\share".into() });
    assert!(checked_update(&old, Workspace { windows: vec![bad], ..old.clone() }, &|_| 0).is_err());
}

#[test]
fn client_sent_command_sealed_is_ignored() {
    let old = Workspace { id: "ws".into(), windows: vec![terminal(Some("mine"))], ..Workspace::default() };
    let sealed_of = |ws: &Workspace| match &ws.windows[0].item {
        Some(Item::Terminal { tabs }) => tabs[0].command_sealed.clone(),
        _ => panic!(),
    };
    // A value planted from elsewhere doesn't stay.
    let planted = Workspace { windows: vec![terminal(Some("planted"))], ..old.clone() };
    assert_eq!(sealed_of(&checked_update(&old, planted, &|_| 0).unwrap()), None);
    // The one this workspace holds does, so an edit of something else keeps a command.
    let same = Workspace { windows: vec![terminal(Some("mine"))], ..old.clone() };
    assert_eq!(sealed_of(&checked_update(&old, same, &|_| 0).unwrap()).as_deref(), Some("mine"));
    // A command in clear replaces it, sealed by the tray before it's saved.
    let mut typed = terminal(Some("planted"));
    if let Some(Item::Terminal { tabs }) = &mut typed.item {
        tabs[0].command = Some("  npm run dev ".into());
    }
    let new = Workspace { windows: vec![typed], ..old.clone() };
    let mut saved = checked_update(&old, new, &|_| 0).unwrap();
    assert_eq!(sealed_of(&saved), None);
    seal_commands(&mut saved, &|c| Some(format!("sealed:{c}"))).unwrap();
    assert_eq!(sealed_of(&saved).as_deref(), Some("sealed:npm run dev"));
    assert!(!serde_json::to_string(&saved).unwrap().contains("\"command\""));
    // A seal that fails refuses the save, so a command is never kept in clear.
    let mut again = Workspace { windows: vec![terminal(None)], ..old.clone() };
    if let Some(Item::Terminal { tabs }) = &mut again.windows[0].item {
        tabs[0].command = Some("ls".into());
    }
    assert!(seal_commands(&mut again, &|_| None).is_err());
}

#[test]
fn a_patch_seals_commands_and_drops_what_fails_its_check() {
    let mut typed = terminal(Some("planted"));
    if let Some(Item::Terminal { tabs }) = &mut typed.item {
        tabs[0].command = Some("npm run dev".into());
    }
    let mut broken = folder_row();
    broken.item = Some(Item::Folder { folder: "relative".into() });
    broken.open_with = Some(r"C:\x".into());
    let mut kept = folder_row();
    kept.open_with = Some(r"C:\x".into());
    let windows = vec![typed, broken, kept, terminal(Some("held"))];
    let mut list = vec![to_value(&Workspace { id: "w".into(), windows, ..Workspace::default() })];
    let (sealed, clear) = (HashSet::from(["held".to_string()]), HashSet::from(["npm run dev".to_string()]));
    let held = Held { sealed: &sealed, clear: &clear, stored: &[] };
    seal_patch(&mut list, &held, &|c| Some(format!("sealed:{c}"))).unwrap();
    let rows = &list[0]["windows"];
    assert_eq!(rows[0]["item"]["tabs"][0]["command_sealed"], "sealed:npm run dev");
    assert!(rows[0]["item"]["tabs"][0].get("command").is_none());
    assert!(rows[1].get("item").is_none());
    assert!(rows[2].get("open_with").is_none() && rows[2]["item"]["kind"] == "folder");
    assert_eq!(rows[3]["item"]["tabs"][0]["command_sealed"], "held");
    assert_eq!(sealed_in(&list), HashSet::from(["sealed:npm run dev".to_string(), "held".to_string()]));
}

#[test]
fn others_see_the_kind_of_an_item_and_the_panel_its_commands() {
    let windows = vec![terminal(Some("mine")), terminal(Some("foreign")), folder_row()];
    let list = [to_value(&Workspace { id: "w".into(), windows, ..Workspace::default() })];
    let others = serde_json::to_string(&kind_only(&list)).unwrap();
    assert!(others.contains(r#""item":{"kind":"terminal"}"#) && others.contains(r#""item":{"kind":"folder"}"#));
    assert!(!others.contains("mine") && !others.contains("Projects") && !others.contains("PowerShell"));
    let open = |s: &str| (s == "mine").then(|| "npm run dev".to_string());
    let panel = for_panel(&list, &open);
    assert_eq!(panel[0]["windows"][0]["item"]["tabs"][0]["command"], "npm run dev");
    assert!(panel[0]["windows"][0]["item"]["tabs"][0].get("command_sealed").is_none());
    assert_eq!(panel[0]["windows"][1]["item"]["tabs"][0]["unreadable"], true);
}

#[test]
fn a_window_added_by_hand_never_updates_an_item_row() {
    let item = folder_row();
    let old = Workspace { id: "w".into(), windows: vec![item.clone()], ..Workspace::default() };
    let fresh = Rule { exe: item.exe.clone(), class: "CabinetWClass".into(), monitor: "m".into(), ..Rule::default() };
    let added = add_window(&old, fresh, &[None]);
    assert!(added.new);
    assert_eq!(added.workspace.windows.len(), 2);
    assert_eq!(added.workspace.windows[0], item);
}

fn with_command(sealed: Option<&str>, command: Option<&str>) -> Rule {
    let mut row = terminal(sealed);
    if let Some(Item::Terminal { tabs }) = &mut row.item {
        tabs[0].command = command.map(Into::into);
    }
    row
}

fn tab_of(row: &Value) -> &Value {
    &row["item"]["tabs"][0]
}

#[test]
fn a_stored_clear_command_is_never_sealed_or_shown() {
    let stored = to_value(&Workspace {
        id: "w".into(),
        windows: vec![with_command(None, Some("calc"))],
        ..Workspace::default()
    });
    // On load it comes off, with the flag.
    let loaded = without_clear(std::slice::from_ref(&stored));
    assert!(tab_of(&loaded[0]["windows"][0]).get("command").is_none());
    // For the panel it isn't shown, and nothing sealed appears.
    let panel = for_panel(std::slice::from_ref(&stored), &|_| Some("x".into()));
    let tab = tab_of(&panel[0]["windows"][0]);
    assert!(tab.get("command").is_none() && tab.get("command_sealed").is_none());
    // A patch holding it seals nothing, because it isn't a command the panel was given.
    let mut list = vec![stored];
    let (sealed, clear) = (HashSet::new(), HashSet::new());
    let held = Held { sealed: &sealed, clear: &clear, stored: &[] };
    seal_patch(&mut list, &held, &|c| Some(format!("sealed:{c}"))).unwrap();
    let tab = tab_of(&list[0]["windows"][0]);
    assert!(tab.get("command").is_none() && tab.get("command_sealed").is_none());
    // An edit that isn't the panel's own seals no clear command.
    let mut ws = Workspace { id: "w".into(), windows: vec![with_command(None, Some("calc"))], ..Workspace::default() };
    without_clear_commands(&mut ws);
    seal_commands(&mut ws, &|_| panic!("nothing to seal")).unwrap();
    assert_eq!(ws.windows[0], terminal(None));
}

#[test]
fn an_unchanged_command_keeps_its_seal_through_a_patch() {
    // The panel was handed "ls", so a whole-list patch sends it back and it's sealed again.
    let sent =
        to_value(&Workspace { id: "w".into(), windows: vec![with_command(None, Some("ls"))], ..Workspace::default() });
    let (sealed, clear) = (HashSet::new(), HashSet::from(["ls".to_string()]));
    let held = Held { sealed: &sealed, clear: &clear, stored: &[] };
    let mut list = vec![sent];
    seal_patch(&mut list, &held, &|c| Some(format!("sealed:{c}"))).unwrap();
    assert_eq!(tab_of(&list[0]["windows"][0])["command_sealed"], "sealed:ls");
    // A seal that fails refuses the patch.
    let mut again = vec![to_value(&Workspace {
        id: "w".into(),
        windows: vec![with_command(None, Some("ls"))],
        ..Workspace::default()
    })];
    assert!(seal_patch(&mut again, &held, &|_| None).is_err());
}

#[test]
fn a_foreign_command_survives_the_panel_saving() {
    let old = Workspace { id: "ws".into(), windows: vec![terminal(Some("elsewhere"))], ..Workspace::default() };
    // The panel was told it's unreadable and sends the tab back with no command and no seal.
    let panel = for_panel(&[to_value(&old)], &|_| None);
    assert_eq!(tab_of(&panel[0]["windows"][0])["unreadable"], true);
    assert!(tab_of(&panel[0]["windows"][0]).get("command_sealed").is_none());
    let back: Workspace = serde_json::from_value(panel[0].clone()).unwrap();
    let saved = checked_update(&old, back, &|_| 0).unwrap();
    let Some(Item::Terminal { tabs }) = &saved.windows[0].item else { panic!() };
    assert_eq!(tabs[0].command_sealed.as_deref(), Some("elsewhere"));
    assert!(!tabs[0].unreadable);
}

#[test]
fn last_session_rows_never_carry_an_item() {
    let session = to_value(&Workspace {
        id: "s".into(),
        windows: vec![folder_row(), terminal(Some("x"))],
        ..Workspace::default()
    });
    let cleaned = without_items(std::slice::from_ref(&session));
    assert!(cleaned[0]["windows"].as_array().unwrap().iter().all(|r| r.get("item").is_none()));
    let config = json!({ "desks": { "d": { "last_session": session, "workspaces": [] } } });
    let seen = crate::api::config_for_others(&config);
    assert!(!seen["desks"]["d"]["last_session"].to_string().contains("item"));
}

#[test]
fn an_item_with_no_text_kind_is_dropped_for_others() {
    let rows = json!({ "windows": [{ "exe": "a", "item": { "kind": 3, "folder": "C:\\x" } }, { "exe": "b", "item": { "kind": "folder" } }] });
    let shown = kind_only(&[rows]);
    assert!(shown[0]["windows"][0].get("item").is_none());
    assert_eq!(shown[0]["windows"][1]["item"], json!({ "kind": "folder" }));
}

#[test]
fn rows_left_alone_by_an_add_are_still_settled() {
    let old =
        Workspace { id: "ws".into(), windows: vec![terminal(Some("mine")), folder_row()], ..Workspace::default() };
    let mut planted = terminal(Some("planted"));
    if let Some(Item::Terminal { tabs }) = &mut planted.item {
        tabs[0].unreadable = true;
    }
    let new = Workspace { windows: vec![planted, folder_row(), folder_row()], ..old.clone() };
    let saved = checked_rows(&old, new, &|_| 0, Some(2)).unwrap();
    let Some(Item::Terminal { tabs }) = &saved.windows[0].item else { panic!() };
    assert_eq!(tabs[0].command_sealed, None);
    assert!(!tabs[0].unreadable);
}

#[test]
fn terminal_commands_too_long_together_refuse_the_save() {
    let old = Workspace { id: "ws".into(), ..Workspace::default() };
    let long = "x".repeat(MAX_COMMAND);
    let mut row = terminal(None);
    row.item =
        Some(Item::Terminal { tabs: (0..8).map(|_| Tab { command: Some(long.clone()), ..Tab::default() }).collect() });
    let new = Workspace { windows: vec![row], ..old.clone() };
    assert!(checked_update(&old, new, &|_| 0).unwrap_err().contains("too long together"));
}

#[test]
fn a_semicolon_in_a_tab_folder_is_refused() {
    assert!(folder_ok(r"C:\a;b").is_err());
    let item = Item::Terminal { tabs: vec![Tab { folder: Some(r"C:\a;b".into()), ..Tab::default() }] };
    assert!(check_item(&item, "", Some(WT)).is_err());
}
