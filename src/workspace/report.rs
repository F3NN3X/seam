//! What a switch tells about the apps it couldn't start or move: which, and why, in plain words (D711).

use super::plan::SCREEN_GONE;

/// What `skipped` says about an app whose window didn't show in time.
pub const TOOK_TOO_LONG: &str = "Took too long to open";
/// What `skipped` says about an app whose program runs with no window.
pub const RUNS_IN_BACKGROUND: &str = "Running in background";
/// What `skipped` says about an app a cancelled switch never started. Not a problem: the user asked.
pub const CANCELLED: &str = "cancelled";

/// What `skipped` says about a Terminal item whose startup command didn't come from this PC (D736).
pub const FOREIGN_COMMAND: &str = "startup command not from this PC";

/// A balloon's text holds 255 characters, and its title 63.
const BODY_MAX: usize = 255;
const TITLE_MAX: usize = 63;
/// The longest an app's name is in a balloon before the words are cut: tried down from here.
const NAME_MAX: usize = 40;

/// The words after the app's name for a reason that is a problem, or `None` for one that is on
/// purpose (sticky, pinned, Steam games, Other windows with nothing to act on, a cancel).
pub fn words(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "not installed" => "isn't installed, or Seam can't find it",
        "refused" => "is on a network drive or a path Seam won't start",
        "open with refused" => "has an Open with that Seam won't use",
        "item refused" => "has something to open that Seam won't use",
        FOREIGN_COMMAND => "opened without its startup command, which didn't come from this PC",
        TOOK_TOO_LONG => "didn't show a window within 15 seconds",
        RUNS_IN_BACKGROUND => "is running with no window, so there was nothing to move",
        "elevated" => "runs as administrator, so Seam can't move it",
        "timeout" => "isn't responding",
        "not full screen" => "didn't go full screen",
        "didn't move" => "moved back by itself",
        SCREEN_GONE => "stayed where it is, because its screen isn't connected",
        _ => return None,
    })
}

/// The apps that had a problem, each once, with the words for the first reason it was skipped for.
pub fn problems<'a>(skipped: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<(String, &'static str)> {
    let mut found: Vec<(String, &'static str)> = Vec::new();
    for (app, reason) in skipped {
        if let Some(said) = words(reason)
            && !found.iter().any(|(name, _)| name == app)
        {
            found.push((app.to_string(), said));
        }
    }
    found
}

/// What a message calls an app: its exe's name without the extension, or failing that its window title.
pub fn app_name(exe: &str, title_hint: &str) -> String {
    let stem = std::path::Path::new(exe).file_stem().map(|s| s.to_string_lossy().into_owned());
    stem.filter(|s| !s.is_empty()).unwrap_or_else(|| title_hint.to_string())
}

/// How many apps a cancel left unstarted, from the rows and reasons skipped.
pub fn left_unstarted<'a>(skipped: impl IntoIterator<Item = (Option<usize>, &'a str)>) -> usize {
    let mut rows: Vec<Option<usize>> =
        skipped.into_iter().filter(|(_, reason)| *reason == CANCELLED).map(|(row, _)| row).collect();
    rows.sort_unstable();
    rows.dedup();
    rows.len()
}

/// "1 app" or "3 apps".
pub fn apps(n: usize) -> String {
    if n == 1 { "1 app".to_string() } else { format!("{n} apps") }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// What the shell counts: UTF-16 units.
fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The sentences of a notice's body: what a cancel left unstarted, then the first two problems and
/// how many more. App names are cut to `max_name` characters.
fn sentences(problems: &[(String, &str)], left: Option<usize>, max_name: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    match left {
        Some(0) | None => {}
        Some(1) => parts.push("1 app wasn't started".to_string()),
        Some(n) => parts.push(format!("{n} apps weren't started")),
    }
    let mut said: Vec<String> =
        problems.iter().take(2).map(|(app, words)| format!("{} {words}", clip(app, max_name))).collect();
    if problems.len() > 2
        && let Some(last) = said.last_mut()
    {
        last.push_str(&format!(" and {} more", problems.len() - 2));
    }
    parts.extend(said);
    parts.join(". ")
}

/// The title of a notice after a switch: it says which workspace, and how it went.
fn title(name: &str, problems: &[(String, &str)], cancelled: bool, max_name: usize) -> String {
    let name = clip(name, max_name);
    if cancelled {
        format!("Stopped switching to {name}")
    } else {
        format!("Switched to {name}. {} had a problem", apps(problems.len()))
    }
}

/// The notice after a switch, as the Seam window shows it: the title and the body, or `None` when
/// there is nothing to say. `cancelled_left` is `Some` for a cancel, with how many apps it left unstarted.
pub fn panel_summary(
    name: &str,
    problems: &[(String, &str)],
    cancelled_left: Option<usize>,
) -> Option<(String, String)> {
    if problems.is_empty() && cancelled_left.is_none() {
        return None;
    }
    Some((title(name, problems, cancelled_left.is_some(), usize::MAX), sentences(problems, cancelled_left, usize::MAX)))
}

/// The notice while a switch starts apps: the title and the body. The tray's balloon can be
/// clicked, the Seam window's has a button, so only the balloon says "click here".
pub fn starting(name: &str, count: usize, balloon: bool) -> (String, String) {
    let stop = if balloon { "Press Esc or click here to stop" } else { "Press Esc to stop" };
    (format!("Switching to {}", clip(name, 40)), format!("Starting {}. {stop}", apps(count)))
}

/// The tray's balloon after a switch: `panel_summary` with "Click to see them" when there are
/// problems, cut to fit a balloon, app names first.
pub fn summary(name: &str, problems: &[(String, &str)], cancelled_left: Option<usize>) -> Option<(String, String)> {
    if problems.is_empty() && cancelled_left.is_none() {
        return None;
    }
    let click = if problems.is_empty() { "" } else { ". Click to see them" };
    let mut title_name = NAME_MAX;
    let mut head = title(name, problems, cancelled_left.is_some(), title_name);
    while units(&head) > TITLE_MAX && title_name > 1 {
        title_name -= 1;
        head = title(name, problems, cancelled_left.is_some(), title_name);
    }
    let mut body = String::new();
    for max_name in (1..=NAME_MAX).rev() {
        body = format!("{}{click}", sentences(problems, cancelled_left, max_name));
        if units(&body) <= BODY_MAX {
            break;
        }
    }
    // Names can't be cut further, so the line is.
    if units(&body) > BODY_MAX {
        body = clip(&body, BODY_MAX);
    }
    Some((head, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn problem(app: &str, reason: &str) -> (String, &'static str) {
        (app.to_string(), words(reason).unwrap())
    }

    #[test]
    fn every_reason_has_words_or_is_on_purpose() {
        // Every reason a switch can skip an app for. A new one fails here until it is sorted.
        let problems = [
            "not installed",
            "refused",
            "open with refused",
            TOOK_TOO_LONG,
            RUNS_IN_BACKGROUND,
            "elevated",
            "timeout",
            "not full screen",
            "didn't move",
            SCREEN_GONE,
        ];
        let on_purpose = ["sticky", "pinned", "steam game", "nothing matched", CANCELLED];
        for reason in problems {
            assert!(words(reason).is_some(), "{reason}");
        }
        for reason in on_purpose {
            assert_eq!(words(reason), None, "{reason}");
        }
    }

    #[test]
    fn an_app_is_named_once_and_only_for_a_problem() {
        let skipped = [("Slack", "sticky"), ("Slack", "timeout"), ("Slack", "elevated"), ("Figma", "pinned")];
        let found = problems(skipped);
        assert_eq!(found, vec![("Slack".to_string(), "isn't responding")]);
    }

    #[test]
    fn summary_names_two_then_counts_the_rest() {
        let three = [problem("Slack", "not installed"), problem("Figma", TOOK_TOO_LONG), problem("Zoom", "timeout")];
        let (title, body) = summary("Work", &three, None).unwrap();
        assert_eq!(title, "Switched to Work. 3 apps had a problem");
        assert_eq!(
            body,
            "Slack isn't installed, or Seam can't find it. Figma didn't show a window within 15 seconds \
             and 1 more. Click to see them"
        );
        let (_, body) = panel_summary("Work", &three, None).unwrap();
        assert!(body.ends_with("and 1 more"));
        let two = &three[..2];
        let (_, body) = summary("Work", two, None).unwrap();
        assert!(body.ends_with("within 15 seconds. Click to see them"));
        assert!(!body.contains("more"));
    }

    #[test]
    fn a_cancel_counts_rows_not_entries() {
        let skipped = [(Some(3), CANCELLED), (Some(3), CANCELLED), (Some(4), CANCELLED), (Some(1), "timeout")];
        assert_eq!(left_unstarted(skipped), 2);
        assert_eq!(left_unstarted([]), 0);
        assert_eq!(app_name("C:\\Apps\\slack.exe", "Slack"), "slack");
        assert_eq!(app_name("", "Slack"), "Slack");
    }

    #[test]
    fn singular_and_plural() {
        let one = [problem("Slack", "elevated")];
        assert_eq!(summary("Work", &one, None).unwrap().0, "Switched to Work. 1 app had a problem");
        assert_eq!(panel_summary("Work", &[], Some(1)).unwrap().1, "1 app wasn't started");
        assert_eq!(panel_summary("Work", &[], Some(2)).unwrap().1, "2 apps weren't started");
        // Nothing left to start: the title says it all.
        assert_eq!(panel_summary("Work", &[], Some(0)).unwrap(), ("Stopped switching to Work".into(), String::new()));
        assert_eq!(summary("Work", &[], None), None);
        let (title, body) = summary("Work", &one, Some(2)).unwrap();
        assert_eq!(title, "Stopped switching to Work");
        assert_eq!(
            body,
            "2 apps weren't started. Slack runs as administrator, so Seam can't move it. Click to see them"
        );
    }

    #[test]
    fn the_starting_notice_says_how_to_stop() {
        let (title, body) = starting("Work", 4, true);
        assert_eq!(
            (title.as_str(), body.as_str()),
            ("Switching to Work", "Starting 4 apps. Press Esc or click here to stop")
        );
        assert_eq!(starting("Work", 1, false).1, "Starting 1 app. Press Esc to stop");
    }

    #[test]
    fn summary_fits_a_balloon() {
        let long = "A".repeat(300);
        let many: Vec<(String, &str)> = (0..5).map(|_| problem(&long, "not installed")).collect();
        let (title, body) = summary(&long, &many, Some(3)).unwrap();
        assert!(units(&title) <= TITLE_MAX, "{}", units(&title));
        assert!(units(&body) <= BODY_MAX, "{}", units(&body));
        assert!(body.ends_with("Click to see them"));
        // Names go first: the words are all still there.
        assert!(body.contains("isn't installed, or Seam can't find it"));
    }
}
