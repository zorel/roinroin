//! Settings behavior for the application.

use super::*;

impl App {
    pub(super) fn update_settings(&mut self, message: SettingsMessage) -> Vec<Task<Message>> {
        let mut tasks = Vec::new();
        match message {
            SettingsMessage::Nickname(value) => self.settings.nickname = value,
            SettingsMessage::TotozServer(value) => self.settings.totoz_server = value,
            SettingsMessage::Blocked(value) => self.settings.blocked = value,
            SettingsMessage::Plop(value) => self.settings.plop = value,
            SettingsMessage::BlockedMode(mode) => self.settings.blocked_mode = mode,
            SettingsMessage::ShowHidden(value) => self.settings.show_hidden = value,
            SettingsMessage::Theme(theme) => self.settings.theme = theme,
            SettingsMessage::NotifyEnabled(value) => self.settings.notifications.enabled = value,
            SettingsMessage::NotifyAll(value) => self.settings.notifications.all_posts = value,
            SettingsMessage::NotifyBigorno(value) => self.settings.notifications.bigorno = value,
            SettingsMessage::NotifyReplies(value) => self.settings.notifications.replies = value,
            SettingsMessage::NotifyOnlyUnfocused(value) => {
                self.settings.notifications.only_unfocused = value
            }
            SettingsMessage::ToggleBoard(name) => {
                if !self.settings.selected_boards.remove(&name) {
                    self.settings.selected_boards.insert(name);
                }
            }
            SettingsMessage::ToggleTribune(key) => {
                if !self.settings.hidden_tribunes.remove(&key) {
                    self.settings.hidden_tribunes.insert(key);
                }
            }
            SettingsMessage::ServerUrlInput(value) => self.settings.server_input = value,
            SettingsMessage::AddServer => {
                let value = self.settings.server_input.trim();
                if let Some(url) = config::normalize_url(value) {
                    let name = url
                        .trim_start_matches("https://")
                        .trim_start_matches("http://")
                        .split('/')
                        .next()
                        .unwrap_or("gb2c")
                        .to_owned();
                    self.settings.servers.push(ServerEdit {
                        name,
                        url,
                        enabled: true,
                    });
                    self.settings.server_input.clear();
                    self.settings.error = None;
                } else {
                    self.settings.error = Some("URL gb2c invalide".into());
                }
            }
            SettingsMessage::RemoveServer(index) => {
                if index < self.settings.servers.len() {
                    self.settings.servers.remove(index);
                }
            }
            SettingsMessage::ToggleServer(index) => {
                if let Some(server) = self.settings.servers.get_mut(index) {
                    server.enabled = !server.enabled;
                }
            }
            SettingsMessage::ServerName(index, value) => {
                if let Some(server) = self.settings.servers.get_mut(index) {
                    server.name = value;
                }
            }
            SettingsMessage::ServerUrl(index, value) => {
                if let Some(server) = self.settings.servers.get_mut(index) {
                    server.url = value;
                }
            }
            SettingsMessage::BoardName(value) => self.settings.board_name = value,
            SettingsMessage::BoardBackend(value) => self.settings.board_backend = value,
            SettingsMessage::BoardPost(value) => self.settings.board_post = value,
            SettingsMessage::BoardPostField(value) => self.settings.board_post_field = value,
            SettingsMessage::BoardInfoField(value) => self.settings.board_info_field = value,
            SettingsMessage::BoardOauth(value) => self.settings.board_oauth = value,
            SettingsMessage::AddBoard => {
                let board = Board {
                    name: self.settings.board_name.trim().to_owned(),
                    backend_url: self.settings.board_backend.trim().to_owned(),
                    post_url: self.settings.board_post.trim().to_owned(),
                    post_field: self.settings.board_post_field.trim().to_owned(),
                    info_field: self.settings.board_info_field.trim().to_owned(),
                    auth: if self.settings.board_oauth {
                        BoardAuth::Oauth2
                    } else {
                        BoardAuth::None
                    },
                };
                if let Err(error) = board.validate() {
                    self.settings.error = Some(error);
                } else if self
                    .settings
                    .custom_boards
                    .iter()
                    .any(|existing| existing.name == board.name)
                    || crate::config::predefined_boards()
                        .iter()
                        .any(|existing| existing.name == board.name)
                {
                    self.settings.error = Some("ce board existe déjà".into());
                } else {
                    let board = board.normalized();
                    self.settings.selected_boards.insert(board.name.clone());
                    self.settings.custom_boards.push(board);
                    self.settings.board_name.clear();
                    self.settings.board_backend.clear();
                    self.settings.board_post.clear();
                    self.settings.board_post_field = config::default_post_field();
                    self.settings.board_info_field.clear();
                    self.settings.board_oauth = false;
                    self.settings.error = None;
                }
            }
            SettingsMessage::RemoveBoard(name) => {
                self.settings
                    .custom_boards
                    .retain(|board| board.name != name);
                self.settings.selected_boards.remove(&name);
            }
            SettingsMessage::LinuxfrPaste(value) => self.settings.linuxfr_paste = value,
            SettingsMessage::Apply => {
                tasks.extend(self.apply_settings());
            }
            SettingsMessage::Cancel => {
                self.settings = SettingsDraft::from_config(&self.config);
                self.screen = Screen::Main;
            }
        }
        tasks
    }

    pub(super) fn apply_settings(&mut self) -> Vec<Task<Message>> {
        let mut config = self.config.clone();
        config.nickname = self.settings.nickname.trim().to_owned();
        config.totoz_server = self.settings.totoz_server.trim().to_owned();
        config.blocked = parse_csv(&self.settings.blocked);
        config.plop_list = parse_csv(&self.settings.plop);
        config.blocked_mode = self.settings.blocked_mode;
        config.show_hidden = self.settings.show_hidden;
        config.notifications = self.settings.notifications.clone();
        config.theme = self.settings.theme;
        config.language = self.settings.language;
        config.selected_boards = self.settings.selected_boards.clone();
        config.hidden_tribunes = self.settings.hidden_tribunes.clone();
        config.custom_boards = self.settings.custom_boards.clone();
        config.servers = Vec::new();
        for server in &self.settings.servers {
            let url = match config::normalize_url(&server.url) {
                Some(url) => url,
                None => {
                    self.settings.error = Some(format!("URL gb2c invalide : {}", server.url));
                    return Vec::new();
                }
            };
            let name = if server.name.trim().is_empty() {
                url.clone()
            } else {
                server.name.trim().to_owned()
            };
            config.servers.push(ServerConfig {
                name,
                url,
                enabled: server.enabled,
            });
        }
        if config.servers.is_empty() && config.selected_boards.is_empty() {
            self.settings.error =
                Some("ajoute au moins un serveur gb2c ou active un board direct".into());
            return Vec::new();
        }
        if !self.settings.linuxfr_paste.trim().is_empty() {
            match LinuxfrSession::from_pasted(&self.settings.linuxfr_paste) {
                Some(mut session) => {
                    if session.login.is_empty() {
                        if let Some(previous) = self.config.linuxfr.as_ref() {
                            session.login = previous.login.clone();
                        }
                    }
                    config.linuxfr = Some(session);
                }
                None => {
                    self.settings.error = Some("jeton LinuxFR illisible".into());
                    return Vec::new();
                }
            }
        }
        if let Err(error) = config.validate_for_save() {
            self.settings.error = Some(error);
            return Vec::new();
        }
        if let Err(error) = config::save_config(&config) {
            self.settings.error = Some(error);
            return Vec::new();
        }
        self.config = config;
        self.settings = SettingsDraft::from_config(&self.config);
        self.sync_sources();
        self.retain_current_sources();
        self.analysis_dirty = true;
        self.screen = Screen::Main;
        self.status = "configuration appliquée".to_owned();
        self.error = None;
        self.refresh_tasks()
    }
}
