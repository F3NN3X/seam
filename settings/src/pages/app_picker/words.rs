//! What the workspace table calls an item row, and the line under its name (D731, D738).

use seam::workspace::{Item, Rule};

/// The folder's last part, or the whole path for a drive's root.
fn last_part(folder: &str) -> &str {
    let trimmed = folder.trim_end_matches('\\');
    match trimmed.rsplit('\\').next() {
        Some(part) if !part.is_empty() && !part.ends_with(':') => part,
        _ => folder,
    }
}

fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {many}") }
}

/// The host of an address, for the line under a Web links row.
fn host(address: &str) -> &str {
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// The row's name, `app` being the app's own name: "Folder: seam", "Web links: 3 pages",
/// "Terminal: 2 tabs", "Code: seam". `None` for a row with no item.
pub fn title(rule: &Rule, app: &str) -> Option<String> {
    Some(match rule.item.as_ref()? {
        Item::Folder { folder } => format!("Folder: {}", last_part(folder)),
        Item::Web { urls } => format!("Web links: {}", count(urls.len(), "page", "pages")),
        Item::Terminal { tabs } => format!("Terminal: {}", count(tabs.len(), "tab", "tabs")),
        Item::Editor { folder } => format!("{app}: {}", last_part(folder)),
    })
}

/// The line under the name: the program and what it opens, with the first startup command
/// named (D738). `exe` is the row's program as the table shows it. `None` for a row with no item.
pub fn meta(rule: &Rule, exe: &str) -> Option<String> {
    Some(match rule.item.as_ref()? {
        Item::Folder { folder } | Item::Editor { folder } => format!("{exe}, {folder}"),
        Item::Web { urls } => match urls.as_slice() {
            [] => exe.to_string(),
            [only] => format!("{exe}, {}", host(only)),
            [first, rest @ ..] => format!("{exe}, {} and {} more", host(first), rest.len()),
        },
        Item::Terminal { tabs } => {
            let line = match tabs.iter().find_map(|t| t.command.as_deref()) {
                Some(command) => format!("Windows Terminal, runs {}", command.lines().next().unwrap_or_default()),
                None => format!("Windows Terminal, {}", count(tabs.len(), "tab", "tabs")),
            };
            // A command that didn't open here never runs, and the row says so (D736).
            match tabs.iter().any(|t| t.unreadable) {
                true => format!("{line}, startup command not from this PC"),
                false => line,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use seam::workspace::Tab;

    fn row(item: Item) -> Rule {
        Rule { item: Some(item), ..Rule::default() }
    }

    #[test]
    fn rows_are_named_by_what_they_open() {
        let folder = row(Item::Folder { folder: r"C:\Projects\seam".into() });
        assert_eq!(title(&folder, "Explorer").as_deref(), Some("Folder: seam"));
        assert_eq!(meta(&folder, "explorer.exe").as_deref(), Some(r"explorer.exe, C:\Projects\seam"));
        let root = row(Item::Folder { folder: r"D:\".into() });
        assert_eq!(title(&root, "Explorer").as_deref(), Some(r"Folder: D:\"));
        let editor = row(Item::Editor { folder: r"C:\Projects\seam\".into() });
        assert_eq!(title(&editor, "Code").as_deref(), Some("Code: seam"));
        assert_eq!(meta(&editor, "Code.exe").as_deref(), Some(r"Code.exe, C:\Projects\seam\"));
        assert_eq!(title(&Rule::default(), "x"), None);
    }

    #[test]
    fn web_rows_name_the_first_page_and_count_the_rest() {
        let urls = |n: usize| (0..n).map(|i| format!("https://github.com/x/{i}?y=1")).collect();
        assert_eq!(title(&row(Item::Web { urls: urls(3) }), "Edge").as_deref(), Some("Web links: 3 pages"));
        assert_eq!(title(&row(Item::Web { urls: urls(1) }), "Edge").as_deref(), Some("Web links: 1 page"));
        assert_eq!(
            meta(&row(Item::Web { urls: urls(3) }), "msedge.exe").as_deref(),
            Some("msedge.exe, github.com and 2 more")
        );
        assert_eq!(meta(&row(Item::Web { urls: urls(1) }), "msedge.exe").as_deref(), Some("msedge.exe, github.com"));
    }

    #[test]
    fn terminal_rows_name_the_first_command() {
        let tab = |command: Option<&str>| Tab { command: command.map(Into::into), ..Tab::default() };
        let two = row(Item::Terminal { tabs: vec![tab(None), tab(None)] });
        assert_eq!(title(&two, "Windows Terminal").as_deref(), Some("Terminal: 2 tabs"));
        assert_eq!(meta(&two, "wt.exe").as_deref(), Some("Windows Terminal, 2 tabs"));
        let runs = row(Item::Terminal { tabs: vec![tab(None), tab(Some("npm run dev"))] });
        assert_eq!(meta(&runs, "wt.exe").as_deref(), Some("Windows Terminal, runs npm run dev"));
        let foreign = row(Item::Terminal { tabs: vec![Tab { unreadable: true, ..Tab::default() }] });
        assert_eq!(
            meta(&foreign, "wt.exe").as_deref(),
            Some("Windows Terminal, 1 tab, startup command not from this PC")
        );
        assert_eq!(title(&row(Item::Terminal { tabs: vec![tab(None)] }), "").as_deref(), Some("Terminal: 1 tab"));
    }
}
