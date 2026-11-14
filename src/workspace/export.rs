//! Export, import, and the safety checks on what a file may open.

use super::*;

/// A screen as an export file describes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScreenInfo {
    /// Line Up number.
    pub number: u32,
    pub name: String,
    pub width: i32,
    pub height: i32,
}

/// What Export writes: the workspace and the screens its windows refer to.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Export {
    pub seam_workspace: u32,
    pub workspace: Workspace,
    /// Device path -> screen.
    pub screens: BTreeMap<String, ScreenInfo>,
}

impl Export {
    pub const VERSION: u32 = 1;
}

/// A program Seam may start: an absolute path on a drive letter (`C:\...`) that ends
/// in `.exe`. UNC, verbatim, relative, drive-relative and bare names are all refused,
/// and so is a `/` or a `..` anywhere. Whether the drive is a network drive is
/// checked where the program is started.
pub fn safe_exe(exe: &str) -> bool {
    let path = Path::new(exe);
    let mut parts = path.components();
    let on_drive = matches!(parts.next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)));
    on_drive
        && matches!(parts.next(), Some(Component::RootDir))
        && !exe.contains('/')
        && !path.components().any(|c| matches!(c, Component::ParentDir))
        && exe.to_lowercase().ends_with(".exe")
}

/// A folder Seam may open in Explorer: an absolute path on a drive letter, with no
/// `/`, `..`, `"` or control character. Whether it exists and isn't on a network
/// drive is checked where it's read and where it's opened.
pub fn safe_folder(folder: &str) -> bool {
    let path = Path::new(folder);
    let mut parts = path.components();
    let on_drive = matches!(parts.next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_)));
    on_drive
        && matches!(parts.next(), Some(Component::RootDir))
        && !folder.contains(['/', '"'])
        && !folder.chars().any(char::is_control)
        && !path.components().any(|c| matches!(c, Component::ParentDir))
}

/// Explorer's argument for a folder: quoted, without a trailing `\` that would
/// escape the quote. A drive's root has no space or comma, so it goes as it is.
pub fn explorer_arg(folder: &str) -> String {
    match folder.trim_end_matches('\\') {
        root if root.len() == 2 => format!(r"{root}\"),
        folder => format!("\"{folder}\""),
    }
}

/// A page Seam may open in a browser: an http or https address with a host, not too
/// long, and with no space, quote or control character, so it reaches the browser
/// as one argument. A user name or password before the host (`user@host`) is refused.
pub fn safe_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"));
    let authority = rest.and_then(|r| r.split(['/', '?', '#']).next()).unwrap_or_default();
    !authority.is_empty()
        && !authority.contains('@')
        && url.len() <= MAX_URL
        && !url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"')
}

/// Bits of a query parameter's name that say it can carry a sign-in or a secret. A
/// page with any such parameter is kept without its query.
const SECRET_PARTS: [&str; 13] =
    ["token", "secret", "key", "pass", "sig", "auth", "code", "sess", "cred", "otp", "jwt", "sid", "state"];

/// `%XX` escapes turned back into the characters they stand for.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let (mut out, mut i) = (Vec::with_capacity(bytes.len()), 0);
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
        match hex.and_then(|h| u8::from_str_radix(h, 16).ok()).filter(|_| bytes[i] == b'%') {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A page as it's stored: without its query when a parameter's name, decoded, has a
/// bit of `SECRET_PARTS` in it, and without its fragment when it holds a value.
/// Otherwise as it is.
fn without_secrets(page: &str) -> String {
    let (rest, fragment) = page.split_once('#').map_or((page, None), |(r, f)| (r, Some(f)));
    let (base, query) = rest.split_once('?').map_or((rest, None), |(b, q)| (b, Some(q)));
    let name = |param: &str| percent_decoded(param.split('=').next().unwrap_or_default()).to_lowercase();
    let secret = |param: &str| {
        let name = name(param);
        SECRET_PARTS.iter().any(|s| name.contains(s))
    };
    let mut kept = base.to_string();
    if let Some(query) = query.filter(|q| !q.split('&').any(secret)) {
        kept = format!("{kept}?{query}");
    }
    if let Some(fragment) = fragment.filter(|f| !f.contains('=')) {
        kept = format!("{kept}#{fragment}");
    }
    kept
}

/// The page a browser's address bar shows, as it's stored, or `None` for anything but
/// an http or https page. Chromium and Firefox hide the scheme, so a bare host gets it
/// back: `http://` for localhost, an IP address, or a name with no dot and a port, and
/// `https://` for a name with a dot. Other text with no dot, a search word, isn't a page.
pub fn page_of(bar: &str) -> Option<String> {
    let bar = bar.trim();
    let lower = bar.to_ascii_lowercase();
    let page = if lower.starts_with("http://") || lower.starts_with("https://") {
        bar.to_string()
    } else {
        let host = bar.split(['/', '?', '#']).next().unwrap_or_default();
        let (name, port) = host.split_once(':').map_or((host, None), |(n, p)| (n, Some(p)));
        let named = !name.is_empty() && !name.starts_with('.') && !name.ends_with('.');
        let plain = name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
        let port_ok = port.is_none_or(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
        let octets: Vec<&str> = name.split('.').collect();
        let ip = octets.len() == 4 && octets.iter().all(|o| !o.is_empty() && o.chars().all(|c| c.is_ascii_digit()));
        let localhost = name.eq_ignore_ascii_case("localhost");
        let dotless = !name.contains('.');
        if !(named && plain && port_ok) || dotless && !localhost && port.is_none() {
            return None;
        }
        let local = localhost || ip || dotless;
        format!("{}://{bar}", if local { "http" } else { "https" })
    };
    let page = without_secrets(&page);
    safe_url(&page).then_some(page)
}

/// Whether any of these saved workspaces remembers a folder or page.
pub fn has_places(saved: &[Value]) -> bool {
    let rows = |ws: &Value| ws.get("windows").and_then(Value::as_array).cloned().unwrap_or_default();
    saved.iter().any(|ws| rows(ws).iter().any(|r| r.get("place").is_some()))
}

/// "Forget saved folders and pages": the saved workspaces as stored, without them.
pub fn without_places(saved: &[Value]) -> Vec<Value> {
    let mut saved = saved.to_vec();
    for ws in &mut saved {
        let rows = ws.get_mut("windows").and_then(Value::as_array_mut);
        for row in rows.into_iter().flatten() {
            if let Some(fields) = row.as_object_mut() {
                fields.remove("place");
            }
        }
    }
    saved
}

/// An AppUserModelID is letters, digits, `.`, `_` and `-`, with an optional `!` and a second such part.
pub fn safe_aumid(aumid: &str) -> bool {
    let part = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    match aumid.split_once('!') {
        Some((a, b)) => part(a) && part(b),
        None => part(aumid),
    }
}

/// Bring an exported workspace onto this desk. A window's screen is found by
/// device path, then by number and model, then by number. A window with no
/// screen here stays where it is. A screen's layout goes to the screen found the
/// same way, without its link to a library layout on the other machine, and one
/// with no screen here is dropped. The shortcut is dropped, so it can't clash. A file
/// from someone else is untrusted: nothing launches until the user ticks it, other
/// windows are left alone, and a
/// program path or AUMID that isn't plainly safe is cleared.
pub fn import(file: Export, here: &BTreeMap<String, ScreenInfo>, names: &[String], ids: &[String]) -> Workspace {
    let find = |old: &str| -> Option<String> {
        let was = file.screens.get(old);
        if here.contains_key(old) {
            return Some(old.to_string());
        }
        let was = was?;
        let by = |same: &dyn Fn(&ScreenInfo) -> bool| here.iter().find(|(_, s)| same(s)).map(|(id, _)| id.clone());
        by(&|s| s.number == was.number && s.name == was.name).or_else(|| by(&|s| s.number == was.number))
    };
    let mut workspace = file.workspace;
    for rule in &mut workspace.windows {
        match find(&rule.monitor) {
            Some(id) => rule.monitor = id,
            None => rule.target = Target::State { state: WinState::Keep },
        }
    }
    let mut zones = BTreeMap::new();
    for (screen, saved) in std::mem::take(&mut workspace.zones) {
        let (Some(id), Some(mut layout)) = (find(&screen), clean_entry(&saved)) else { continue };
        if let Some(fields) = layout.as_object_mut() {
            fields.remove("custom");
        }
        zones.entry(id).or_insert(layout);
    }
    workspace.zones = zones;
    workspace.id = new_id(ids);
    workspace.name = unique_name(&workspace.name, names);
    for rule in &mut workspace.windows {
        rule.launch = false;
        if !safe_exe(&rule.exe) {
            rule.exe.clear();
        }
        rule.aumid = rule.aumid.take().filter(|a| safe_aumid(a));
        // A folder or page from a file is never opened (D98, D330).
        rule.place = None;
    }
    workspace.launch_missing = false;
    // Close and Minimize are the user's opt-in, so a file can't arm them.
    workspace.other_windows = OtherWindows::Leave;
    workspace.hotkey = None;
    workspace
}
