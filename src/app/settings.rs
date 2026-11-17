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
            SettingsMessage::Hungry(value) => self.settings.hungry = value,
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
            SettingsMessage::LinuxfrClientId(value) => self.settings.linuxfr_client_id = value,
            SettingsMessage::LinuxfrClientSecret(value) => {
                self.settings.linuxfr_client_secret = value
            }
            SettingsMessage::LinuxfrAuthorize => {
                tasks.extend(self.start_linuxfr_authorize());
            }
            SettingsMessage::LinuxfrConnect => {
                tasks.extend(self.start_linuxfr_connect());
            }
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

    /// Open the LinuxFR authorization page. With client credentials, roinroin
    /// uses LinuxFr's native redirect: LinuxFr displays a code that the user
    /// pastes back (LinuxFr rejects `http` redirect URIs and an `https` loopback
    /// would require a self-signed certificate). gb2c remains the fallback.
    pub(super) fn start_linuxfr_authorize(&mut self) -> Vec<Task<Message>> {
        let client_id = self.settings.linuxfr_client_id.trim().to_owned();
        let client_secret = self.settings.linuxfr_client_secret.trim().to_owned();
        if !client_id.is_empty() && !client_secret.is_empty() {
            self.linuxfr_oauth_client = Some((client_id.clone(), client_secret));
            self.settings.error = None;
            self.settings.info =
                Some("autorise LinuxFR, puis colle le code affiché dans le champ".into());
            return vec![Task::done(Message::OpenUrl(net::linuxfr_authorize_url(
                &client_id,
                config::LINUXFR_REDIRECT_URI,
            )))];
        }

        if let Some(server) = self
            .settings
            .servers
            .iter()
            .find(|server| server.enabled)
            .map(|server| server.url.trim_end_matches('/').to_owned())
        {
            return vec![Task::done(Message::OpenUrl(format!(
                "{server}/gb2c/linuxfr/authorize"
            )))];
        }
        self.settings.error = Some(
            "renseigne l'identifiant et le secret de l'application LinuxFR, \
             ou active un serveur gb2c"
                .into(),
        );
        Vec::new()
    }

    /// Exchange the code displayed by LinuxFr for an access token, directly
    /// against LinuxFr (no gb2c).
    pub(super) fn start_linuxfr_connect(&mut self) -> Vec<Task<Message>> {
        if self.linuxfr_connecting {
            self.settings.error = Some("connexion LinuxFR déjà en cours".into());
            return Vec::new();
        }
        let Some(code) = net::extract_linuxfr_code(&self.settings.linuxfr_paste) else {
            self.settings.error = Some("colle le code affiché par LinuxFR".into());
            return Vec::new();
        };
        let credentials = self.linuxfr_oauth_client.clone().or_else(|| {
            let client_id = self.settings.linuxfr_client_id.trim();
            let client_secret = self.settings.linuxfr_client_secret.trim();
            if client_id.is_empty() || client_secret.is_empty() {
                None
            } else {
                Some((client_id.to_owned(), client_secret.to_owned()))
            }
        });
        let Some((client_id, client_secret)) = credentials else {
            self.settings.error = Some(
                "identifiant et secret de l'application LinuxFR nécessaires pour \
                 échanger le code"
                    .into(),
            );
            return Vec::new();
        };
        self.linuxfr_oauth_client = Some((client_id.clone(), client_secret.clone()));
        self.linuxfr_connecting = true;
        self.settings.error = None;
        self.settings.info = Some("échange du code LinuxFR…".into());
        let client = self.client.clone();
        vec![Task::perform(
            async move {
                net::exchange_linuxfr_code(
                    &client,
                    &client_id,
                    &client_secret,
                    &code,
                    config::LINUXFR_REDIRECT_URI,
                )
                .await
            },
            Message::LinuxfrAuthorized,
        )]
    }

    /// Refresh the LinuxFR token without gb2c when application credentials are
    /// configured, and fall back to the gb2c endpoint otherwise. The error
    /// explains which piece is missing.
    pub(super) fn start_linuxfr_refresh(&mut self) -> Result<Task<Message>, String> {
        if self.linuxfr_refreshing {
            return Err("rafraîchissement LinuxFR déjà en cours".into());
        }
        let direct = self
            .config
            .linuxfr
            .as_ref()
            .is_some_and(|session| session.direct);
        let refresh_token = self
            .config
            .linuxfr
            .as_ref()
            .map(|session| session.refresh_token.clone())
            .filter(|token| !token.is_empty())
            .ok_or_else(|| "aucun refresh token LinuxFR".to_owned())?;
        let client = self.client.clone();
        if direct {
            let (client_id, client_secret) = self
                .config
                .linuxfr_direct_credentials()
                .ok_or_else(|| {
                    "identifiants de l'application LinuxFR manquants pour rafraîchir le jeton"
                        .to_owned()
                })?;
            let client_id = client_id.to_owned();
            let client_secret = client_secret.to_owned();
            self.linuxfr_refreshing = true;
            self.settings.info = Some("rafraîchissement LinuxFR…".into());
            Ok(Task::perform(
                async move {
                    net::refresh_linuxfr_direct(
                        &client,
                        &client_id,
                        &client_secret,
                        &refresh_token,
                    )
                    .await
                },
                Message::LinuxfrRefreshed,
            ))
        } else if let Some(server) = self
            .config
            .servers
            .iter()
            .find(|server| server.enabled)
            .map(|server| server.url.clone())
        {
            self.linuxfr_refreshing = true;
            self.settings.info = Some("rafraîchissement LinuxFR…".into());
            Ok(Task::perform(
                async move { net::refresh_linuxfr(&client, &server, &refresh_token).await },
                Message::LinuxfrRefreshed,
            ))
        } else {
            Err("aucun serveur gb2c pour rafraîchir le jeton LinuxFR".into())
        }
    }

    pub(super) fn apply_settings(&mut self) -> Vec<Task<Message>> {
        let mut config = self.config.clone();
        config.nickname = self.settings.nickname.trim().to_owned();
        config.totoz_server = self.settings.totoz_server.trim().to_owned();
        config.blocked = parse_csv(&self.settings.blocked);
        config.plop_list = parse_csv(&self.settings.plop);
        config.blocked_mode = self.settings.blocked_mode;
        config.hungry = self.settings.hungry;
        config.notifications = self.settings.notifications.clone();
        config.theme = self.settings.theme;
        config.language = self.settings.language;
        config.linuxfr_client_id = self.settings.linuxfr_client_id.trim().to_owned();
        config.linuxfr_client_secret = self.settings.linuxfr_client_secret.trim().to_owned();
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
