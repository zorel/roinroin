//! Linux desktop notifications (freedesktop.org / D-Bus).

use notify_rust::{Notification, Timeout, Urgency};

pub fn send(summary: &str, body: &str, critical: bool) -> Result<(), String> {
    let summary = one_line(summary, 120);
    let body = one_line(body, 360);
    let icon = icon_path();
    let mut notification = Notification::new();
    notification
        .appname("roinroin")
        .summary(&summary)
        .body(&body)
        .icon(&icon)
        .timeout(Timeout::Milliseconds(if critical { 10_000 } else { 6_000 }))
        .urgency(if critical {
            Urgency::Critical
        } else {
            Urgency::Normal
        });
    #[cfg(target_os = "linux")]
    {
        notification.hint(notify_rust::Hint::Category("im".to_owned()));
    }
    notification
        .show()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn icon_path() -> String {
    let mut candidates = Vec::new();
    if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
        candidates
            .push(std::path::PathBuf::from(data).join("icons/hicolor/scalable/apps/roinroin.svg"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(
            std::path::PathBuf::from(home)
                .join(".local/share/icons/hicolor/scalable/apps/roinroin.svg"),
        );
    }
    candidates.push(std::path::PathBuf::from(
        "/usr/share/icons/hicolor/scalable/apps/roinroin.svg",
    ));
    for candidate in candidates {
        if candidate.is_file() {
            return candidate.display().to_string();
        }
    }
    "roinroin".to_owned()
}

fn one_line(value: &str, max_chars: usize) -> String {
    let mut result = String::new();
    let mut previous_space = false;
    for character in value.chars() {
        let character = if character.is_control() || character.is_whitespace() {
            if previous_space {
                continue;
            }
            previous_space = true;
            ' '
        } else {
            previous_space = false;
            character
        };
        if result.chars().count() >= max_chars {
            result.push('…');
            break;
        }
        result.push(character);
    }
    result.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_whitespace() {
        assert_eq!(one_line("hello\n\n  world", 100), "hello world");
    }
}
