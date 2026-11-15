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
    let added = add_window(&old, fresh, &[], &HashSet::new());
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
    mine.open_with = Some(r"C:\x".into());
    mine.place = Some(Place::Url("https://a.example".into()));
    let other = rule(r"C:\b.exe", None, "B", "b");
    let old = Workspace { windows: vec![other.clone(), mine], ..named("w") };
    let mut fresh = rule(ff, None, "MozillaWindowClass", "GitHub - now");
    fresh.target = Target::Zone { zone: 3, span_to: None };
    fresh.monitor = "other screen".into();
    fresh.z = 5;
    fresh.place = Some(Place::Url("https://b.example".into()));
    let added = add_window(&old, fresh, &[], &HashSet::new());
    assert!(!added.new);
    assert_eq!(added.row, 1);
    assert_eq!(added.workspace.windows.len(), 2);
    let row = &added.workspace.windows[1];
    // Where it is now is taken ...
    assert_eq!(row.target, Target::Zone { zone: 3, span_to: None });
    assert_eq!(row.monitor, "other screen");
    // ... and what the row was set up with stays, launch on included.
    assert!(row.launch && row.focused);
    assert_eq!((row.delay_ms, row.z), (1500, 0));
    assert_eq!(row.open_with.as_deref(), Some(r"C:\x"));
    assert_eq!(row.place, Some(Place::Url("https://a.example".into())));
    assert_eq!(added.place, Some(Place::Url("https://b.example".into())));
    assert_eq!(added.workspace.windows[0], other);
}

#[test]
fn a_place_row_bound_to_another_window_leaves_this_one_a_row_of_its_own() {
    let explorer = r"C:\Windows\explorer.exe";
    let mut folder = rule(explorer, None, "CabinetWClass", "src");
    folder.place = Some(Place::Folder(r"C:\src".into()));
    let old = Workspace { windows: vec![folder], ..named("w") };
    let fresh = rule(explorer, None, "CabinetWClass", "docs");
    // The row's own window is open, and it isn't the one being added.
    let bound = [Some(Bound::Alive(-1))];
    let added = add_window(&old, fresh, &bound, &HashSet::new());
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
    for skip in ["tool window", "owned", "not in Alt+Tab", "no activate", "shell", "child"] {
        assert!(why_not(skip).contains("taskbar button"), "{skip}");
    }
    let all = ["seam", "never include", "administrator", "sticky", "off screen", "no exe", "tool window"];
    let mut texts: Vec<&str> = all.iter().map(|s| why_not(s)).collect();
    texts.dedup();
    assert_eq!(texts.len(), all.len());
}
