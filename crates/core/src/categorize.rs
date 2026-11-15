use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Browser,
    Game,
    GameLauncher,
    Communication,
    Cloud,
    System,
    Download,
    #[default]
    Other,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Browser => "browser",
            Category::Game => "game",
            Category::GameLauncher => "game_launcher",
            Category::Communication => "communication",
            Category::Cloud => "cloud",
            Category::System => "system",
            Category::Download => "download",
            Category::Other => "other",
        }
    }
}

/// Conservative keyword classification. Anything uncertain stays `Other` so the
/// UI never claims a category the data does not support.
pub fn classify(exe_name: &str, publisher: Option<&str>, is_system: bool) -> Category {
    if is_system {
        return Category::System;
    }
    let exe = exe_name.to_ascii_lowercase();
    let name = exe.trim_end_matches(".exe");

    const HINTS: &[(Category, &[&str])] = &[
        (
            Category::Browser,
            &["chrome", "msedge", "firefox", "brave", "opera", "vivaldi", "iexplore"],
        ),
        (
            Category::GameLauncher,
            &[
                "steam", "epicgameslauncher", "battle.net", "origin", "eadesktop",
                "ubisoftconnect", "goggalaxy", "riotclientservices", "galaxyclient",
            ],
        ),
        (
            Category::Communication,
            &[
                "discord", "teams", "slack", "zoom", "skype", "telegram", "whatsapp",
                "signal", "element",
            ],
        ),
        (
            Category::Cloud,
            &["onedrive", "dropbox", "googledrive", "megasync", "nextcloud", "icloud"],
        ),
        (
            Category::Download,
            &[
                "idman", "thunder", "qbittorrent", "transmission", "utorrent",
                "deluge", "aria2", "freegate",
            ],
        ),
    ];

    for (category, keywords) in HINTS {
        if keywords.iter().any(|k| name.contains(k)) {
            return *category;
        }
    }

    if let Some(p) = publisher {
        let p = p.to_ascii_lowercase();
        if p.contains("electronic arts") || p.contains("ubisoft") || p.contains("riot games") {
            return Category::Game;
        }
    }

    // Heuristic for game executables: engine markers in the name.
    if name.contains("unity") || name.ends_with("-win64-shipping") || name.ends_with("-win32-shipping")
    {
        return Category::Game;
    }

    Category::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_browsers() {
        assert_eq!(classify("chrome.exe", None, false), Category::Browser);
        assert_eq!(classify("msedge.exe", None, false), Category::Browser);
    }

    #[test]
    fn classifies_launchers_before_games() {
        assert_eq!(
            classify("steam.exe", None, false),
            Category::GameLauncher
        );
    }

    #[test]
    fn system_flag_wins() {
        assert_eq!(classify("chrome.exe", None, true), Category::System);
    }

    #[test]
    fn unknown_stays_other() {
        assert_eq!(classify("weirdthing.exe", None, false), Category::Other);
    }
}
