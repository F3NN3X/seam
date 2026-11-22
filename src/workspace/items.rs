//! Items a row starts instead of an app (D731): the checks on what I typed, and every
//! argument string a launch uses. Pure, so each is tested; the launch only passes them on.

use super::export::{LAUNCHER_PREFIXES, LAUNCHERS, MAX_OPEN_WITH, invisible, plain_path, without_credentials};
use super::terminal::{Profile, Shell, shell_of};
use super::*;

/// The most addresses in a Web links item, and tabs in a Terminal item.
pub const MAX_ADDRESSES: usize = 20;
pub const MAX_TABS: usize = 8;
/// The longest startup command, in characters.
pub const MAX_COMMAND: usize = 1024;
/// The longest command line a launch builds, in characters.
const MAX_LINE: usize = 8000;
/// The longest profile name kept.
const MAX_PROFILE: usize = 128;

/// Browsers that open addresses as tabs of one new window, by exe name. Chromium's first.
pub const BROWSERS: [&str; 7] =
    ["chrome.exe", "msedge.exe", "brave.exe", "vivaldi.exe", "opera.exe", "helium.exe", "firefox.exe"];

/// Editors Seam looks for, by exe stem (D739).
pub const EDITORS: [&str; 6] = ["code", "code - insiders", "cursor", "vscodium", "windsurf", "zed"];

/// Windows Terminal's AUMID, and its Preview's.
pub const TERMINAL_AUMIDS: [&str; 2] =
    ["Microsoft.WindowsTerminal_8wekyb3d8bbwe!App", "Microsoft.WindowsTerminalPreview_8wekyb3d8bbwe!App"];

const NOT_LOCAL: &str = r"Use a full path on this PC's own drive, like C:\Projects";
const NOT_WEB: &str = "Use a web address starting with http:// or https://";
const BAD_COMMAND: &str = "A command can't have line breaks or hidden characters";
const CMD_QUOTES: &str = "A Command Prompt command can't have quotes";

fn exe_name(exe: &str) -> String {
    exe.rsplit('\\').next().unwrap_or(exe).to_lowercase()
}

fn exe_stem(exe: &str) -> String {
    let name = exe_name(exe);
    name.strip_suffix(".exe").map_or(name.clone(), str::to_string)
}

/// A folder as it's stored, or why not: a full path on a drive letter, as `safe_open_with`
/// takes one, with no `%`. Whether it exists, and is on a drive of this PC, is checked where
/// it's opened.
pub fn folder_ok(folder: &str) -> Result<String, &'static str> {
    let folder = folder.trim();
    let plain = folder.len() <= MAX_OPEN_WITH && !folder.contains('%') && !folder.chars().any(invisible);
    if !plain || !safe_folder(folder) {
        return Err(NOT_LOCAL);
    }
    plain_path(folder)?;
    Ok(folder.to_string())
}

/// An address as it's stored, or why not: http or https only, with a host, with no space,
/// quote, backslash or control character, and without a user name or password.
pub fn address_ok(address: &str) -> Result<String, &'static str> {
    let address = address.trim();
    let lower = address.to_ascii_lowercase();
    let web = lower.starts_with("http://") || lower.starts_with("https://");
    if !web || address.contains('\\') || address.chars().any(invisible) {
        return Err(NOT_WEB);
    }
    let clean = without_credentials(address).ok_or(NOT_WEB)?;
    safe_url(&clean).then_some(clean).ok_or(NOT_WEB)
}

/// A startup command as it's stored: one line, with no control or invisible character, and
/// `None` when it's blank.
pub fn command_ok(command: &str) -> Result<Option<String>, &'static str> {
    if command.chars().any(|c| c.is_control() || invisible(c)) {
        return Err(BAD_COMMAND);
    }
    let command = command.trim();
    if command.chars().count() > MAX_COMMAND {
        return Err("A command can have up to 1,024 characters");
    }
    Ok((!command.is_empty()).then(|| command.to_string()))
}

fn profile_ok(name: &str) -> Result<String, &'static str> {
    let name = name.trim();
    let bad = name.is_empty()
        || name.chars().count() > MAX_PROFILE
        || name.contains(['"', ';'])
        || name.ends_with('\\')
        || name.chars().any(|c| c.is_control() || invisible(c));
    if bad { Err("That profile name can't be used") } else { Ok(name.to_string()) }
}

/// The browser's arguments for these addresses as tabs of one new window. Chromium takes
/// them all after `--new-window`, Firefox needs `-new-tab` for each after the first.
pub fn web_args(browser: &str, urls: &[String]) -> Result<String, &'static str> {
    if urls.is_empty() {
        return Err("Add at least one address");
    }
    if urls.len() > MAX_ADDRESSES {
        return Err("Up to 20 addresses");
    }
    let line = match (exe_name(browser).as_str(), urls) {
        ("firefox.exe", [first, rest @ ..]) => {
            let tabs: String = rest.iter().map(|u| format!(" -new-tab {u}")).collect();
            format!("-new-window {first}{tabs}")
        }
        (_, urls) => format!("--new-window {}", urls.join(" ")),
    };
    if line.chars().count() > MAX_LINE { Err("Those addresses are too long together") } else { Ok(line) }
}

/// An editor's one argument: the folder, quoted.
pub fn editor_args(folder: &str) -> String {
    explorer_arg(folder)
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            match i <= chunk.len() {
                true => out.push(BASE64[(n >> (18 - 6 * i) & 63) as usize] as char),
                false => out.push('='),
            }
        }
    }
    out
}

/// The bytes of base64 text, or `None` for anything that isn't padded base64.
pub fn unbase64(text: &str) -> Option<Vec<u8>> {
    let text = text.as_bytes();
    if !text.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for (at, quad) in text.chunks(4).enumerate() {
        let last = at == text.len() / 4 - 1;
        let pad = quad.iter().rev().take_while(|b| **b == b'=').count();
        if pad > 2 || pad > 0 && !last {
            return None;
        }
        let mut n = 0u32;
        for b in &quad[..4 - pad] {
            n = n << 6 | BASE64.iter().position(|c| c == b)? as u32;
        }
        n <<= 6 * pad as u32;
        out.extend((0..3 - pad).map(|i| (n >> (16 - 8 * i)) as u8));
    }
    Some(out)
}

/// PowerShell's `-EncodedCommand` value: the command as UTF-16LE, in base64, so nothing in
/// it needs quoting and no `;` or `"` can end it early.
pub fn encoded_command(command: &str) -> String {
    let bytes: Vec<u8> = command.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64(&bytes)
}

/// A Command Prompt command as Windows Terminal reads it: each `;` is escaped, because it
/// would end the tab's arguments, and a quote is refused (Q2).
pub fn cmd_escape(command: &str) -> Result<String, &'static str> {
    if command.contains('"') {
        return Err(CMD_QUOTES);
    }
    Ok(command.replace(';', r"\;"))
}

/// The arguments for `wt.exe`: a new window with one `new-tab` for each tab, joined by ` ; `.
/// `commands[i]` is tab `i`'s unsealed command. A command goes in only for a tab whose
/// profile is in `profiles` and runs PowerShell or Command Prompt; a profile that isn't
/// there any more opens the default profile with no command.
pub fn terminal_args(tabs: &[Tab], profiles: &[Profile], commands: &[Option<String>]) -> Result<String, &'static str> {
    if tabs.is_empty() {
        return Err("Add at least one tab");
    }
    if tabs.len() > MAX_TABS {
        return Err("Up to 8 tabs");
    }
    let mut parts = Vec::with_capacity(tabs.len());
    for (i, tab) in tabs.iter().enumerate() {
        let profile = tab.profile.as_deref().and_then(|name| profiles.iter().find(|p| p.name == name));
        let mut part = String::from("new-tab");
        if let Some(profile) = profile {
            part += &format!(" -p \"{}\"", profile_ok(&profile.name)?);
        }
        if let Some(folder) = &tab.folder {
            part += &format!(" -d {}", explorer_arg(&folder_ok(folder)?));
        }
        if let (Some(profile), Some(Some(command))) = (profile, commands.get(i)) {
            let command = command_ok(command)?.unwrap_or_default();
            match (shell_of(profile), command.is_empty()) {
                (_, true) | (Shell::Other, _) => {}
                (Shell::PowerShell(exe), false) => {
                    part += &format!(" {exe} -NoExit -EncodedCommand {}", encoded_command(&command))
                }
                (Shell::Cmd, false) => part += &format!(" cmd.exe /k {}", cmd_escape(&command)?),
            }
        }
        parts.push(part);
    }
    let line = format!("-w new {}", parts.join(" ; "));
    if line.chars().count() > MAX_LINE { Err("That terminal is too long") } else { Ok(line) }
}

/// An item as it's stored, or why not, for the row it's on: the row's app must be the one the
/// kind needs, every address, folder, profile and command must pass its check, and the lists
/// must be within their limits. A Terminal tab's sealed command and its flags are left as they are.
pub fn check_item(item: &Item, exe: &str, aumid: Option<&str>) -> Result<Item, &'static str> {
    let app = exe_name(exe);
    let startable = safe_exe(exe);
    match item {
        Item::Folder { folder } => {
            if app != "explorer.exe" || !startable {
                return Err("Explorer opens a folder here");
            }
            Ok(Item::Folder { folder: folder_ok(folder)? })
        }
        Item::Editor { folder } => {
            let launcher =
                LAUNCHERS.contains(&exe_stem(exe).as_str()) || LAUNCHER_PREFIXES.iter().any(|p| app.starts_with(p));
            if !startable || launcher {
                return Err("That program can't be an editor here");
            }
            Ok(Item::Editor { folder: folder_ok(folder)? })
        }
        Item::Web { urls } => {
            if !startable || !BROWSERS.contains(&app.as_str()) {
                return Err("Pick a browser Seam opens pages in");
            }
            let urls = urls.iter().map(|u| address_ok(u)).collect::<Result<Vec<_>, _>>()?;
            web_args(exe, &urls)?;
            Ok(Item::Web { urls })
        }
        Item::Terminal { tabs } => {
            if !aumid.is_some_and(|a| TERMINAL_AUMIDS.contains(&a)) {
                return Err("A terminal needs Windows Terminal");
            }
            if tabs.is_empty() {
                return Err("Add at least one tab");
            }
            if tabs.len() > MAX_TABS {
                return Err("Up to 8 tabs");
            }
            let tab = |t: &Tab| -> Result<Tab, &'static str> {
                let profile = t.profile.as_deref().map(profile_ok).transpose()?;
                let folder = t.folder.as_deref().map(folder_ok).transpose()?;
                let command = t.command.as_deref().map(command_ok).transpose()?.flatten();
                Ok(Tab { profile, folder, command, ..t.clone() })
            };
            Ok(Item::Terminal { tabs: tabs.iter().map(tab).collect::<Result<_, _>>()? })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WT: &str = "Microsoft.WindowsTerminal_8wekyb3d8bbwe!App";

    fn profile(name: &str, line: Option<&str>) -> Profile {
        Profile { name: name.into(), commandline: line.map(Into::into), ..Profile::default() }
    }

    fn tab(name: &str, folder: Option<&str>) -> Tab {
        Tab { profile: Some(name.into()), folder: folder.map(Into::into), ..Tab::default() }
    }

    #[test]
    fn folder_items_take_only_local_full_paths() {
        for ok in [r"C:\Projects\seam", r"D:\", r"  C:\a b\c  "] {
            assert!(folder_ok(ok).is_ok(), "{ok}");
        }
        for bad in [
            r"\\server\share\x",
            r"C:\a\..\b",
            "C:/a/b",
            r"C:\100%",
            r"C:\a:stream",
            r"C:\CON",
            r"C:\x\nul.txt",
            r"C:\a.",
            "C:\\a\u{202e}b",
            "C:\\a\u{200b}",
            r"relative\path",
            r"C:",
            r"\\?\C:\x",
            "C:\\a\"b",
            "C:\\a\nb",
            "",
        ] {
            assert!(folder_ok(bad).is_err(), "{bad:?}");
        }
        let explorer = r"C:\Windows\explorer.exe";
        let folder = Item::Folder { folder: r"C:\Projects".into() };
        assert!(check_item(&folder, explorer, None).is_ok());
        assert!(check_item(&folder, r"C:\Windows\notepad.exe", None).is_err());
    }

    #[test]
    fn web_items_take_one_to_twenty_http_addresses() {
        let chrome = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
        let web = |urls: Vec<&str>| Item::Web { urls: urls.into_iter().map(Into::into).collect() };
        let ok =
            check_item(&web(vec!["https://user:pw@github.com/x", " http://localhost:3000 "]), chrome, None).unwrap();
        assert_eq!(ok, web(vec!["https://github.com/x", "http://localhost:3000"]));
        for bad in [
            "ftp://x.com",
            "file:///C:/x",
            "javascript:alert(1)",
            "vscode://file/x",
            "https://a b.com",
            "https://a.com/\"x",
            "https://a.com\\x",
            "https://",
            "github.com",
            "https://a.com/\u{202e}",
        ] {
            assert!(check_item(&web(vec![bad]), chrome, None).is_err(), "{bad}");
        }
        assert!(check_item(&web(vec![]), chrome, None).is_err());
        let many: Vec<String> = (0..21).map(|i| format!("https://a.com/{i}")).collect();
        assert_eq!(check_item(&Item::Web { urls: many[..20].to_vec() }, chrome, None).map(|_| ()), Ok(()));
        assert_eq!(check_item(&Item::Web { urls: many }, chrome, None), Err("Up to 20 addresses"));
        // Only a browser Seam knows, from a drive letter.
        assert!(check_item(&web(vec!["https://a.com"]), r"C:\x\notepad.exe", None).is_err());
        assert!(check_item(&web(vec!["https://a.com"]), r"\\s\x\chrome.exe", None).is_err());
    }

    #[test]
    fn web_args_for_chromium_and_firefox() {
        let urls = ["https://a.com".to_string(), "https://b.com/x?y=1".to_string(), "https://c.com".to_string()];
        assert_eq!(
            web_args(r"C:\x\msedge.exe", &urls).unwrap(),
            "--new-window https://a.com https://b.com/x?y=1 https://c.com"
        );
        assert_eq!(
            web_args(r"C:\x\Firefox.exe", &urls).unwrap(),
            "-new-window https://a.com -new-tab https://b.com/x?y=1 -new-tab https://c.com"
        );
        assert_eq!(web_args(r"C:\x\firefox.exe", &urls[..1]).unwrap(), "-new-window https://a.com");
    }

    #[test]
    fn web_args_are_capped() {
        let long = format!("https://a.com/{}", "x".repeat(2000));
        let urls = vec![long; 5];
        assert!(web_args(r"C:\x\chrome.exe", &urls).is_err());
        assert!(web_args(r"C:\x\chrome.exe", &[]).is_err());
    }

    #[test]
    fn editor_items_refuse_launchers() {
        let folder = Item::Editor { folder: r"C:\Projects".into() };
        for launcher in [
            r"C:\Windows\System32\cmd.exe",
            r"C:\x\pwsh.exe",
            r"C:\x\wt.exe",
            r"C:\x\powershell.exe",
            r"C:\x\python3.12.exe",
        ] {
            assert!(check_item(&folder, launcher, None).is_err(), "{launcher}");
        }
        assert!(check_item(&folder, r"C:\Users\me\AppData\Local\Programs\Microsoft VS Code\Code.exe", None).is_ok());
        assert!(check_item(&folder, r"\\s\x\Code.exe", None).is_err());
        assert_eq!(editor_args(r"C:\Projects\my app\"), "\"C:\\Projects\\my app\"");
    }

    #[test]
    fn terminal_args_quote_profiles_and_join_tabs() {
        let profiles = [profile("PowerShell", Some("pwsh.exe")), profile("Ubuntu", Some("wsl.exe"))];
        let tabs = [tab("PowerShell", Some(r"C:\Projects\seam")), tab("Ubuntu", None), Tab::default()];
        assert_eq!(
            terminal_args(&tabs, &profiles, &[None, None, None]).unwrap(),
            r#"-w new new-tab -p "PowerShell" -d "C:\Projects\seam" ; new-tab -p "Ubuntu" ; new-tab"#
        );
        // A profile that isn't there any more opens the default one, with no command.
        let gone = [tab("Renamed", None)];
        assert_eq!(terminal_args(&gone, &profiles, &[Some("npm run dev".into())]).unwrap(), "-w new new-tab");
        // A folder that isn't a plain local path is refused.
        assert!(terminal_args(&[tab("Ubuntu", Some(r"\\s\x"))], &profiles, &[None]).is_err());
    }

    #[test]
    fn profile_names_with_quotes_or_semicolons_are_refused() {
        for name in ["a\"b", "a;b", "a\\", "", "a\nb"] {
            let profiles = [profile(name, None)];
            assert!(terminal_args(&[tab(name, None)], &profiles, &[None]).is_err(), "{name:?}");
            let item = Item::Terminal { tabs: vec![tab(name, None)] };
            assert!(check_item(&item, "", Some(WT)).is_err(), "{name:?}");
        }
    }

    #[test]
    fn eight_tabs_at_most() {
        let eight: Vec<Tab> = (0..8).map(|_| Tab::default()).collect();
        assert!(terminal_args(&eight, &[], &[]).is_ok());
        let nine: Vec<Tab> = (0..9).map(|_| Tab::default()).collect();
        assert_eq!(terminal_args(&nine, &[], &[]), Err("Up to 8 tabs"));
        assert_eq!(check_item(&Item::Terminal { tabs: nine }, "", Some(WT)), Err("Up to 8 tabs"));
        assert!(check_item(&Item::Terminal { tabs: eight.clone() }, "", None).is_err());
        assert!(check_item(&Item::Terminal { tabs: eight }, "", Some(WT)).is_ok());
        assert!(check_item(&Item::Terminal { tabs: vec![] }, "", Some(WT)).is_err());
    }

    #[test]
    fn powershell_commands_are_encoded() {
        // The usual test vector: "Hello" is "SABlAGwAbABvAA==".
        assert_eq!(encoded_command("Hello"), "SABlAGwAbABvAA==");
        let nasty = "Write-Output \"a;b\" & $x = `n; echo 'é€😀'";
        let text = encoded_command(nasty);
        assert!(text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')));
        let bytes = unbase64(&text).unwrap();
        let units: Vec<u16> = bytes.chunks(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
        assert_eq!(String::from_utf16(&units).unwrap(), nasty);
        for len in 0..9 {
            let data: Vec<u8> = (0..len as u8).map(|b| b.wrapping_mul(37)).collect();
            assert_eq!(unbase64(&base64(&data)).unwrap(), data);
        }
        assert!(unbase64("abc").is_none() && unbase64("a=bc").is_none() && unbase64("ab!d").is_none());
        let profiles = [profile("PowerShell", Some("pwsh.exe"))];
        let args = terminal_args(&[tab("PowerShell", None)], &profiles, &[Some(nasty.into())]).unwrap();
        assert_eq!(args, format!("-w new new-tab -p \"PowerShell\" pwsh.exe -NoExit -EncodedCommand {text}"));
    }

    #[test]
    fn cmd_commands_escape_semicolons_and_refuse_quotes() {
        assert_eq!(cmd_escape("echo a;b").unwrap(), r"echo a\;b");
        assert_eq!(cmd_escape("npm run dev").unwrap(), "npm run dev");
        assert_eq!(cmd_escape("echo \"a\""), Err(CMD_QUOTES));
        let profiles = [profile("Command Prompt", Some(r"%SystemRoot%\System32\cmd.exe"))];
        let args = terminal_args(&[tab("Command Prompt", None)], &profiles, &[Some("echo a;b".into())]).unwrap();
        assert_eq!(args, r#"-w new new-tab -p "Command Prompt" cmd.exe /k echo a\;b"#);
        assert!(terminal_args(&[tab("Command Prompt", None)], &profiles, &[Some("echo \"a\"".into())]).is_err());
    }

    #[test]
    fn commands_refuse_line_breaks_and_hidden_characters() {
        for bad in ["a\nb", "a\r\nb", "a\tb", "a\u{202e}b", "a\u{200b}b", "a\u{0}b", "a\n"] {
            assert_eq!(command_ok(bad), Err(BAD_COMMAND), "{bad:?}");
        }
        assert_eq!(command_ok("  npm run dev  ").unwrap().as_deref(), Some("npm run dev"));
        assert_eq!(command_ok("   ").unwrap(), None);
        assert!(command_ok(&"x".repeat(MAX_COMMAND)).is_ok());
        assert!(command_ok(&"x".repeat(MAX_COMMAND + 1)).is_err());
        let item = Item::Terminal { tabs: vec![Tab { command: Some("a\nb".into()), ..Tab::default() }] };
        assert_eq!(check_item(&item, "", Some(WT)), Err(BAD_COMMAND));
    }

    #[test]
    fn other_shells_get_no_command() {
        let profiles = [
            profile("Ubuntu", Some("wsl.exe -d Ubuntu")),
            profile("Git Bash", Some(r"C:\Program Files\Git\bin\bash.exe")),
        ];
        let tabs = [tab("Ubuntu", None), tab("Git Bash", None)];
        let commands = [Some("rm -rf x".to_string()), Some("ls".to_string())];
        assert_eq!(
            terminal_args(&tabs, &profiles, &commands).unwrap(),
            r#"-w new new-tab -p "Ubuntu" ; new-tab -p "Git Bash""#
        );
        // A tab with no profile is the default one, whose shell isn't known.
        assert_eq!(terminal_args(&[Tab::default()], &profiles, &[Some("ls".into())]).unwrap(), "-w new new-tab");
    }
}
