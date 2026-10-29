//! Persistent configuration and session state.
//!
//! The client keeps a small TOML configuration under the XDG config directory
//! and a JSON state file (last-read posts, known tribunes, window size) under
//! the XDG data directory. Both files are deliberately human editable.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "roinroin";
pub const DEFAULT_SERVER: &str = "http://localhost:16666";
pub const DEFAULT_TOTOZ: &str = "https://nsfw.totoz.eu";
pub const MAX_MESSAGE_LENGTH: usize = 65_536;
pub const HISTORY_LIMIT: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BlockedMode {
    Hide,
    #[default]
    Gray,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    French,
    English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoardAuth {
    #[default]
    None,
    Oauth2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Board {
    pub name: String,
    pub backend_url: String,
    pub post_url: String,
    #[serde(default = "default_post_field")]
    pub post_field: String,
    #[serde(default)]
    pub info_field: String,
    #[serde(default)]
    pub auth: BoardAuth,
}

pub fn default_post_field() -> String {
    "message".to_owned()
}

impl Board {
    pub fn new(
        name: impl Into<String>,
        backend_url: impl Into<String>,
        post_url: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            backend_url: backend_url.into(),
            post_url: post_url.into(),
            post_field: default_post_field(),
            info_field: String::new(),
            auth: BoardAuth::None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("le nom du board est vide".into());
        }
        if name.len() > 64 {
            return Err("le nom du board est trop long".into());
        }
        if name
            .chars()
            .any(|c| c.is_whitespace() || c == ',' || c == '|' || c.is_control())
        {
            return Err("le nom du board contient un caractère interdit".into());
        }
        validate_http_url(&self.backend_url)?;
        validate_http_url(&self.post_url)?;
        if !valid_field_name(&self.post_field) {
            return Err("le champ de message est invalide".into());
        }
        if !self.info_field.is_empty() && !valid_field_name(&self.info_field) {
            return Err("le champ info est invalide".into());
        }
        Ok(())
    }

    pub fn normalized(&self) -> Self {
        Self {
            name: self.name.trim().to_owned(),
            backend_url: normalize_url(&self.backend_url)
                .unwrap_or_else(|| self.backend_url.clone()),
            post_url: normalize_url(&self.post_url).unwrap_or_else(|| self.post_url.clone()),
            post_field: {
                let field = self.post_field.trim();
                if field.is_empty() {
                    default_post_field()
                } else {
                    field.to_owned()
                }
            },
            info_field: self.info_field.trim().to_owned(),
            auth: self.auth,
        }
    }

    pub fn requires_oauth(&self) -> bool {
        self.auth == BoardAuth::Oauth2
    }
}

fn valid_field_name(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty() && value.len() <= 64 && !value.chars().any(char::is_control)
}

pub fn normalize_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains("://")
        && !(trimmed.starts_with("http://") || trimmed.starts_with("https://"))
    {
        return None;
    }
    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_owned()
    } else {
        format!("http://{trimmed}")
    };
    let mut url = url::Url::parse(&with_scheme).ok()?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return None;
    }
    // A URL without a path is normalized to a slash; trim it to keep
    // concatenation of API paths predictable.
    if url.path() == "/" {
        url.set_path("");
    }
    Some(url.to_string().trim_end_matches('/').to_owned())
}

fn validate_http_url(raw: &str) -> Result<(), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("URL vide".into());
    }
    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_owned()
    } else {
        format!("http://{trimmed}")
    };
    let url = url::Url::parse(&with_scheme).map_err(|_| "URL invalide".to_owned())?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("seuls http et https sont acceptés".into());
    }
    if url.host_str().is_none() {
        return Err("URL sans hôte".into());
    }
    Ok(())
}

/// The built-in board list, kept in sync with gb2c's default configuration and
/// the Android client. They can be enabled without any gb2c server.
pub fn predefined_boards() -> Vec<Board> {
    let mut boards = vec![
        Board {
            name: "dlfp".into(),
            backend_url: "https://linuxfr.org/board/index.tsv".into(),
            post_url: "https://linuxfr.org/api/v1/board".into(),
            post_field: "message".into(),
            info_field: String::new(),
            auth: BoardAuth::Oauth2,
        },
        Board::new(
            "euromussels",
            "https://faab.euromussels.eu/data/backend.tsv",
            "https://faab.euromussels.eu/add.php",
        ),
        Board::new(
            "gabuzomeu",
            "https://gb3.plop.cc/gb0/tsv",
            "https://gb3.plop.cc/gb0/post",
        ),
        Board::new(
            "moules",
            "https://moules.org/board/backend/tsv",
            "https://moules.org/board/add.php?backend=tsv",
        ),
        Board::new(
            "devnewton",
            "https://gb3.devnewton.fr/gb0/tsv",
            "https://gb3.devnewton.fr/gb0/post",
        ),
        Board::new(
            "ototu",
            "https://www.ototu.me/goboard/backend/tsv",
            "https://www.ototu.me/goboard/post",
        ),
        Board::new(
            "reveildutroll",
            "https://miaoli.im/%2B6VtcdSCI0Y/posts.tsv",
            "https://miaoli.im/%2B6VtcdSCI0Y",
        ),
        Board {
            name: "sveetch".into(),
            backend_url: "http://sveetch.net/tribune/remote/tsv/".into(),
            post_url: "http://sveetch.net/tribune/post/tsv/?last_id=1".into(),
            post_field: "content".into(),
            info_field: String::new(),
            auth: BoardAuth::None,
        },
    ];
    boards.sort_by(|a, b| a.name.cmp(&b.name));
    boards
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerConfig {
    pub name: String,
    pub url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl ServerConfig {
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct NotifyConfig {
    pub enabled: bool,
    pub all_posts: bool,
    pub bigorno: bool,
    pub replies: bool,
    pub only_unfocused: bool,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            all_posts: false,
            bigorno: true,
            replies: true,
            only_unfocused: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub nickname: String,
    pub servers: Vec<ServerConfig>,
    pub custom_boards: Vec<Board>,
    pub selected_boards: BTreeSet<String>,
    pub hidden_tribunes: BTreeSet<String>,
    pub totoz_server: String,
    pub blocked: BTreeSet<String>,
    pub blocked_mode: BlockedMode,
    pub plop_list: BTreeSet<String>,
    pub notifications: NotifyConfig,
    pub theme: ThemeMode,
    pub language: Language,
    pub linuxfr: Option<LinuxfrSession>,
    pub draft: String,
    pub last_posted: String,
    pub show_hidden: bool,
    pub history_limit: usize,
    pub hourly_bell: bool,
}

impl Default for Config {
    fn default() -> Self {
        let mut selected_boards = BTreeSet::new();
        selected_boards.insert("devnewton".to_owned());
        Self {
            nickname: random_nickname(),
            servers: vec![ServerConfig::new("local", DEFAULT_SERVER)],
            custom_boards: Vec::new(),
            selected_boards,
            hidden_tribunes: BTreeSet::new(),
            totoz_server: DEFAULT_TOTOZ.to_owned(),
            blocked: BTreeSet::new(),
            blocked_mode: BlockedMode::Gray,
            plop_list: BTreeSet::new(),
            notifications: NotifyConfig::default(),
            theme: ThemeMode::Dark,
            language: Language::French,
            linuxfr: None,
            draft: String::new(),
            last_posted: String::new(),
            show_hidden: false,
            history_limit: HISTORY_LIMIT,
            hourly_bell: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LinuxfrSession {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix time in seconds, 0 when unknown.
    pub expires_at: i64,
    pub login: String,
}

impl LinuxfrSession {
    pub fn is_expired(&self) -> bool {
        self.expires_at > 0 && chrono::Utc::now().timestamp() >= self.expires_at - 60
    }

    /// Accept either a raw OAuth token, the base64 JSON blob produced by
    /// gb2c's `/linuxfr.html` redirect, or the whole redirect URL.
    pub fn from_pasted(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }

        let fragment = trimmed.rsplit('#').next().unwrap_or(trimmed);
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(fragment) {
            return Self::from_json(&json);
        }
        if let Some(decoded) = base64_decode(fragment) {
            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&decoded) {
                if let Some(session) = Self::from_json(&json) {
                    return Some(session);
                }
            }
        }
        // A bare access token is still useful, even without a refresh token.
        Some(Self {
            access_token: trimmed.to_owned(),
            ..Self::default()
        })
    }

    fn from_json(value: &serde_json::Value) -> Option<Self> {
        let access = value.get("access_token")?.as_str()?.trim();
        if access.is_empty() {
            return None;
        }
        Some(Self {
            access_token: access.to_owned(),
            refresh_token: value
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned(),
            expires_at: value
                .get("expires_in")
                .and_then(|v| v.as_i64())
                .map(|seconds| chrono::Utc::now().timestamp() + seconds)
                .unwrap_or(0),
            login: value
                .get("login")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_owned(),
        })
    }
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    // Tiny dependency-free decoder for standard base64. Invalid characters are
    // rejected; this is only used for the gb2c LinuxFR redirect fragment.
    fn value(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let input = input.trim_end_matches('=');
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        let value = value(byte)?;
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

impl Config {
    pub fn all_boards(&self) -> Vec<Board> {
        let mut boards = predefined_boards();
        let names: BTreeSet<String> = boards.iter().map(|b| b.name.clone()).collect();
        for board in &self.custom_boards {
            if board.validate().is_ok() {
                let board = board.normalized();
                if !names.contains(&board.name) {
                    boards.push(board);
                }
            }
        }
        boards.sort_by(|a, b| a.name.cmp(&b.name));
        boards
    }

    pub fn board(&self, name: &str) -> Option<Board> {
        self.all_boards().into_iter().find(|b| b.name == name)
    }

    pub fn my_identities(&self) -> BTreeSet<String> {
        let mut identities = BTreeSet::new();
        if !self.nickname.trim().is_empty() {
            identities.insert(self.nickname.trim().to_owned());
        }
        if let Some(linuxfr) = &self.linuxfr {
            if !linuxfr.login.trim().is_empty() {
                identities.insert(linuxfr.login.trim().to_owned());
            }
        }
        identities
    }

    pub fn validate_for_save(&self) -> Result<(), String> {
        if self.nickname.trim().is_empty() {
            return Err("le pseudo est vide".into());
        }
        if self.nickname.len() > 128 || self.nickname.chars().any(char::is_control) {
            return Err("le pseudo est invalide".into());
        }
        for server in &self.servers {
            validate_http_url(&server.url)?;
        }
        for board in &self.custom_boards {
            board.validate()?;
        }
        if let Some(totoz) = normalize_url(&self.totoz_server) {
            if !totoz.starts_with("http") {
                return Err("serveur totoz invalide".into());
            }
        } else {
            return Err("serveur totoz invalide".into());
        }
        Ok(())
    }
}

/// Session state that is not really configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PersistedState {
    pub last_read: BTreeMap<String, i64>,
    pub known_tribunes: BTreeMap<String, Vec<String>>,
    pub active: Option<String>,
    pub window_width: f32,
    pub window_height: f32,
    pub sidebar_width: f32,
}

pub fn config_dir() -> PathBuf {
    directories::ProjectDirs::from("fr", "devnewton", APP_NAME)
        .map(|dirs| dirs.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn data_dir() -> PathBuf {
    directories::ProjectDirs::from("fr", "devnewton", APP_NAME)
        .map(|dirs| dirs.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn state_path() -> PathBuf {
    data_dir().join("state.json")
}

pub fn load_config() -> (Config, Option<String>) {
    let (mut config, warning) = load_config_file();
    if let Ok(url) = std::env::var("ROINROIN_GB2C") {
        if let Some(url) = normalize_url(&url) {
            config.servers = vec![ServerConfig::new("cli", url)];
        }
    }
    if let Ok(nickname) = std::env::var("ROINROIN_NICKNAME") {
        if !nickname.trim().is_empty() {
            config.nickname = nickname.trim().to_owned();
        }
    }
    if std::env::var("ROINROIN_NO_NOTIFICATIONS").is_ok() {
        config.notifications.enabled = false;
    }
    (config, warning)
}

fn load_config_file() -> (Config, Option<String>) {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(text) => match toml::from_str::<Config>(&text) {
            Ok(mut config) => {
                // Old or hand-edited files may have an empty list; keep a
                // useful default server rather than silently doing nothing.
                if config.servers.is_empty() {
                    config
                        .servers
                        .push(ServerConfig::new("local", DEFAULT_SERVER));
                }
                (config, None)
            }
            Err(error) => (
                Config::default(),
                Some(format!(
                    "Configuration illisible ({}), valeurs par défaut.",
                    error
                )),
            ),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let config = Config::default();
            let warning = save_config(&config)
                .err()
                .map(|error| format!("Impossible de créer la configuration : {error}"));
            (config, warning)
        }
        Err(error) => (
            Config::default(),
            Some(format!("Impossible de lire la configuration : {error}")),
        ),
    }
}

pub fn save_config(config: &Config) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = toml::to_string_pretty(config).map_err(|error| error.to_string())?;
    let header = "# roinroin — client de tribune natif pour Linux\n\
         # Les boards directs peuvent être activés dans selected_boards.\n\
         # Voir roinroin.example.toml pour un exemple commenté.\n\n";
    write_atomic(&path, format!("{header}{text}").as_bytes())
}

pub fn load_state() -> PersistedState {
    let path = state_path();
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_state(state: &PersistedState) -> Result<(), String> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_vec_pretty(state).map_err(|error| error.to_string())?;
    write_atomic(&path, &text)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temporary = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("tmp")
    ));
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    fs::rename(&temporary, path).map_err(|error| error.to_string())
}

pub fn random_nickname() -> String {
    const CONSONANTS: &[u8] = b"bcdfghjklmnpqrstvwxz";
    const VOWELS: &[u8] = b"aeiouy";
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0x1234_5678)
        ^ (std::process::id() as u64) << 32;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut name = String::with_capacity(6);
    for _ in 0..3 {
        name.push(CONSONANTS[(next() as usize) % CONSONANTS.len()] as char);
        name.push(VOWELS[(next() as usize) % VOWELS.len()] as char);
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_urls() {
        assert_eq!(
            normalize_url("localhost:16666/").as_deref(),
            Some("http://localhost:16666")
        );
        assert!(normalize_url("ftp://example.com").is_none());
    }

    #[test]
    fn parses_linuxfr_token() {
        let session = LinuxfrSession::from_pasted("abc123").unwrap();
        assert_eq!(session.access_token, "abc123");
        let json =
            r#"{"access_token":"tok","refresh_token":"ref","expires_in":3600,"login":"dave"}"#;
        let session = LinuxfrSession::from_pasted(json).unwrap();
        assert_eq!(session.login, "dave");
        assert_eq!(session.refresh_token, "ref");
    }

    #[test]
    fn board_validation() {
        assert!(
            Board::new("ok", "https://example.com/a", "https://example.com/b")
                .validate()
                .is_ok()
        );
        assert!(
            Board::new("bad name", "https://example.com/a", "https://example.com/b")
                .validate()
                .is_err()
        );
    }
}
