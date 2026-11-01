//! Application state, update loop and subscriptions.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use chrono::{DateTime, Timelike, Utc};
use chrono_tz::Europe::Paris;
use futures::StreamExt;
use iced::keyboard;
use iced::widget::{image, Id};
use iced::{mouse, window, Point, Size, Subscription, Task, Theme};

use crate::config::{
    self, BlockedMode, Board, BoardAuth, Config, Language, LinuxfrSession, NotifyConfig,
    PersistedState, ServerConfig, ThemeMode,
};
use crate::conversation::{self, Analysis};
use crate::message::{self, ParsedMessage, Token};
use crate::model::{quote_clock, Post, PostKey, SourceId, TribuneKey};
use crate::net;
use crate::notify;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkAction {
    Url(String),
    Jump(PostKey),
    Spoiler(PostKey, usize),
    Totoz(String),
    Info(String, PostKey),
    Unresolved(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Main,
    Settings,
    Totoz,
    Emoji,
}

#[derive(Debug, Clone)]
pub struct ServerState {
    pub name: String,
    pub url: String,
    pub known: BTreeSet<String>,
    pub blocked: BTreeSet<String>,
    pub connected: bool,
    pub fetching: bool,
    pub last_error: Option<String>,
    pub last_event: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Online,
    Error,
    Working,
}

#[derive(Debug, Clone, Default)]
pub struct BoardState {
    pub connected: bool,
    pub fetching: bool,
    pub last_error: Option<String>,
    pub last_fetch: Option<Instant>,
}

#[derive(Debug, Clone, Default)]
pub struct TotozState {
    pub query: String,
    pub loading: bool,
    pub results: Vec<net::Totoz>,
    pub images: HashMap<String, TotozImage>,
    pub error: Option<String>,
}

/// Inline totoz autocompletion shown in the composer.
///
/// The composer token starts with `[:`; once at least
/// [`TOTOZ_SUGGEST_MIN_CHARS`] characters follow it, matching totoz names are
/// listed right above the entry field. An empty `results` list means nothing
/// is displayed, so an unknown prefix never leaves an empty dropdown behind.
#[derive(Debug, Clone, Default)]
pub struct TotozSuggest {
    pub query: String,
    pub results: Vec<net::Totoz>,
    pub selected: Option<usize>,
    /// Incremented on every query change to discard stale network responses.
    pub generation: u64,
    /// Previews already requested for suggestions: avoids fetching the same
    /// image again while it is loading or after the server returned none.
    requested_images: HashSet<String>,
}

impl TotozSuggest {
    pub fn visible(&self) -> bool {
        !self.results.is_empty()
    }

    fn clear(&mut self) {
        self.query.clear();
        self.results.clear();
        self.selected = None;
        self.generation = self.generation.wrapping_add(1);
    }

    /// Close the list without forgetting the typed term, so merely hitting
    /// Escape does not immediately reopen it on the next keystroke.
    fn dismiss(&mut self) {
        self.results.clear();
        self.selected = None;
        self.generation = self.generation.wrapping_add(1);
    }
}

/// Characters needed after `[:` before autocompletion starts.
const TOTOZ_SUGGEST_MIN_CHARS: usize = 2;
/// Keep the list short enough to scan at a glance (Baymard: at most ten).
const TOTOZ_SUGGEST_LIMIT: usize = 8;
/// Short pause so typing does not hit the totoz server on every keystroke.
const TOTOZ_SUGGEST_DELAY: Duration = Duration::from_millis(150);

#[derive(Debug, Clone)]
pub struct TotozImage {
    pub frames: Vec<image::Handle>,
    pub delays: Vec<Duration>,
    pub width: u32,
    pub height: u32,
}

impl TotozImage {
    pub fn first_frame(&self) -> &image::Handle {
        &self.frames[0]
    }
}

#[derive(Debug, Clone, Default)]
pub struct EmojiState {
    pub query: String,
    pub results: Vec<net::Emoji>,
}

#[derive(Debug, Clone)]
pub struct HoveredTotoz {
    pub name: String,
    pub image: Option<TotozImage>,
    pub anchor: Point,
    pub frame_index: usize,
    pub frame_started: Instant,
    pub loading: bool,
    pub error: Option<String>,
}

const LINK_PREVIEW_CACHE_LIMIT: usize = 20;

/// Metadata cached for a link found in a post.
#[derive(Debug, Clone)]
pub enum LinkPreview {
    Loading,
    Title(String),
    Image {
        handle: image::Handle,
        width: u32,
        height: u32,
    },
    Error,
}

#[derive(Debug, Clone)]
pub struct HoveredLinkPreview {
    pub url: String,
    pub anchor: Point,
}

#[derive(Debug, Clone)]
pub struct ServerEdit {
    pub name: String,
    pub url: String,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub struct SettingsDraft {
    pub nickname: String,
    pub totoz_server: String,
    pub blocked: String,
    pub plop: String,
    pub blocked_mode: BlockedMode,
    pub show_hidden: bool,
    pub notifications: NotifyConfig,
    pub theme: ThemeMode,
    pub language: Language,
    pub selected_boards: BTreeSet<String>,
    pub hidden_tribunes: BTreeSet<String>,
    pub servers: Vec<ServerEdit>,
    pub custom_boards: Vec<Board>,
    pub server_input: String,
    pub board_name: String,
    pub board_backend: String,
    pub board_post: String,
    pub board_post_field: String,
    pub board_info_field: String,
    pub board_oauth: bool,
    pub linuxfr_paste: String,
    pub error: Option<String>,
    pub info: Option<String>,
}

impl SettingsDraft {
    pub fn from_config(config: &Config) -> Self {
        Self {
            nickname: config.nickname.clone(),
            totoz_server: config.totoz_server.clone(),
            blocked: config
                .blocked
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            plop: config
                .plop_list
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            blocked_mode: config.blocked_mode,
            show_hidden: config.show_hidden,
            notifications: config.notifications.clone(),
            theme: config.theme,
            language: config.language,
            selected_boards: config.selected_boards.clone(),
            hidden_tribunes: config.hidden_tribunes.clone(),
            servers: config
                .servers
                .iter()
                .map(|server| ServerEdit {
                    name: server.name.clone(),
                    url: server.url.clone(),
                    enabled: server.enabled,
                })
                .collect(),
            custom_boards: config.custom_boards.clone(),
            server_input: String::new(),
            board_name: String::new(),
            board_backend: String::new(),
            board_post: String::new(),
            board_post_field: config::default_post_field(),
            board_info_field: String::new(),
            board_oauth: false,
            linuxfr_paste: String::new(),
            error: None,
            info: None,
        }
    }
}

#[derive(Debug, Clone)]
pub enum SettingsMessage {
    Nickname(String),
    TotozServer(String),
    Blocked(String),
    Plop(String),
    BlockedMode(BlockedMode),
    ShowHidden(bool),
    Theme(ThemeMode),
    NotifyEnabled(bool),
    NotifyAll(bool),
    NotifyBigorno(bool),
    NotifyReplies(bool),
    NotifyOnlyUnfocused(bool),
    ToggleBoard(String),
    ToggleTribune(String),
    ServerUrlInput(String),
    AddServer,
    RemoveServer(usize),
    ToggleServer(usize),
    ServerName(usize, String),
    ServerUrl(usize, String),
    BoardName(String),
    BoardBackend(String),
    BoardPost(String),
    BoardPostField(String),
    BoardInfoField(String),
    BoardOauth(bool),
    AddBoard,
    RemoveBoard(String),
    LinuxfrPaste(String),
    Apply,
    Cancel,
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    WindowEvent(window::Id, window::Event),
    Keyboard(keyboard::Event),
    ServerInfo(String, Result<net::ServerInfo, String>),
    BoardPosts(String, Result<Vec<Post>, String>),
    Sse(String, net::SseEvent),
    SelectTribune(TribuneKey),
    PostClicked(PostKey),
    ClearSelection,
    JumpTo(PostKey),
    ToggleBlockedMode,
    ToggleShowHidden,
    MarkAllRead,
    Scrolled(f32),
    Composer(String),
    ComposerSubmit,
    Insert(String),
    TotozPicked(String),
    TotozSuggestSearch(u64),
    TotozSuggestLoaded(u64, Result<Vec<net::Totoz>, String>),
    TotozSuggestImage(String, Result<Vec<u8>, String>),
    TotozSuggestPick(String),
    TotozSuggestDismiss,
    Send,
    FlushSse,
    CursorMoved(Point),
    SidebarDragStart,
    SidebarDragEnd,
    Posted(String, Result<(), String>),
    RefreshAll,
    PollBoards,
    TotozQuery(String),
    TotozSearch,
    TotozLoaded(Result<Vec<net::Totoz>, String>),
    TotozImages(Vec<(String, Result<Vec<u8>, String>)>),
    EmojiQuery(String),
    EmojiSearch,
    EmojiLoaded(Vec<net::Emoji>),
    Settings(SettingsMessage),
    OpenSettings,
    OpenTotoz,
    OpenEmoji,
    CloseScreen,
    Spoiler(PostKey, usize),
    LinkClicked(LinkAction),
    OpenUrl(String),
    HoverPost(Option<PostKey>),
    HoverExit,
    HoverLink(Option<LinkAction>, Point),
    PreviewEnter,
    PreviewExit,
    HidePreview(u64),
    PostShown(PostKey),
    PostHidden(PostKey),
    TotozHoverImage(String, Result<Vec<u8>, String>),
    LinkFetched(String, Result<net::LinkMetadata, String>),
    TotozFrameTick,
    LinuxfrRefreshed(Result<LinuxfrSession, String>),
    RefreshLinuxfr,
    Info(String),
    Error(String),
    Noop,
}

pub struct App {
    pub(crate) config: Config,
    pub(crate) persisted: PersistedState,
    pub(crate) screen: Screen,
    pub(crate) servers: BTreeMap<String, ServerState>,
    pub(crate) boards: BTreeMap<String, BoardState>,
    pub(crate) posts: BTreeMap<PostKey, Post>,
    pub(crate) analysis: Analysis,
    pub(crate) analysis_dirty: bool,
    pub(crate) active: Option<TribuneKey>,
    pub(crate) selected: Option<PostKey>,
    pub(crate) hovered_post: Option<PostKey>,
    pub(crate) hovered_link_post: Option<PostKey>,
    pub(crate) hovered_info: Option<String>,
    pub(crate) hovered_totoz: Option<HoveredTotoz>,
    pub(crate) hovered_link_preview: Option<HoveredLinkPreview>,
    pub(crate) link_previews: HashMap<String, LinkPreview>,
    pub(crate) link_image_order: VecDeque<String>,
    pub(crate) link_title_order: VecDeque<String>,
    pub(crate) hovered_preview: Option<PostKey>,
    pub(crate) preview_hovered: bool,
    pub(crate) preview_epoch: u64,
    pub(crate) visible_posts: HashSet<PostKey>,
    pub(crate) window_size: Size,
    pub(crate) sidebar_width: f32,
    pub(crate) sidebar_dragging: bool,
    pub(crate) scroll_offset: f32,
    pub(crate) composer: String,
    pub(crate) totoz_suggest: TotozSuggest,
    pub(crate) draft_dirty: bool,
    pub(crate) state_dirty: bool,
    pub(crate) settings: SettingsDraft,
    pub(crate) totoz: TotozState,
    pub(crate) emoji: EmojiState,
    pub(crate) status: String,
    pub(crate) error: Option<String>,
    pub(crate) focused: bool,
    pub(crate) now: DateTime<chrono_tz::Tz>,
    pub(crate) pre_existing_tribunes: HashSet<TribuneKey>,
    pub(crate) client: reqwest::Client,
    pub(crate) seen: BTreeSet<PostKey>,
    pub(crate) pending_notifications: Vec<PostKey>,
    pub(crate) pending_sse: Vec<(String, Post)>,
    pub(crate) sse_flush_scheduled: bool,
    pub(crate) reveal: HashSet<(PostKey, usize)>,
    pub(crate) scroll_to_bottom: bool,
    pub(crate) posts_scroll_id: Id,
    pub(crate) composer_id: Id,
    pub(crate) totoz_search_id: Id,
    pub(crate) emoji_search_id: Id,
    pub(crate) linuxfr_refreshing: bool,
    pub(crate) sse_epoch: u64,
    pub(crate) linuxfr_retry_at: Option<Instant>,
    pub(crate) sending: bool,
    pub(crate) pending_post: Option<String>,
    pub(crate) plop_parsed: ParsedMessage,
    pub(crate) plop_cache: HashMap<PostKey, ParsedMessage>,
    pub(crate) last_hourly_bell: Option<String>,
    pub(crate) window_id: Option<window::Id>,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let (config, warning) = config::load_config();
        let persisted = config::load_state();
        let sidebar_width = if persisted.sidebar_width >= 160.0 {
            persisted.sidebar_width
        } else {
            250.0
        };
        let pre_existing_tribunes = persisted
            .last_read
            .keys()
            .filter_map(|key| TribuneKey::from_key(key))
            .collect();
        let now = Utc::now().with_timezone(&Paris);
        let mut app = Self {
            composer: config.draft.clone(),
            totoz_suggest: TotozSuggest::default(),
            settings: SettingsDraft::from_config(&config),
            config,
            persisted,
            screen: Screen::Main,
            servers: BTreeMap::new(),
            boards: BTreeMap::new(),
            posts: BTreeMap::new(),
            analysis: Analysis::default(),
            analysis_dirty: true,
            active: None,
            selected: None,
            hovered_post: None,
            hovered_link_post: None,
            hovered_info: None,
            hovered_totoz: None,
            hovered_link_preview: None,
            link_previews: HashMap::new(),
            link_image_order: VecDeque::new(),
            link_title_order: VecDeque::new(),
            hovered_preview: None,
            preview_hovered: false,
            preview_epoch: 0,
            visible_posts: HashSet::new(),
            window_size: Size::new(1180.0, 820.0),
            sidebar_width,
            sidebar_dragging: false,
            scroll_offset: 0.0,
            draft_dirty: false,
            state_dirty: false,
            totoz: TotozState::default(),
            emoji: EmojiState::default(),
            status: "connexion…".to_owned(),
            error: None,
            focused: true,
            now,
            pre_existing_tribunes,
            client: net::client(),
            seen: BTreeSet::new(),
            pending_notifications: Vec::new(),
            pending_sse: Vec::new(),
            sse_flush_scheduled: false,
            reveal: HashSet::new(),
            scroll_to_bottom: true,
            posts_scroll_id: Id::new("roinroin-posts"),
            composer_id: Id::new("roinroin-composer"),
            totoz_search_id: Id::new("roinroin-totoz-search"),
            emoji_search_id: Id::new("roinroin-emoji-search"),
            linuxfr_refreshing: false,
            sse_epoch: 0,
            linuxfr_retry_at: None,
            sending: false,
            pending_post: None,
            plop_parsed: message::parse("plop"),
            plop_cache: HashMap::new(),
            last_hourly_bell: None,
            window_id: None,
        };
        app.sync_sources();
        app.active = app
            .persisted
            .active
            .as_deref()
            .and_then(TribuneKey::from_key)
            .filter(|key| app.tribunes().contains(key));
        app.ensure_analysis();
        if app.active.is_none() {
            app.active = app.tribunes().first().cloned();
        }
        let mut tasks = app.refresh_tasks();
        if app.active.is_some() {
            // Start with the caret in the entry zone, ready to type.
            tasks.push(focus_text_input(&app.composer_id));
        }
        let width = app.persisted.window_width;
        let height = app.persisted.window_height;
        if width >= 640.0 && height >= 420.0 {
            tasks.push(
                window::latest()
                    .and_then(move |id| window::resize(id, iced::Size::new(width, height))),
            );
        }
        if let Some(warning) = warning {
            tasks.push(Task::done(Message::Info(warning)));
        }
        (app, Task::batch(tasks))
    }

    pub fn title(&self) -> String {
        let unread = self.total_badges();
        match &self.active {
            Some(active) if unread > 0 => format!("{active} ({unread}) — roinroin"),
            Some(active) => format!("{active} — roinroin"),
            None => "roinroin".to_owned(),
        }
    }

    pub fn theme(&self) -> Theme {
        let mode = if self.screen == Screen::Settings {
            self.settings.theme
        } else {
            self.config.theme
        };
        crate::theme::theme(mode)
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            iced::time::every(Duration::from_secs(30)).map(|_| Message::PollBoards),
            window::events().map(|(id, event)| Message::WindowEvent(id, event)),
            keyboard::listen().filter_map(keyboard_shortcut),
        ];
        if self.sidebar_dragging {
            subscriptions.push(iced::event::listen_with(
                |event, _status, _window| match event {
                    iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                        Some(Message::CursorMoved(position))
                    }
                    iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                        Some(Message::SidebarDragEnd)
                    }
                    _ => None,
                },
            ));
        }
        // The text input consumes Escape to blur itself, so watch the raw
        // events to close the totoz suggestions without leaving the field.
        if self.screen == Screen::Main && !self.totoz_suggest.query.is_empty() {
            subscriptions.push(iced::event::listen_with(
                |event, _status, _window| match event {
                    iced::Event::Keyboard(keyboard::Event::KeyPressed {
                        key: keyboard::Key::Named(keyboard::key::Named::Escape),
                        ..
                    }) => Some(Message::TotozSuggestDismiss),
                    _ => None,
                },
            ));
        }
        for server in self.config.servers.iter().filter(|server| server.enabled) {
            let url = config::normalize_url(&server.url).unwrap_or_else(|| server.url.clone());
            let state = self.servers.get(&url);
            let known = state.map(|state| &state.known);
            let filter = known
                .map(|known| {
                    known
                        .iter()
                        .filter(|name| {
                            let key = TribuneKey::new(SourceId::Gb2c(url.clone()), (*name).clone());
                            !self.config.hidden_tribunes.contains(&key.key())
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let config = net::SseConfig {
                server_url: url,
                filter,
                epoch: self.sse_epoch,
            };
            subscriptions.push(Subscription::run_with(config, build_sse));
        }
        if self.hovered_totoz.as_ref().is_some_and(|totoz| {
            totoz
                .image
                .as_ref()
                .is_some_and(|image| image.frames.len() > 1)
        }) {
            subscriptions.push(
                iced::time::every(Duration::from_millis(80)).map(|_| Message::TotozFrameTick),
            );
        }
        Subscription::batch(subscriptions)
    }

    // ---------------------------------------------------------------------
    // Update

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let mut tasks: Vec<Task<Message>> = Vec::new();
        match message {
            Message::Tick => {
                self.now = Utc::now().with_timezone(&Paris);
                if self.draft_dirty {
                    self.config.draft = self.composer.clone();
                    if let Err(error) = config::save_config(&self.config) {
                        self.error = Some(error);
                    }
                    self.draft_dirty = false;
                }
                if self.state_dirty {
                    self.persisted.active = self.active.as_ref().map(TribuneKey::key);
                    self.persisted.sidebar_width = self.sidebar_width;
                    if let Err(error) = config::save_state(&self.persisted) {
                        self.error = Some(error);
                    }
                    self.state_dirty = false;
                }
                if self.focused {
                    self.mark_active_read();
                }
                if self.seen.len() > 20_000 {
                    self.seen.retain(|key| self.posts.contains_key(key));
                }
                if self.config.hourly_bell && self.now.minute() == 0 && self.now.second() < 2 {
                    let stamp = self.now.format("%Y%m%d%H").to_string();
                    if self.last_hourly_bell.as_deref() != Some(&stamp) {
                        self.last_hourly_bell = Some(stamp);
                        let summary = format!("{}:00", self.now.format("%H:%M"));
                        tasks.push(Task::perform(
                            async move {
                                notify::send("roinroin", &format!("{summary} — heure pile"), false)
                            },
                            |_| Message::Noop,
                        ));
                    }
                }
                if !self.linuxfr_refreshing
                    && self
                        .linuxfr_retry_at
                        .is_none_or(|retry| Instant::now() >= retry)
                {
                    if let Some(session) = self.config.linuxfr.as_ref() {
                        if session.is_expired() && !session.refresh_token.is_empty() {
                            if let Some(server) = self
                                .config
                                .servers
                                .iter()
                                .find(|server| server.enabled)
                                .map(|server| server.url.clone())
                            {
                                self.linuxfr_refreshing = true;
                                let client = self.client.clone();
                                let refresh = session.refresh_token.clone();
                                tasks.push(Task::perform(
                                    async move {
                                        net::refresh_linuxfr(&client, &server, &refresh).await
                                    },
                                    Message::LinuxfrRefreshed,
                                ));
                            }
                        }
                    }
                }
            }
            Message::WindowEvent(id, event) => {
                self.window_id = Some(id);
                match event {
                    window::Event::Focused => {
                        self.focused = true;
                        self.mark_active_read();
                    }
                    window::Event::Unfocused => {
                        self.focused = false;
                        self.config.draft = self.composer.clone();
                        self.persisted.active = self.active.as_ref().map(TribuneKey::key);
                        self.persisted.sidebar_width = self.sidebar_width;
                        let _ = config::save_config(&self.config);
                        let _ = config::save_state(&self.persisted);
                    }
                    window::Event::Resized(size) => {
                        self.window_size = size;
                        let max = (size.width - 360.0).max(240.0);
                        self.sidebar_width = self.sidebar_width.clamp(160.0, max);
                    }
                    window::Event::CloseRequested => {
                        self.config.draft = self.composer.clone();
                        self.persisted.active = self.active.as_ref().map(TribuneKey::key);
                        self.persisted.sidebar_width = self.sidebar_width;
                        let _ = config::save_config(&self.config);
                        let _ = config::save_state(&self.persisted);
                        tasks.push(window::close(id));
                    }
                    _ => {}
                }
            }
            Message::Keyboard(event) => {
                tasks.extend(self.handle_keyboard(event));
            }
            Message::ServerInfo(url, result) => {
                let configured = self.config.servers.iter().any(|server| {
                    config::normalize_url(&server.url).as_deref() == Some(url.as_str())
                });
                if !configured {
                    self.ensure_analysis();
                    return Task::batch(tasks);
                }
                match result {
                    Ok(info) => {
                        let name = self
                            .config
                            .servers
                            .iter()
                            .find(|server| {
                                config::normalize_url(&server.url).as_deref() == Some(url.as_str())
                            })
                            .map(|server| server.name.clone())
                            .unwrap_or_else(|| url.clone());
                        let state =
                            self.servers
                                .entry(url.clone())
                                .or_insert_with(|| ServerState {
                                    name,
                                    url: url.clone(),
                                    known: BTreeSet::new(),
                                    blocked: BTreeSet::new(),
                                    connected: false,
                                    fetching: false,
                                    last_error: None,
                                    last_event: None,
                                });
                        state.known = info.tribunes.into_iter().collect();
                        state.blocked = info.blocked;
                        state.connected = true;
                        state.fetching = false;
                        state.last_error = None;
                        state.last_event = Some(Instant::now());
                        self.persisted
                            .known_tribunes
                            .insert(url, state.known.iter().cloned().collect());
                        self.state_dirty = true;
                        self.analysis_dirty = true;
                        self.status = "connecté".to_owned();
                        self.error = None;
                        if self.active.is_none() {
                            self.active = self.tribunes().first().cloned();
                            if self.active.is_some() && self.screen == Screen::Main {
                                // The entry zone just appeared: put the caret
                                // in it as on startup.
                                tasks.push(focus_text_input(&self.composer_id));
                            }
                        }
                    }
                    Err(error) => {
                        if let Some(state) = self.servers.get_mut(&url) {
                            state.connected = false;
                            state.fetching = false;
                            state.last_error = Some(error.clone());
                        }
                        self.error = Some(format!("{url} : {error}"));
                        self.status = "hors ligne".to_owned();
                    }
                }
            }
            Message::BoardPosts(name, result) => {
                if !self.config.selected_boards.contains(&name) {
                    self.ensure_analysis();
                    return Task::batch(tasks);
                }
                if let Some(state) = self.boards.get_mut(&name) {
                    state.fetching = false;
                    state.last_fetch = Some(Instant::now());
                    match result {
                        Ok(posts) => {
                            state.connected = true;
                            state.last_error = None;
                            self.error = None;
                            tasks.extend(self.ingest(
                                SourceId::Direct(name.clone()),
                                posts,
                                Some(&name),
                            ));
                            if self.selected.is_none()
                                && self.active.as_ref().is_some_and(|active| {
                                    active.source.kind() == "direct" && active.name == name
                                })
                            {
                                tasks.push(iced::widget::operation::snap_to_end(
                                    self.posts_scroll_id.clone(),
                                ));
                            }
                        }
                        Err(error) => {
                            state.connected = false;
                            let previous = state.last_error.replace(error.clone());
                            if previous.as_deref() != Some(error.as_str()) {
                                self.error = Some(format!("{name} : {error}"));
                            }
                        }
                    }
                } else if let Ok(posts) = result {
                    tasks.extend(self.ingest(SourceId::Direct(name.clone()), posts, Some(&name)));
                }
            }
            Message::Sse(server, event) => match event {
                net::SseEvent::Connected => {
                    if let Some(state) = self.servers.get_mut(&server) {
                        state.connected = true;
                        state.last_error = None;
                        state.last_event = Some(Instant::now());
                    }
                    self.status = "connecté".to_owned();
                    self.error = None;
                }
                net::SseEvent::Disconnected(error) => {
                    if let Some(state) = self.servers.get_mut(&server) {
                        state.connected = false;
                        state.last_error = Some(error.clone());
                    }
                    self.status = "reconnexion…".to_owned();
                    self.error = Some(format!("{server} : {error}"));
                }
                net::SseEvent::Post(post) => {
                    let configured = self.config.servers.iter().any(|cfg| {
                        config::normalize_url(&cfg.url).as_deref() == Some(server.as_str())
                    });
                    if !configured {
                        return Task::batch(tasks);
                    }
                    // A reconnecting SSE stream replays the whole tribune
                    // history one event at a time. Buffer posts and ingest
                    // them in batches so the conversation analysis is not
                    // rebuilt once per post while the backlog arrives.
                    self.pending_sse.push((server, post));
                    if !self.sse_flush_scheduled {
                        self.sse_flush_scheduled = true;
                        tasks.push(Task::perform(
                            async {
                                tokio::time::sleep(Duration::from_millis(100)).await;
                            },
                            |_| Message::FlushSse,
                        ));
                    }
                }
            },
            Message::FlushSse => {
                self.sse_flush_scheduled = false;
                let pending = std::mem::take(&mut self.pending_sse);
                if !pending.is_empty() {
                    let mut grouped: BTreeMap<String, Vec<Post>> = BTreeMap::new();
                    for (server, post) in pending {
                        grouped.entry(server).or_default().push(post);
                    }
                    let mut active_received = false;
                    for (server, posts) in grouped {
                        let label = self
                            .servers
                            .get(&server)
                            .map(|state| state.name.clone())
                            .unwrap_or_else(|| server.clone());
                        if self.active.as_ref().is_some_and(
                            |active| matches!(&active.source, SourceId::Gb2c(url) if url == &server),
                        ) {
                            active_received = true;
                        }
                        tasks.extend(self.ingest(SourceId::Gb2c(server), posts, Some(&label)));
                    }
                    if active_received && self.selected.is_none() {
                        tasks.push(iced::widget::operation::snap_to_end(
                            self.posts_scroll_id.clone(),
                        ));
                    }
                }
            }
            Message::SelectTribune(key) => {
                if self.active.as_ref() != Some(&key) {
                    if let Some(previous) = self.active.take() {
                        self.mark_read(&previous);
                        let draft = self.composer.clone();
                        if !draft.is_empty() {
                            let rewritten =
                                message::rewrite_bouchot_suffix(&draft, &previous.name, &key.name);
                            if rewritten != draft {
                                self.composer = rewritten;
                                self.draft_dirty = true;
                                tasks.extend(self.refresh_totoz_suggest());
                            }
                        }
                    }
                    self.active = Some(key);
                    tasks.push(focus_text_input(&self.composer_id));
                    self.selected = None;
                    self.hovered_post = None;
                    self.hovered_link_post = None;
                    self.hovered_info = None;
                    self.hovered_totoz = None;
                    self.hovered_link_preview = None;
                    self.hovered_preview = None;
                    self.visible_posts.clear();
                    self.scroll_to_bottom = true;
                    self.mark_active_read();
                    self.state_dirty = true;
                    tasks.push(iced::widget::operation::snap_to_end(
                        self.posts_scroll_id.clone(),
                    ));
                }
            }
            Message::PostClicked(key) => {
                // Only the clock area is a click target (it quotes the post).
                // Clicking the rest of the card must not select/highlight it.
                if self.hovered_post.as_ref() == Some(&key) {
                    if let Some(post) = self.posts.get(&key) {
                        let quote = format!("{} ", quote_clock(post));
                        tasks.push(self.insert_composer(&quote));
                    }
                }
            }
            Message::ClearSelection => {
                self.selected = None;
            }
            Message::JumpTo(key) => {
                if self.active.as_ref() != Some(&key.tribune) {
                    self.active = Some(key.tribune.clone());
                    self.mark_active_read();
                    self.state_dirty = true;
                    self.visible_posts.clear();
                }
                self.hovered_preview = None;
                self.selected = Some(key.clone());
                if let Some(index) = self.visible_keys().iter().position(|item| item == &key) {
                    let total = self.visible_keys().len().max(1);
                    let y = (index as f32 / total as f32).clamp(0.0, 0.98);
                    tasks.push(iced::widget::operation::snap_to(
                        self.posts_scroll_id.clone(),
                        iced::widget::scrollable::RelativeOffset { x: 0.0, y },
                    ));
                }
            }
            Message::ToggleBlockedMode => {
                self.config.blocked_mode = match self.config.blocked_mode {
                    BlockedMode::Hide => BlockedMode::Gray,
                    BlockedMode::Gray => BlockedMode::Hide,
                };
                self.settings.blocked_mode = self.config.blocked_mode;
                self.analysis_dirty = true;
                self.save_config_quietly();
            }
            Message::ToggleShowHidden => {
                self.config.show_hidden = !self.config.show_hidden;
                self.settings.show_hidden = self.config.show_hidden;
                self.analysis_dirty = true;
                self.save_config_quietly();
            }
            Message::MarkAllRead => {
                for tribune in self.tribunes() {
                    self.mark_read(&tribune);
                }
                self.state_dirty = true;
            }
            Message::Scrolled(offset) => {
                self.scroll_offset = offset;
            }
            Message::CursorMoved(position) => {
                if self.sidebar_dragging {
                    let max = (self.window_size.width - 360.0).max(240.0);
                    self.sidebar_width = position.x.clamp(160.0, max);
                    self.persisted.sidebar_width = self.sidebar_width;
                    self.state_dirty = true;
                }
            }
            Message::SidebarDragStart => {
                self.sidebar_dragging = true;
            }
            Message::SidebarDragEnd => {
                self.sidebar_dragging = false;
            }
            Message::Composer(value) => {
                self.composer = value;
                self.draft_dirty = true;
                tasks.extend(self.refresh_totoz_suggest());
            }
            Message::ComposerSubmit => {
                if self.totoz_suggest.visible() {
                    tasks.extend(self.pick_totoz_suggestion());
                } else {
                    tasks.extend(self.send_message());
                }
            }
            Message::Insert(text) => {
                tasks.push(self.insert_composer(&text));
            }
            Message::TotozPicked(name) => {
                self.screen = Screen::Main;
                self.hovered_totoz = None;
                self.hovered_link_preview = None;
                self.hovered_preview = None;
                self.scroll_to_bottom = true;
                tasks.push(self.insert_composer(&format!("[:{name}] ")));
                tasks.push(iced::widget::operation::snap_to_end(
                    self.posts_scroll_id.clone(),
                ));
            }
            Message::TotozSuggestSearch(generation) => {
                if generation != self.totoz_suggest.generation {
                    return Task::batch(tasks);
                }
                let terms = self.totoz_suggest.query.clone();
                let still_typed = totoz_suggestion_term(&self.composer) == Some(terms.as_str());
                if terms.is_empty() || !still_typed {
                    return Task::batch(tasks);
                }
                let client = self.client.clone();
                let server = self.config.totoz_server.clone();
                let gb2c = self
                    .config
                    .servers
                    .iter()
                    .find(|server| server.enabled)
                    .map(|server| server.url.clone());
                tasks.push(Task::perform(
                    async move {
                        search_totoz_any(&client, &server, gb2c.as_deref(), &terms).await
                    },
                    move |result| Message::TotozSuggestLoaded(generation, result),
                ));
            }
            Message::TotozSuggestLoaded(generation, result) => {
                if generation == self.totoz_suggest.generation {
                    tasks.extend(self.apply_totoz_suggestions(result));
                }
            }
            Message::TotozSuggestImage(name, result) => {
                if let Ok(bytes) = result {
                    if let Some(image) = decode_totoz_image(&bytes) {
                        self.totoz.images.insert(name, image);
                    }
                }
            }
            Message::TotozSuggestPick(name) => {
                tasks.extend(self.pick_totoz_suggestion_named(&name));
            }
            Message::TotozSuggestDismiss => {
                if self.totoz_suggest.visible() {
                    self.totoz_suggest.dismiss();
                    // The text input also handles Escape by unfocusing itself;
                    // keep the caret in the entry zone so typing can resume.
                    tasks.push(iced::widget::operation::focus(self.composer_id.clone()));
                } else if !self.totoz_suggest.query.is_empty() {
                    // Cancel a pending debounced search without stealing the
                    // Escape key from the text input.
                    self.totoz_suggest.dismiss();
                }
            }
            Message::Send => {
                tasks.extend(self.send_message());
            }
            Message::Posted(description, result) => {
                self.sending = false;
                match result {
                    Ok(()) => {
                        self.status = format!("envoyé à {description}");
                        self.error = None;
                        self.pending_post = None;
                        tasks.push(Task::done(Message::PollBoards));
                    }
                    Err(error) => {
                        self.status = "envoi échoué".to_owned();
                        self.error = Some(error);
                        if self.composer.trim().is_empty() {
                            if let Some(pending) = self.pending_post.take() {
                                self.config.draft = pending.clone();
                                self.composer = pending;
                                self.draft_dirty = true;
                            }
                        }
                    }
                }
            }
            Message::RefreshAll => {
                self.sse_epoch = self.sse_epoch.wrapping_add(1);
                tasks.extend(self.refresh_tasks());
            }
            Message::PollBoards => {
                tasks.extend(self.poll_boards());
            }
            Message::TotozQuery(value) => self.totoz.query = value,
            Message::TotozSearch => {
                let terms = self.totoz.query.trim().to_owned();
                if terms.is_empty() {
                    self.totoz.error = Some("cherche un totoz".into());
                } else {
                    self.totoz.loading = true;
                    self.totoz.error = None;
                    self.totoz.images.clear();
                    // Let the inline autocompletion fetch its thumbnails again
                    // instead of pointing at names removed with the cache.
                    self.totoz_suggest.requested_images.clear();
                    let client = self.client.clone();
                    let server = self.config.totoz_server.clone();
                    let gb2c = self
                        .config
                        .servers
                        .iter()
                        .find(|server| server.enabled)
                        .map(|server| server.url.clone());
                    tasks.push(Task::perform(
                        async move {
                            search_totoz_any(&client, &server, gb2c.as_deref(), &terms).await
                        },
                        Message::TotozLoaded,
                    ));
                }
            }
            Message::TotozLoaded(result) => {
                self.totoz.loading = false;
                match result {
                    Ok(results) => {
                        self.totoz.results = results;
                        self.totoz.error = None;
                        let client = self.client.clone();
                        let requests: Vec<(String, String)> = self
                            .totoz
                            .results
                            .iter()
                            .map(|totoz| (totoz.name.clone(), totoz.image.clone()))
                            .collect();
                        if !requests.is_empty() {
                            tasks.push(Task::perform(
                                async move {
                                    futures::stream::iter(requests)
                                        .map(|(name, url)| {
                                            let client = client.clone();
                                            async move {
                                                let result = async {
                                                    let response = client
                                                        .get(&url)
                                                        .timeout(Duration::from_secs(20))
                                                        .send()
                                                        .await
                                                        .map_err(|error| error.to_string())?;
                                                    if !response.status().is_success() {
                                                        return Err(format!(
                                                            "HTTP {}",
                                                            response.status()
                                                        ));
                                                    }
                                                    let bytes = response
                                                        .bytes()
                                                        .await
                                                        .map_err(|error| error.to_string())?;
                                                    if bytes.len() > 2 * 1024 * 1024 {
                                                        return Err("image trop lourde".to_owned());
                                                    }
                                                    Ok(bytes.to_vec())
                                                }
                                                .await;
                                                (name, result)
                                            }
                                        })
                                        .buffer_unordered(8)
                                        .collect::<Vec<_>>()
                                        .await
                                },
                                Message::TotozImages,
                            ));
                        }
                    }
                    Err(error) => {
                        self.totoz.results.clear();
                        self.totoz.error = Some(error);
                    }
                }
            }
            Message::TotozImages(images) => {
                for (name, bytes) in images {
                    let Ok(bytes) = bytes else {
                        continue;
                    };
                    if let Some(image) = decode_totoz_image(&bytes) {
                        self.totoz.images.insert(name, image);
                    }
                }
            }
            Message::EmojiQuery(value) => self.emoji.query = value,
            Message::EmojiSearch => {
                let terms = self.emoji.query.trim().to_owned();
                let client = self.client.clone();
                let server = self
                    .config
                    .servers
                    .iter()
                    .find(|server| server.enabled)
                    .map(|server| server.url.clone());
                tasks.push(Task::perform(
                    async move {
                        let fallback = || {
                            let terms = terms.to_lowercase();
                            net::builtin_emoji()
                                .into_iter()
                                .filter(|item| {
                                    terms.is_empty()
                                        || item.name.to_lowercase().contains(&terms)
                                        || item.characters.contains(&terms)
                                })
                                .take(120)
                                .collect::<Vec<_>>()
                        };
                        match server {
                            Some(server) => net::search_emoji(&client, &server, &terms)
                                .await
                                .unwrap_or_else(|_| fallback()),
                            None => fallback(),
                        }
                    },
                    Message::EmojiLoaded,
                ));
            }
            Message::EmojiLoaded(results) => {
                self.emoji.results = results;
            }
            Message::Settings(settings) => {
                tasks.extend(self.update_settings(settings));
            }
            Message::OpenSettings => {
                self.totoz_suggest.clear();
                self.settings = SettingsDraft::from_config(&self.config);
                self.screen = Screen::Settings;
            }
            Message::OpenTotoz => {
                self.totoz_suggest.clear();
                self.screen = Screen::Totoz;
                tasks.push(focus_text_input(&self.totoz_search_id));
                if self.totoz.results.is_empty() && !self.totoz.query.trim().is_empty() {
                    tasks.push(Task::done(Message::TotozSearch));
                }
            }
            Message::OpenEmoji => {
                self.totoz_suggest.clear();
                self.screen = Screen::Emoji;
                tasks.push(focus_text_input(&self.emoji_search_id));
                if self.emoji.results.is_empty() {
                    tasks.push(Task::done(Message::EmojiSearch));
                }
            }
            Message::CloseScreen => {
                self.screen = Screen::Main;
                self.settings.error = None;
                self.settings.info = None;
                self.hovered_post = None;
                self.hovered_link_post = None;
                self.hovered_info = None;
                self.hovered_totoz = None;
                self.hovered_link_preview = None;
                self.hovered_preview = None;
                self.scroll_to_bottom = true;
                tasks.push(focus_text_input(&self.composer_id));
                tasks.push(iced::widget::operation::snap_to_end(
                    self.posts_scroll_id.clone(),
                ));
            }
            Message::Spoiler(key, spoiler) => {
                if !self.reveal.insert((key.clone(), spoiler)) {
                    self.reveal.remove(&(key, spoiler));
                }
            }
            Message::LinkClicked(action) => {
                tasks.extend(self.handle_link(action));
            }
            Message::OpenUrl(url) => {
                if !url.trim().is_empty() {
                    tasks.push(Task::perform(
                        async move {
                            if let Err(error) = open::that(&url) {
                                return Err(error.to_string());
                            }
                            Ok(())
                        },
                        |result| match result {
                            Ok(()) => Message::Noop,
                            Err(error) => Message::Error(error),
                        },
                    ));
                }
            }
            Message::HoverPost(key) => {
                if key.is_some() {
                    self.hovered_info = None;
                    self.hovered_totoz = None;
                    self.hovered_link_preview = None;
                }
                self.hovered_post = key;
            }
            Message::HoverExit => {
                self.hovered_post = None;
                self.hovered_link_post = None;
                self.hovered_info = None;
                self.hovered_totoz = None;
                self.hovered_link_preview = None;
                self.schedule_preview_hide(&mut tasks);
            }
            Message::PostShown(key) => {
                self.visible_posts.insert(key);
            }
            Message::PostHidden(key) => {
                self.visible_posts.remove(&key);
            }
            Message::HoverLink(link, anchor) => {
                self.hovered_info = None;
                self.hovered_link_post = None;
                self.hovered_link_preview = None;
                if link.is_some() {
                    self.hovered_post = None;
                }
                match link {
                    Some(LinkAction::Url(url)) => {
                        self.hovered_preview = None;
                        self.hovered_totoz = None;
                        // HoverRich reports the text anchor in the scrollable
                        // content space; convert it to window coordinates so
                        // the popup stays anchored above the link text.
                        self.hovered_link_preview = Some(HoveredLinkPreview {
                            url,
                            anchor: Point::new(anchor.x, anchor.y - self.scroll_offset),
                        });
                    }
                    Some(LinkAction::Jump(key)) => {
                        if !self.visible_posts.contains(&key) {
                            self.hovered_preview = Some(key.clone());
                            self.preview_hovered = false;
                            // Invalidate any pending hide from the previous
                            // preview, so a new target is never closed by it.
                            self.preview_epoch = self.preview_epoch.wrapping_add(1);
                        } else {
                            self.hovered_preview = None;
                        }
                        self.hovered_link_post = Some(key);
                        self.hovered_totoz = None;
                    }
                    Some(LinkAction::Info(info, _)) => {
                        self.hovered_info = Some(info);
                        self.hovered_preview = None;
                        self.hovered_totoz = None;
                    }
                    Some(LinkAction::Totoz(name)) => {
                        self.hovered_preview = None;
                        tasks.extend(self.hover_totoz(name, anchor));
                    }
                    Some(_) => {
                        self.hovered_preview = None;
                        self.hovered_totoz = None;
                    }
                    None => {
                        self.hovered_totoz = None;
                        // The pointer left the norloge. Keep the preview
                        // briefly so the user can move the cursor onto it.
                        self.schedule_preview_hide(&mut tasks);
                    }
                }
            }
            Message::PreviewEnter => {
                self.preview_hovered = true;
                self.preview_epoch = self.preview_epoch.wrapping_add(1);
            }
            Message::PreviewExit => {
                self.preview_hovered = false;
                self.schedule_preview_hide(&mut tasks);
            }
            Message::HidePreview(epoch) => {
                if epoch == self.preview_epoch && !self.preview_hovered {
                    self.hovered_preview = None;
                }
            }
            Message::TotozFrameTick => {
                if let Some(totoz) = self.hovered_totoz.as_mut() {
                    if let Some(image) = &totoz.image {
                        if image.frames.len() > 1 {
                            let now = Instant::now();
                            let delay = image
                                .delays
                                .get(totoz.frame_index)
                                .copied()
                                .unwrap_or(Duration::from_millis(80));
                            if now.duration_since(totoz.frame_started) >= delay {
                                totoz.frame_index = (totoz.frame_index + 1) % image.frames.len();
                                totoz.frame_started = now;
                            }
                        }
                    }
                }
            }
            Message::TotozHoverImage(name, result) => {
                let current = self
                    .hovered_totoz
                    .as_ref()
                    .is_some_and(|totoz| totoz.name == name);
                match result {
                    Ok(bytes) => match decode_totoz_image(&bytes) {
                        Some(image) => {
                            self.totoz.images.insert(name.clone(), image.clone());
                            if current {
                                if let Some(totoz) = self.hovered_totoz.as_mut() {
                                    totoz.image = Some(image);
                                    totoz.frame_index = 0;
                                    totoz.frame_started = Instant::now();
                                    totoz.loading = false;
                                    totoz.error = None;
                                }
                            }
                        }
                        None => {
                            if current {
                                if let Some(totoz) = self.hovered_totoz.as_mut() {
                                    totoz.loading = false;
                                    totoz.error = Some("image invalide".into());
                                }
                            }
                        }
                    },
                    Err(error) => {
                        if current {
                            if let Some(totoz) = self.hovered_totoz.as_mut() {
                                totoz.loading = false;
                                totoz.error = Some(error);
                            }
                        }
                    }
                }
            }
            Message::LinkFetched(url, result) => {
                let preview = match result {
                    Ok(net::LinkMetadata::Title(title)) => LinkPreview::Title(title),
                    Ok(net::LinkMetadata::Image(bytes)) => match decode_link_image(&bytes) {
                        Some((handle, width, height)) => LinkPreview::Image {
                            handle,
                            width,
                            height,
                        },
                        None => LinkPreview::Error,
                    },
                    Err(_) => LinkPreview::Error,
                };
                self.cache_link_preview(url, preview);
            }
            Message::RefreshLinuxfr => {
                if !self.linuxfr_refreshing
                    && self
                        .linuxfr_retry_at
                        .is_none_or(|retry| Instant::now() >= retry)
                {
                    let refresh = self
                        .config
                        .linuxfr
                        .as_ref()
                        .map(|session| session.refresh_token.clone())
                        .filter(|token| !token.is_empty());
                    let server = self
                        .config
                        .servers
                        .iter()
                        .find(|server| server.enabled)
                        .map(|server| server.url.clone());
                    match (refresh, server) {
                        (Some(refresh), Some(server)) => {
                            self.linuxfr_refreshing = true;
                            self.settings.info = Some("rafraîchissement LinuxFR…".into());
                            let client = self.client.clone();
                            tasks.push(Task::perform(
                                async move {
                                    net::refresh_linuxfr(&client, &server, &refresh).await
                                },
                                Message::LinuxfrRefreshed,
                            ));
                        }
                        _ => {
                            self.settings.error =
                                Some("aucun refresh token LinuxFR ou aucun serveur gb2c".into());
                        }
                    }
                }
            }
            Message::LinuxfrRefreshed(result) => {
                self.linuxfr_refreshing = false;
                match result {
                    Ok(mut session) => {
                        if session.login.is_empty() {
                            if let Some(previous) = self.config.linuxfr.as_ref() {
                                session.login = previous.login.clone();
                            }
                        }
                        self.config.linuxfr = Some(session);
                        self.linuxfr_retry_at = None;
                        self.settings.info = Some("jeton LinuxFR rafraîchi".into());
                        self.save_config_quietly();
                    }
                    Err(error) => {
                        self.linuxfr_retry_at = Some(Instant::now() + Duration::from_secs(300));
                        self.settings.error = Some(error.clone());
                        self.error = Some(error);
                    }
                }
            }
            Message::Info(info) => {
                self.status = info.clone();
                self.settings.info = Some(info);
            }
            Message::Error(error) => {
                self.error = Some(error);
                self.status = "erreur".to_owned();
            }
            Message::Noop => {}
        }

        self.ensure_analysis();
        // Dispatch notifications before advancing the read marker: otherwise a
        // post arriving in the active tribune could be marked as read before
        // its bigorno/notification check, which silently suppressed it when
        // the "only when unfocused" option was disabled.
        tasks.extend(self.dispatch_notifications());
        if self.focused {
            self.mark_active_read();
        }
        Task::batch(tasks)
    }

    // ---------------------------------------------------------------------
    // Helpers used by the update loop and the view

    fn save_config_quietly(&mut self) {
        if let Err(error) = config::save_config(&self.config) {
            self.error = Some(error);
        }
    }

    pub fn tribunes(&self) -> Vec<TribuneKey> {
        let mut keys = BTreeSet::new();
        for server in self.config.servers.iter().filter(|server| server.enabled) {
            let url = config::normalize_url(&server.url).unwrap_or_else(|| server.url.clone());
            if let Some(state) = self.servers.get(&url) {
                for name in &state.known {
                    let key = TribuneKey::new(SourceId::Gb2c(url.clone()), name.clone());
                    if !self.config.hidden_tribunes.contains(&key.key()) {
                        keys.insert(key);
                    }
                }
            }
        }
        for board in self.config.all_boards() {
            if self.config.selected_boards.contains(&board.name) {
                let key = TribuneKey::new(SourceId::Direct(board.name.clone()), board.name.clone());
                if !self.config.hidden_tribunes.contains(&key.key()) {
                    keys.insert(key);
                }
            }
        }
        keys.into_iter().collect()
    }

    pub fn active_tribune(&self) -> Option<&TribuneKey> {
        self.active.as_ref()
    }

    pub fn is_active(&self, key: &TribuneKey) -> bool {
        self.active.as_ref() == Some(key)
    }

    pub fn source_label(&self, source: &SourceId) -> String {
        match source {
            SourceId::Gb2c(url) => self
                .servers
                .get(url)
                .map(|state| state.name.clone())
                .unwrap_or_else(|| url.clone()),
            SourceId::Direct(name) => name.clone(),
        }
    }

    pub fn server_status(&self, url: &str) -> ConnectionStatus {
        let url = config::normalize_url(url).unwrap_or_else(|| url.to_owned());
        let posting = self.sending
            && self.active.as_ref().is_some_and(
                |active| matches!(&active.source, SourceId::Gb2c(active_url) if active_url == &url),
            );
        let Some(state) = self.servers.get(&url) else {
            return ConnectionStatus::Working;
        };
        if posting || state.fetching {
            ConnectionStatus::Working
        } else if state.last_error.is_some() {
            ConnectionStatus::Error
        } else if state.connected {
            ConnectionStatus::Online
        } else {
            ConnectionStatus::Working
        }
    }

    pub fn board_status(&self, name: &str) -> ConnectionStatus {
        let posting = self.sending
            && self
                .active
                .as_ref()
                .is_some_and(|active| matches!(&active.source, SourceId::Direct(active_name) if active_name == name));
        let Some(state) = self.boards.get(name) else {
            return ConnectionStatus::Working;
        };
        if posting || state.fetching {
            ConnectionStatus::Working
        } else if state.last_error.is_some() {
            ConnectionStatus::Error
        } else if state.connected {
            ConnectionStatus::Online
        } else {
            ConnectionStatus::Working
        }
    }

    pub fn visible_keys(&self) -> Vec<PostKey> {
        let Some(active) = &self.active else {
            return Vec::new();
        };
        self.analysis
            .posts_for(active)
            .iter()
            .filter(|key| {
                let hidden = self
                    .analysis
                    .info(key)
                    .map(|info| info.hidden)
                    .unwrap_or(false);
                !hidden || self.config.show_hidden || self.config.blocked_mode == BlockedMode::Gray
            })
            .cloned()
            .collect()
    }

    pub fn parsed_display(&self, key: &PostKey) -> Option<&ParsedMessage> {
        let post = self.posts.get(key)?;
        if self
            .analysis
            .info(key)
            .map(|info| info.hidden)
            .unwrap_or(false)
        {
            return None;
        }
        if self.config.plop_list.contains(post.info.trim()) {
            return self.plop_cache.get(key).or(Some(&self.plop_parsed));
        }
        self.analysis.parsed(key)
    }

    pub fn is_revealed(&self, key: &PostKey, spoiler: usize) -> bool {
        self.reveal.contains(&(key.clone(), spoiler))
    }

    pub fn badges(&self, tribune: &TribuneKey) -> (usize, bool, bool) {
        let last_read = self.last_read(tribune);
        let mut unread = 0usize;
        let mut bigorno = false;
        let mut reply = false;
        for key in self.analysis.posts_for(tribune) {
            if key.id <= last_read {
                continue;
            }
            let Some(info) = self.analysis.info(key) else {
                continue;
            };
            if info.is_mine {
                continue;
            }
            if info.hidden
                && self.config.blocked_mode == BlockedMode::Hide
                && !self.config.show_hidden
            {
                continue;
            }
            unread += 1;
            bigorno |= info.is_bigorno;
            reply |= info.is_reply;
        }
        (unread, bigorno, reply)
    }

    pub fn total_badges(&self) -> usize {
        self.tribunes()
            .iter()
            .map(|tribune| self.badges(tribune).0)
            .sum()
    }

    pub fn last_read(&self, tribune: &TribuneKey) -> i64 {
        self.persisted
            .last_read
            .get(&tribune.key())
            .copied()
            .unwrap_or(0)
    }

    fn mark_read(&mut self, tribune: &TribuneKey) {
        let max = self.analysis.max_id_for(tribune);
        if max > 0 && self.last_read(tribune) < max {
            self.persisted.last_read.insert(tribune.key(), max);
            self.state_dirty = true;
        }
    }

    fn mark_active_read(&mut self) {
        if let Some(active) = self.active.clone() {
            self.mark_read(&active);
        }
    }

    fn ensure_analysis(&mut self) {
        if self.analysis_dirty {
            self.prune_posts();
            self.baseline_history();
            let blocked: BTreeSet<String> = self
                .config
                .blocked
                .iter()
                .cloned()
                .chain(
                    self.servers
                        .values()
                        .flat_map(|state| state.blocked.iter().cloned()),
                )
                .collect();
            let identities = self.config.my_identities();
            self.analysis =
                conversation::analyze(&self.posts, &blocked, &identities, self.now.naive_local());
            self.plop_cache.clear();
            for (key, post) in &self.posts {
                if self.config.plop_list.contains(post.info.trim()) {
                    self.plop_cache
                        .insert(key.clone(), message::parse(conversation::plop_for(key)));
                }
            }
            self.analysis_dirty = false;
            let tribunes = self.tribunes();
            if self
                .active
                .as_ref()
                .is_none_or(|active| !tribunes.contains(active))
            {
                self.active = tribunes.first().cloned();
            }
            if self
                .selected
                .as_ref()
                .is_some_and(|selected| !self.posts.contains_key(selected))
            {
                self.selected = None;
            }
        }
    }

    fn prune_posts(&mut self) {
        let limit = self.config.history_limit.clamp(100, 10_000);
        let mut by_tribune: BTreeMap<TribuneKey, Vec<PostKey>> = BTreeMap::new();
        for key in self.posts.keys() {
            by_tribune
                .entry(key.tribune.clone())
                .or_default()
                .push(key.clone());
        }
        let mut remove = Vec::new();
        for keys in by_tribune.values_mut() {
            keys.sort_by_key(|key| key.id);
            if keys.len() > limit {
                remove.extend(keys[..keys.len() - limit].iter().cloned());
            }
        }
        for key in remove {
            self.posts.remove(&key);
            self.analysis.posts.remove(&key);
        }
    }

    fn sync_sources(&mut self) {
        let mut configured = BTreeSet::new();
        for server in &self.config.servers {
            let url = config::normalize_url(&server.url).unwrap_or_else(|| server.url.clone());
            configured.insert(url.clone());
            let known = self
                .persisted
                .known_tribunes
                .get(&url)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect();
            let state = self
                .servers
                .entry(url.clone())
                .or_insert_with(|| ServerState {
                    name: server.name.clone(),
                    url: url.clone(),
                    known,
                    blocked: BTreeSet::new(),
                    connected: false,
                    fetching: false,
                    last_error: None,
                    last_event: None,
                });
            state.name = server.name.clone();
            state.url = url;
        }
        self.servers.retain(|url, _| configured.contains(url));

        let selected: BTreeSet<String> = self.config.selected_boards.clone();
        for name in &selected {
            self.boards.entry(name.clone()).or_default();
        }
        self.boards.retain(|name, _| selected.contains(name));
    }

    fn retain_current_sources(&mut self) {
        let servers: BTreeSet<String> = self
            .config
            .servers
            .iter()
            .map(|server| config::normalize_url(&server.url).unwrap_or_else(|| server.url.clone()))
            .collect();
        let boards = self.config.selected_boards.clone();
        self.posts.retain(|key, _| match &key.tribune.source {
            SourceId::Gb2c(url) => servers.contains(url),
            SourceId::Direct(name) => boards.contains(name),
        });
        self.analysis_dirty = true;
    }

    fn ingest(
        &mut self,
        source: SourceId,
        posts: Vec<Post>,
        label: Option<&str>,
    ) -> Vec<Task<Message>> {
        let mut new_links = BTreeSet::new();
        for mut post in posts {
            if post.id <= 0 || post.tribune.trim().is_empty() {
                continue;
            }
            post.source_label = label.unwrap_or_default().to_owned();
            let key = post.key(&source);
            let is_new = !self.posts.contains_key(&key);
            if is_new {
                self.pending_notifications.push(key.clone());
                new_links.extend(post_links(&post.message));
            }
            self.posts.insert(key, post);
        }
        self.analysis_dirty = true;

        let mut tasks = Vec::new();
        for url in new_links {
            if self.link_previews.contains_key(&url) {
                continue;
            }
            self.link_previews.insert(url.clone(), LinkPreview::Loading);
            let client = self.client.clone();
            let request_url = url.clone();
            tasks.push(Task::perform(
                async move { net::fetch_link_metadata(&client, &request_url).await },
                move |result| Message::LinkFetched(url.clone(), result),
            ));
        }
        tasks
    }

    /// Treat the backlog of a newly discovered tribune as read. SSE delivers
    /// stored posts one event at a time, so the baseline has to be advanced
    /// incrementally. Tribunes that already had a last-read marker when the
    /// application started keep their unread posts across restarts.
    fn baseline_history(&mut self) {
        let mut max_old: BTreeMap<TribuneKey, i64> = BTreeMap::new();
        for (key, post) in &self.posts {
            if self.pre_existing_tribunes.contains(&key.tribune) {
                continue;
            }
            let old = post.time().age_seconds().is_none_or(|age| age > 180);
            if old {
                max_old
                    .entry(key.tribune.clone())
                    .and_modify(|max| *max = (*max).max(key.id))
                    .or_insert(key.id);
            }
        }
        for (tribune, max_id) in max_old {
            if self.last_read(&tribune) < max_id {
                self.persisted.last_read.insert(tribune.key(), max_id);
                self.state_dirty = true;
            }
        }
    }

    fn refresh_tasks(&mut self) -> Vec<Task<Message>> {
        let mut tasks = Vec::new();
        for server in self.config.servers.iter().filter(|server| server.enabled) {
            let url = config::normalize_url(&server.url).unwrap_or_else(|| server.url.clone());
            if let Some(state) = self.servers.get_mut(&url) {
                state.fetching = true;
            }
            let client = self.client.clone();
            let request_url = url.clone();
            tasks.push(Task::perform(
                async move { net::fetch_server_info(&client, &request_url).await },
                move |result| Message::ServerInfo(url.clone(), result),
            ));
        }
        for board in self.config.all_boards() {
            if self.config.selected_boards.contains(&board.name) {
                tasks.push(self.board_task(board));
            }
        }
        tasks
    }

    fn board_task(&mut self, board: Board) -> Task<Message> {
        if let Some(state) = self.boards.get_mut(&board.name) {
            if state.fetching {
                return Task::none();
            }
            state.fetching = true;
        }
        let client = self.client.clone();
        let name = board.name.clone();
        Task::perform(
            async move { net::fetch_board(&client, &board).await },
            move |result| Message::BoardPosts(name.clone(), result),
        )
    }

    fn poll_boards(&mut self) -> Vec<Task<Message>> {
        let boards: Vec<Board> = self
            .config
            .all_boards()
            .into_iter()
            .filter(|board| self.config.selected_boards.contains(&board.name))
            .collect();
        boards
            .into_iter()
            .map(|board| self.board_task(board))
            .collect()
    }

    fn send_message(&mut self) -> Vec<Task<Message>> {
        let Some(active) = self.active.clone() else {
            self.error = Some("aucune tribune active".into());
            return Vec::new();
        };
        let text = self.composer.trim().to_owned();
        if text.is_empty() {
            return Vec::new();
        }
        if text.chars().count() > config::MAX_MESSAGE_LENGTH {
            self.error = Some("message trop long".into());
            return Vec::new();
        }
        if self.sending {
            return Vec::new();
        }
        self.sending = true;
        self.status = "envoi…".to_owned();
        self.error = None;
        self.config.draft.clear();
        self.composer.clear();
        self.totoz_suggest.clear();
        self.config.last_posted = text.clone();
        self.pending_post = Some(text.clone());
        self.save_config_quietly();
        self.draft_dirty = false;

        let client = self.client.clone();
        let nickname = self.config.nickname.clone();
        let token = if active.name == "dlfp" {
            self.config
                .linuxfr
                .as_ref()
                .map(|session| session.access_token.clone())
        } else {
            None
        };
        let description = format!("{} ({})", active.name, active.source.kind());
        let task = match &active.source {
            SourceId::Gb2c(url) => {
                let url = url.clone();
                let tribune = active.name.clone();
                Task::perform(
                    async move {
                        net::post_gb2c(&client, &url, &tribune, &nickname, &text, token.as_deref())
                            .await
                    },
                    move |result| Message::Posted(description.clone(), result),
                )
            }
            SourceId::Direct(name) => match self.config.board(name) {
                Some(board) => Task::perform(
                    async move {
                        net::post_board(&client, &board, &nickname, &text, token.as_deref()).await
                    },
                    move |result| Message::Posted(description.clone(), result),
                ),
                None => {
                    self.sending = false;
                    self.error = Some(format!("board {name} introuvable"));
                    return Vec::new();
                }
            },
        };
        vec![task]
    }

    /// Append text to the composer and place the caret right after it, so the
    /// user can keep typing without reaching for the entry field.
    fn insert_composer(&mut self, text: &str) -> Task<Message> {
        self.composer.push_str(text);
        self.draft_dirty = true;
        self.totoz_suggest.clear();
        focus_text_input(&self.composer_id)
    }

    /// Detect the unfinished `[:…` token at the end of the composer and start
    /// a (debounced) totoz search when it changed. Anything shorter than
    /// [`TOTOZ_SUGGEST_MIN_CHARS`] characters closes the suggestion list.
    fn refresh_totoz_suggest(&mut self) -> Vec<Task<Message>> {
        let query = totoz_suggestion_term(&self.composer)
            .unwrap_or_default()
            .to_owned();
        if query == self.totoz_suggest.query {
            return Vec::new();
        }
        self.totoz_suggest.generation = self.totoz_suggest.generation.wrapping_add(1);
        self.totoz_suggest.query = query.clone();
        if query.is_empty() {
            self.totoz_suggest.results.clear();
            self.totoz_suggest.selected = None;
            return Vec::new();
        }
        // Filter the previous list right away so it keeps matching while the
        // debounced request runs, instead of blinking out on every keypress.
        let lower = query.to_lowercase();
        self.totoz_suggest
            .results
            .retain(|totoz| totoz.name.to_lowercase().contains(&lower));
        self.totoz_suggest.selected = (!self.totoz_suggest.results.is_empty()).then_some(0);
        let generation = self.totoz_suggest.generation;
        vec![Task::perform(
            async move {
                tokio::time::sleep(TOTOZ_SUGGEST_DELAY).await;
            },
            move |_| Message::TotozSuggestSearch(generation),
        )]
    }

    fn apply_totoz_suggestions(
        &mut self,
        result: Result<Vec<net::Totoz>, String>,
    ) -> Vec<Task<Message>> {
        let matches = match result {
            Ok(results) => {
                rank_totoz_suggestions(results, &self.totoz_suggest.query, TOTOZ_SUGGEST_LIMIT)
            }
            // Unknown prefix or unreachable server: display nothing at all.
            Err(_) => Vec::new(),
        };
        self.totoz_suggest.selected = (!matches.is_empty()).then_some(0);
        self.totoz_suggest.results = matches;
        self.fetch_totoz_suggest_images()
    }

    /// Fetch the small previews shown next to each suggestion. Images are
    /// cached in [`TotozState::images`] and shared with the totoz picker and
    /// the hover previews, so each name is only downloaded once.
    fn fetch_totoz_suggest_images(&mut self) -> Vec<Task<Message>> {
        let names: Vec<String> = self
            .totoz_suggest
            .results
            .iter()
            .map(|totoz| totoz.name.clone())
            .collect();
        let client = self.client.clone();
        let server = self.config.totoz_server.clone();
        let mut tasks = Vec::new();
        for name in names {
            if self.totoz.images.contains_key(&name)
                || self.totoz_suggest.requested_images.contains(&name)
            {
                continue;
            }
            self.totoz_suggest.requested_images.insert(name.clone());
            let client = client.clone();
            let server = server.clone();
            tasks.push(Task::perform(
                async move {
                    let result = net::fetch_totoz_image(&client, &server, &name).await;
                    (name, result)
                },
                |(name, result)| Message::TotozSuggestImage(name, result),
            ));
        }
        tasks
    }

    /// Move the highlighted suggestion, wrapping around at both ends.
    fn move_totoz_suggestion(&mut self, delta: isize) {
        let count = self.totoz_suggest.results.len();
        if count == 0 {
            return;
        }
        let current = self.totoz_suggest.selected.unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(count as isize) as usize;
        self.totoz_suggest.selected = Some(next);
    }

    fn pick_totoz_suggestion(&mut self) -> Vec<Task<Message>> {
        let Some(totoz) = self
            .totoz_suggest
            .selected
            .and_then(|index| self.totoz_suggest.results.get(index))
        else {
            return Vec::new();
        };
        let name = totoz.name.clone();
        self.pick_totoz_suggestion_named(&name)
    }

    /// Replace the in-progress `[:…` token with the complete totoz reference.
    fn pick_totoz_suggestion_named(&mut self, name: &str) -> Vec<Task<Message>> {
        let Some(start) = self.composer.rfind("[:") else {
            return Vec::new();
        };
        if totoz_suggestion_term(&self.composer).is_none() {
            return Vec::new();
        }
        self.composer.truncate(start);
        self.composer.push_str(&format!("[:{name}] "));
        self.draft_dirty = true;
        self.totoz_suggest.clear();
        vec![focus_text_input(&self.composer_id)]
    }

    fn handle_link(&mut self, action: LinkAction) -> Vec<Task<Message>> {
        match action {
            LinkAction::Url(url) => {
                let url = url.clone();
                vec![Task::perform(
                    async move {
                        let _ = open::that(url);
                    },
                    |_| Message::Noop,
                )]
            }
            LinkAction::Jump(key) => {
                vec![Task::done(Message::JumpTo(key))]
            }
            LinkAction::Spoiler(key, spoiler) => {
                vec![Task::done(Message::Spoiler(key, spoiler))]
            }
            LinkAction::Totoz(name) => {
                self.totoz.query = name;
                self.screen = Screen::Totoz;
                vec![
                    focus_text_input(&self.totoz_search_id),
                    Task::done(Message::TotozSearch),
                ]
            }
            LinkAction::Info(_, key) => {
                let Some(post) = self.posts.get(&key) else {
                    return Vec::new();
                };
                let identity = post.identity();
                if identity.is_empty() {
                    return Vec::new();
                }
                let text = format!("{identity}<");
                vec![self.insert_composer(&text)]
            }
            LinkAction::Unresolved(raw) => {
                self.status = format!("norloge introuvable : {raw}");
                Vec::new()
            }
        }
    }

    fn handle_keyboard(&mut self, event: keyboard::Event) -> Vec<Task<Message>> {
        let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
            return Vec::new();
        };
        let control = modifiers.control();
        let alt = modifiers.alt();
        let shift = modifiers.shift();

        if self.screen == Screen::Main && self.totoz_suggest.visible() {
            match &key {
                keyboard::Key::Named(keyboard::key::Named::ArrowDown) => {
                    self.move_totoz_suggestion(1);
                    return Vec::new();
                }
                keyboard::Key::Named(keyboard::key::Named::ArrowUp) => {
                    self.move_totoz_suggestion(-1);
                    return Vec::new();
                }
                // Tab accepts the highlighted totoz; Ctrl+Tab keeps switching
                // tribunes below. The composer keeps focus either way.
                keyboard::Key::Named(keyboard::key::Named::Tab) if !control => {
                    return self.pick_totoz_suggestion();
                }
                _ => {}
            }
        }

        if (control || alt) && key == keyboard::Key::Named(keyboard::key::Named::Enter) {
            return vec![Task::done(Message::Send)];
        }
        if key == keyboard::Key::Named(keyboard::key::Named::Escape) {
            return vec![Task::done(if self.screen == Screen::Main {
                Message::ClearSelection
            } else {
                Message::CloseScreen
            })];
        }
        if key == keyboard::Key::Named(keyboard::key::Named::F5) {
            return vec![Task::done(Message::RefreshAll)];
        }
        if control {
            match key {
                keyboard::Key::Named(keyboard::key::Named::Tab) => {
                    let tribunes = self.tribunes();
                    if !tribunes.is_empty() {
                        let current = self
                            .active
                            .as_ref()
                            .and_then(|active| tribunes.iter().position(|key| key == active))
                            .unwrap_or(0);
                        let next = if shift {
                            (current + tribunes.len() - 1) % tribunes.len()
                        } else {
                            (current + 1) % tribunes.len()
                        };
                        return vec![Task::done(Message::SelectTribune(tribunes[next].clone()))];
                    }
                }
                keyboard::Key::Character(ref character) => match character.as_str() {
                    "r" | "R" => return vec![Task::done(Message::RefreshAll)],
                    "," => return vec![Task::done(Message::OpenSettings)],
                    "m" | "M" => return vec![Task::done(Message::MarkAllRead)],
                    "t" | "T" => return vec![Task::done(Message::OpenTotoz)],
                    "e" | "E" => return vec![Task::done(Message::OpenEmoji)],
                    number @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9") => {
                        let index = number.parse::<usize>().unwrap_or(1) - 1;
                        let tribunes = self.tribunes();
                        if let Some(tribune) = tribunes.get(index) {
                            return vec![Task::done(Message::SelectTribune(tribune.clone()))];
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        if alt {
            if let keyboard::Key::Character(character) = key {
                let (prefix, suffix) = match character.to_lowercase().as_str() {
                    "b" => ("<b>", "</b>"),
                    "i" => ("<i>", "</i>"),
                    "u" => ("<u>", "</u>"),
                    "s" => ("<s>", "</s>"),
                    "t" => ("<tt>", "</tt>"),
                    "c" => ("<code>", "</code>"),
                    "p" => ("<spoiler>", "</spoiler>"),
                    "m" => ("====> <b>Moment</b> <====", ""),
                    "a" => ("_o/* <b>BLAM</b>! ", ""),
                    _ => return Vec::new(),
                };
                return vec![Task::done(Message::Insert(format!("{prefix}{suffix}")))];
            }
        }
        Vec::new()
    }

    fn update_settings(&mut self, message: SettingsMessage) -> Vec<Task<Message>> {
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

    fn apply_settings(&mut self) -> Vec<Task<Message>> {
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

    /// Give the pointer time to leave a norloge and reach the preview panel
    /// before hiding it. `preview_hovered` cancels the hide when the pointer
    /// actually enters the panel.
    fn schedule_preview_hide(&mut self, tasks: &mut Vec<Task<Message>>) {
        if self.hovered_preview.is_none() || self.preview_hovered {
            return;
        }
        self.preview_epoch = self.preview_epoch.wrapping_add(1);
        let epoch = self.preview_epoch;
        tasks.push(Task::perform(
            async {
                tokio::time::sleep(Duration::from_millis(700)).await;
            },
            move |_| Message::HidePreview(epoch),
        ));
    }

    /// Keep only the most recent title and image previews. Errors and pending
    /// fetches are cheap and stay cached so hovering a failed link keeps
    /// showing its red cross.
    fn cache_link_preview(&mut self, url: String, preview: LinkPreview) {
        let is_image = match &preview {
            LinkPreview::Image { .. } => Some(true),
            LinkPreview::Title(_) => Some(false),
            LinkPreview::Loading | LinkPreview::Error => None,
        };
        if let Some(is_image) = is_image {
            let order = if is_image {
                &mut self.link_image_order
            } else {
                &mut self.link_title_order
            };
            order.retain(|item| item != &url);
            order.push_back(url.clone());
            while order.len() > LINK_PREVIEW_CACHE_LIMIT {
                if let Some(oldest) = order.pop_front() {
                    self.link_previews.remove(&oldest);
                }
            }
        }
        self.link_previews.insert(url, preview);
    }

    fn hover_totoz(&mut self, name: String, anchor: Point) -> Vec<Task<Message>> {
        // The widget reports the text anchor in the scrollable content
        // coordinate space; convert it back to window coordinates so the
        // popup is anchored above the totoz and stays still while scrolling.
        let anchor = Point::new(anchor.x, anchor.y - self.scroll_offset);
        if let Some(image) = self.totoz.images.get(&name).cloned() {
            self.hovered_totoz = Some(HoveredTotoz {
                name,
                image: Some(image),
                anchor,
                frame_index: 0,
                frame_started: Instant::now(),
                loading: false,
                error: None,
            });
            return Vec::new();
        }
        self.hovered_totoz = Some(HoveredTotoz {
            name: name.clone(),
            image: None,
            anchor,
            frame_index: 0,
            frame_started: Instant::now(),
            loading: true,
            error: None,
        });
        let client = self.client.clone();
        let server = self.config.totoz_server.clone();
        vec![Task::perform(
            async move {
                let result = net::fetch_totoz_image(&client, &server, &name).await;
                (name, result)
            },
            |(name, result)| Message::TotozHoverImage(name, result),
        )]
    }

    fn dispatch_notifications(&mut self) -> Vec<Task<Message>> {
        let pending = std::mem::take(&mut self.pending_notifications);
        if pending.is_empty() {
            return Vec::new();
        }
        let notifications = self.config.notifications.clone();
        let mut tasks = Vec::new();
        for key in pending {
            if self.seen.contains(&key) {
                continue;
            }
            self.seen.insert(key.clone());
            let Some(post) = self.posts.get(&key) else {
                continue;
            };
            let Some(info) = self.analysis.info(&key) else {
                continue;
            };
            if info.is_mine || info.hidden {
                continue;
            }
            let is_bigorno = info.is_bigorno;
            let is_reply = info.is_reply;
            if !notification_allowed(&notifications, self.focused, is_bigorno, is_reply) {
                continue;
            }
            if key.id <= self.last_read(&key.tribune) {
                continue;
            }
            // Do not wake the user for history replayed after a reconnect.
            if post.time().age_seconds().is_some_and(|age| age > 180) {
                continue;
            }
            let identity = post.display_identity();
            let summary = if is_bigorno {
                format!("📢 bigorno de {identity} sur {}", key.tribune.name)
            } else if is_reply {
                format!("↩ {identity} te répond sur {}", key.tribune.name)
            } else {
                format!("{} · {}", key.tribune.name, identity)
            };
            let body = self
                .parsed_display(&key)
                .map(|parsed| parsed.plain_text())
                .unwrap_or_else(|| post.message.clone());
            let critical = is_bigorno || is_reply;
            tasks.push(Task::perform(
                async move { notify::send(&summary, &body, critical) },
                |_| Message::Noop,
            ));
            if critical && !self.focused {
                tasks.push(window::latest().and_then(move |id| {
                    window::request_user_attention(id, Some(window::UserAttention::Informational))
                }));
            }
        }
        tasks
    }
}

fn post_links(message: &str) -> impl Iterator<Item = String> {
    message::parse(message)
        .tokens
        .into_iter()
        .filter_map(|token| match token {
            Token::Link { href, .. }
                if url::Url::parse(&href)
                    .is_ok_and(|url| matches!(url.scheme(), "http" | "https")) =>
            {
                Some(href)
            }
            _ => None,
        })
}

fn decode_link_image(bytes: &[u8]) -> Option<(image::Handle, u32, u32)> {
    let reader = ::image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let (width, height) = reader.into_dimensions().ok()?;
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return None;
    }

    let decoded = ::image::load_from_memory(bytes).ok()?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();

    // Keep the preview reasonable on screen and avoid uploading huge GPU
    // textures; preserve the aspect ratio.
    let scale = (480.0 / width as f32).min(360.0 / height as f32).min(1.0);
    let (width, height, buffer) = if scale < 1.0 {
        let scaled_width = ((width as f32 * scale).round() as u32).max(1);
        let scaled_height = ((height as f32 * scale).round() as u32).max(1);
        let scaled = ::image::imageops::resize(
            &rgba,
            scaled_width,
            scaled_height,
            ::image::imageops::FilterType::Triangle,
        );
        (scaled_width, scaled_height, scaled)
    } else {
        (width, height, rgba)
    };
    Some((
        image::Handle::from_rgba(width, height, buffer.into_raw()),
        width,
        height,
    ))
}

fn decode_totoz_image(bytes: &[u8]) -> Option<TotozImage> {
    let reader = ::image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let (width, height) = reader.into_dimensions().ok()?;
    if !(2..=512).contains(&width) || !(2..=512).contains(&height) {
        return None;
    }

    if bytes.starts_with(b"GIF8") {
        use ::image::AnimationDecoder;
        if let Ok(decoder) = ::image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)) {
            if let Ok(frames) = decoder.into_frames().collect_frames() {
                let mut handles = Vec::new();
                let mut delays = Vec::new();
                for frame in frames.into_iter().take(32) {
                    let delay: Duration = frame.delay().into();
                    let buffer = frame.into_buffer();
                    if buffer.width() != width || buffer.height() != height {
                        continue;
                    }
                    handles.push(image::Handle::from_rgba(width, height, buffer.into_raw()));
                    delays.push(std::cmp::max(delay, Duration::from_millis(20)));
                }
                if !handles.is_empty() {
                    return Some(TotozImage {
                        frames: handles,
                        delays,
                        width,
                        height,
                    });
                }
            }
        }
    }

    let decoded = ::image::load_from_memory(bytes).ok()?;
    let rgba = decoded.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(TotozImage {
        frames: vec![image::Handle::from_rgba(width, height, rgba.into_raw())],
        delays: vec![Duration::ZERO],
        width,
        height,
    })
}

fn parse_csv(value: &str) -> BTreeSet<String> {
    value
        .split(',')
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn build_sse(config: &net::SseConfig) -> impl futures::Stream<Item = Message> {
    let server = config.server_url.clone();
    net::sse_stream(config.clone()).map(move |event| Message::Sse(server.clone(), event))
}

/// Focus a text input and place the caret at the end so typing can resume.
fn focus_text_input(id: &Id) -> Task<Message> {
    iced::widget::operation::focus(id.clone())
        .chain(iced::widget::operation::move_cursor_to_end(id.clone()))
}

/// Keep only names containing `query`, with prefix matches first, then the
/// rest in alphabetical order. The result never exceeds `limit`.
fn rank_totoz_suggestions(results: Vec<net::Totoz>, query: &str, limit: usize) -> Vec<net::Totoz> {
    let query = query.to_lowercase();
    let mut matches: Vec<net::Totoz> = results
        .into_iter()
        .filter(|totoz| totoz.name.to_lowercase().contains(&query))
        .collect();
    matches.sort_by(|left, right| {
        let left_name = left.name.to_lowercase();
        let right_name = right.name.to_lowercase();
        let left_prefix = !left_name.starts_with(&query);
        let right_prefix = !right_name.starts_with(&query);
        left_prefix
            .cmp(&right_prefix)
            .then_with(|| left_name.cmp(&right_name))
    });
    matches.dedup_by(|left, right| left.name.eq_ignore_ascii_case(&right.name));
    matches.truncate(limit);
    matches
}

/// Return the unfinished `[:…` totoz term typed at the very end of `text`.
///
/// A term only counts once it holds [`TOTOZ_SUGGEST_MIN_CHARS`] characters:
/// after `[:` and two characters the autocompletion can start. Closed tokens
/// (`[:duck]`) and line breaks end the term.
fn totoz_suggestion_term(text: &str) -> Option<&str> {
    let start = text.rfind("[:")?;
    let term = text[start + 2..].trim();
    if term.contains(']')
        || term.contains('\n')
        || term.contains('\r')
        || term.chars().count() < TOTOZ_SUGGEST_MIN_CHARS
    {
        return None;
    }
    Some(term)
}

/// Search the configured totoz server, falling back to a gb2c relay when the
/// server has no direct match. Shared by the totoz screen and the inline
/// autocompletion.
async fn search_totoz_any(
    client: &reqwest::Client,
    server: &str,
    gb2c: Option<&str>,
    terms: &str,
) -> Result<Vec<net::Totoz>, String> {
    match net::search_totoz(client, server, terms).await {
        Ok(results) if !results.is_empty() => Ok(results),
        Ok(_) => match gb2c {
            Some(base) => net::search_totoz_gb2c(client, base, terms).await,
            None => Ok(Vec::new()),
        },
        Err(error) => match gb2c {
            Some(base) => net::search_totoz_gb2c(client, base, terms)
                .await
                .map_err(|fallback| format!("{error} ; relais gb2c : {fallback}")),
            None => Err(error),
        },
    }
}

fn keyboard_shortcut(event: keyboard::Event) -> Option<Message> {
    match event {
        keyboard::Event::KeyPressed { .. } => Some(Message::Keyboard(event)),
        _ => None,
    }
}

fn notification_allowed(
    notifications: &NotifyConfig,
    focused: bool,
    is_bigorno: bool,
    is_reply: bool,
) -> bool {
    if !notifications.enabled {
        return false;
    }
    // Bigornos are direct mentions: keep them even when the window is in the
    // foreground, so a nick< is never missed.
    if notifications.only_unfocused && focused && !is_bigorno {
        return false;
    }
    notifications.all_posts
        || (notifications.bigorno && is_bigorno)
        || (notifications.replies && is_reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_http_links_from_posts() {
        let links = post_links(
            "see https://example.com/a and <a href=\"http://example.org/x\">label</a> \
             plus ftp://example.net/file",
        )
        .collect::<Vec<_>>();
        assert_eq!(
            links,
            vec![
                "https://example.com/a".to_owned(),
                "http://example.org/x".to_owned()
            ]
        );
    }

    #[test]
    fn bigornos_notify_even_when_focused() {
        let notifications = NotifyConfig::default();
        assert!(notifications.only_unfocused);
        assert!(notification_allowed(&notifications, true, true, false));
        assert!(!notification_allowed(&notifications, true, false, true));
        assert!(notification_allowed(&notifications, false, false, true));
    }

    #[test]
    fn disabled_notifications_stay_disabled() {
        let notifications = NotifyConfig {
            enabled: false,
            ..NotifyConfig::default()
        };
        assert!(!notification_allowed(&notifications, false, true, true));
    }

    #[test]
    fn totoz_autocomplete_starts_after_two_chars() {
        assert_eq!(totoz_suggestion_term("hello [:du"), Some("du"));
        assert_eq!(totoz_suggestion_term("[:du"), Some("du"));
        assert_eq!(totoz_suggestion_term("[:d"), None);
        assert_eq!(totoz_suggestion_term("[:  d"), None);
        assert_eq!(totoz_suggestion_term("[:éé"), Some("éé"));
    }

    #[test]
    fn totoz_autocomplete_ignores_closed_tokens() {
        assert_eq!(totoz_suggestion_term("hello"), None);
        assert_eq!(totoz_suggestion_term("[:duck] hello"), None);
        assert_eq!(totoz_suggestion_term("[:du\nmore"), None);
        assert_eq!(totoz_suggestion_term("word [:du "), Some("du"));
        assert_eq!(totoz_suggestion_term("word [:du] then [:mo"), Some("mo"));
    }

    fn totoz(name: &str) -> net::Totoz {
        net::Totoz {
            name: name.to_owned(),
            image: String::new(),
        }
    }

    #[test]
    fn totoz_suggestions_prefer_prefix_matches() {
        let ranked = rank_totoz_suggestions(
            vec![
                totoz("goodduck"),
                totoz("duckling"),
                totoz("Duck"),
                totoz("cat"),
            ],
            "du",
            8,
        );
        let names: Vec<_> = ranked.iter().map(|totoz| totoz.name.as_str()).collect();
        assert_eq!(names, vec!["Duck", "duckling", "goodduck"]);
    }

    #[test]
    fn totoz_suggestions_are_empty_without_a_match() {
        let ranked = rank_totoz_suggestions(vec![totoz("duck")], "zz", 8);
        assert!(ranked.is_empty());
    }

    #[test]
    fn totoz_suggestions_are_capped() {
        let results = (0..12)
            .map(|index| totoz(&format!("duck{index}")))
            .collect();
        assert_eq!(rank_totoz_suggestions(results, "duck", 8).len(), 8);
    }
}
