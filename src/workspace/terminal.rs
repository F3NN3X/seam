//! Windows Terminal's profiles, read from its settings file (D739), and which shell
//! a profile runs, which decides whether a startup command can go in its tab (D735).
//! Pure: the file is read by the caller, off the main thread.

use serde_json::Value;

/// A profile of Windows Terminal that a tab may use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub commandline: Option<String>,
    /// The generator of a profile Windows Terminal made itself, such as `Windows.Terminal.PowershellCore`.
    pub source: Option<String>,
    pub guid: Option<String>,
}

/// The shell of a profile, as far as a startup command goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    /// `pwsh.exe` or `powershell.exe`, never a path from the file.
    PowerShell(&'static str),
    Cmd,
    /// WSL, Git Bash, SSH, Azure Cloud Shell, or anything Seam can't tell.
    Other,
}

/// Windows Terminal's own ids for the two profiles it makes for the Windows shells.
const WINDOWS_POWERSHELL: &str = "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}";
const COMMAND_PROMPT: &str = "{0caa0dad-35be-5f56-a8ff-afceeeaa6101}";
const POWERSHELL_SOURCE: &str = "Windows.Terminal.PowershellCore";

/// The first program of a command line, as a lower-case file stem.
fn program_of(commandline: &str) -> String {
    let line = commandline.trim();
    let first = match line.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or_default(),
        None => line.split_whitespace().next().unwrap_or_default(),
    };
    let name = first.rsplit(['\\', '/']).next().unwrap_or_default().to_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

/// Which shell a profile runs: by its command line, or for one Windows Terminal made, by its
/// id or source.
pub fn shell_of(profile: &Profile) -> Shell {
    let guid = profile.guid.as_deref().map(str::to_lowercase);
    let program = match &profile.commandline {
        Some(line) => program_of(line),
        None if profile.source.as_deref() == Some(POWERSHELL_SOURCE) => "pwsh".to_string(),
        None if guid.as_deref() == Some(WINDOWS_POWERSHELL) => "powershell".to_string(),
        None if guid.as_deref() == Some(COMMAND_PROMPT) => "cmd".to_string(),
        None => String::new(),
    };
    match program.as_str() {
        "pwsh" => Shell::PowerShell("pwsh.exe"),
        "powershell" => Shell::PowerShell("powershell.exe"),
        "cmd" => Shell::Cmd,
        _ => Shell::Other,
    }
}

/// The settings file without its comments and trailing commas, which `serde_json` can't read.
/// A `//` or `/*` inside a string stays.
pub fn strip_jsonc(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let (mut i, mut in_string) = (0, false);
    while i < chars.len() {
        let c = chars[i];
        if in_string {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            out.push(c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
            continue;
        } else if c == ',' && matches!(chars[i + 1..].iter().find(|c| !c.is_whitespace()), Some('}' | ']')) {
            // A comma before a closing bracket, with only blanks between. Comments are gone only
            // as the loop reaches them, so a comment in between hides it until the next pass.
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// Settings text as JSON, comments and trailing commas stripped, twice so a comma that had a
/// comment before its bracket goes too.
fn parse(text: &str) -> Option<Value> {
    let once = strip_jsonc(text.trim_start_matches('\u{feff}'));
    serde_json::from_str(&strip_jsonc(&once)).ok()
}

fn list_of(settings: &Value) -> Vec<&Value> {
    let profiles = settings.get("profiles");
    let list = match profiles {
        Some(Value::Array(list)) => Some(list),
        Some(p) => p.get("list").and_then(Value::as_array),
        None => None,
    };
    list.into_iter().flatten().collect()
}

/// The profiles to offer, in the file's order: not hidden and not elevated, which would
/// make Windows Terminal ask for permission and which Seam couldn't move.
pub fn profiles(settings_json: &str) -> Vec<Profile> {
    let Some(settings) = parse(settings_json) else { return Vec::new() };
    let inherited = settings.pointer("/profiles/defaults/elevate").and_then(Value::as_bool).unwrap_or(false);
    let text = |p: &Value, key: &str| p.get(key).and_then(Value::as_str).map(str::to_string);
    list_of(&settings)
        .into_iter()
        .filter(|p| !p.get("hidden").and_then(Value::as_bool).unwrap_or(false))
        .filter(|p| !p.get("elevate").and_then(Value::as_bool).unwrap_or(inherited))
        .filter_map(|p| {
            let name = text(p, "name").filter(|n| !n.is_empty())?;
            Some(Profile {
                name,
                commandline: text(p, "commandline"),
                source: text(p, "source"),
                guid: text(p, "guid"),
            })
        })
        .collect()
}

/// The name of the profile Windows Terminal opens by default, if it's among `offered`.
pub fn default_profile(settings_json: &str, offered: &[Profile]) -> Option<String> {
    let settings = parse(settings_json)?;
    let want = settings.get("defaultProfile").and_then(Value::as_str)?.to_lowercase();
    let found = offered.iter().find(|p| p.guid.as_deref().map(str::to_lowercase).as_deref() == Some(&want));
    found.or_else(|| offered.iter().find(|p| p.name.to_lowercase() == want)).map(|p| p.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonc_comments_are_stripped() {
        let text = "{ // line\n \"a\": \"http://x\", /* block\n comment */ \"b\": \"/* kept */\", \"c\": [1, 2,], }";
        let value: Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
        assert_eq!(value["a"], "http://x");
        assert_eq!(value["b"], "/* kept */");
        assert_eq!(value["c"], serde_json::json!([1, 2]));
        // An escaped quote doesn't end the string.
        let escaped = r#"{ "a": "say \" // not a comment" }"#;
        assert_eq!(serde_json::from_str::<Value>(&strip_jsonc(escaped)).unwrap()["a"], "say \" // not a comment");
    }

    #[test]
    fn profiles_are_read_from_list_or_array() {
        let object = r#"{ // settings
            "defaultProfile": "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}",
            "profiles": { "defaults": {}, "list": [
                { "guid": "{61c54bbd-c2c6-5271-96e7-009a87ff44bf}", "name": "Windows PowerShell" },
                { "name": "Ubuntu", "commandline": "wsl.exe -d Ubuntu" },
            ] } }"#;
        let list = profiles(object);
        assert_eq!(list.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Windows PowerShell", "Ubuntu"]);
        assert_eq!(default_profile(object, &list).as_deref(), Some("Windows PowerShell"));
        let array = r#"{ "profiles": [ { "name": "One" }, { "name": "Two" } ] }"#;
        assert_eq!(profiles(array).len(), 2);
        assert!(profiles("not json").is_empty());
        assert_eq!(default_profile(array, &profiles(array)), None);
    }

    #[test]
    fn hidden_and_elevated_profiles_are_left_out() {
        let text = r#"{ "profiles": { "defaults": { "elevate": false }, "list": [
            { "name": "Shown" },
            { "name": "Hidden", "hidden": true },
            { "name": "Admin", "elevate": true } ] } }"#;
        assert_eq!(profiles(text).iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Shown"]);
        let all = r#"{ "profiles": { "defaults": { "elevate": true }, "list": [ { "name": "A" }, { "name": "B", "elevate": false } ] } }"#;
        assert_eq!(profiles(all).iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["B"]);
    }

    #[test]
    fn shells_are_told_by_command_line_id_or_source() {
        let by = |line: Option<&str>, guid: Option<&str>, source: Option<&str>| {
            shell_of(&Profile {
                name: "x".into(),
                commandline: line.map(Into::into),
                guid: guid.map(Into::into),
                source: source.map(Into::into),
            })
        };
        assert_eq!(by(Some(r"%SystemRoot%\System32\cmd.exe"), None, None), Shell::Cmd);
        assert_eq!(
            by(Some(r#""C:\Program Files\PowerShell\7\pwsh.exe" -NoLogo"#), None, None),
            Shell::PowerShell("pwsh.exe")
        );
        assert_eq!(by(None, Some(WINDOWS_POWERSHELL), None), Shell::PowerShell("powershell.exe"));
        assert_eq!(by(None, Some(COMMAND_PROMPT), None), Shell::Cmd);
        assert_eq!(by(None, None, Some(POWERSHELL_SOURCE)), Shell::PowerShell("pwsh.exe"));
        assert_eq!(by(Some("wsl.exe -d Ubuntu"), None, None), Shell::Other);
        assert_eq!(by(Some(r"C:\Program Files\Git\bin\bash.exe"), None, None), Shell::Other);
        assert_eq!(by(None, None, Some("Windows.Terminal.Azure")), Shell::Other);
    }
}
