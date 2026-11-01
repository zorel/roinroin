//! Application state, update loop and subscriptions.

mod composer;
mod notifications;
mod posting;
mod previews;
mod settings;
mod sources;
mod update;

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
use crate::model::{compare_posts, quote_clock, Post, PostKey, SourceId, TribuneKey};
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

/// A conversation kept open next to the timeline. The whole thread is derived
/// from the analysis on every render, so new replies appear automatically.
#[derive(Debug, Clone)]
pub struct ConversationPane {
    /// Post that was right-clicked. The root is resolved again on every
    /// analysis rebuild so a newly discovered ancestor does not detach the
    /// pane from the rest of its thread.
    pub anchor: PostKey,
    /// Stay at the bottom when new messages arrive; turned off as soon as the
    /// user scrolls up to read older posts.
    pub follow: bool,
    /// Latest post already seen in this pane, used to detect arrivals.
    last: Option<PostKey>,
}

impl ConversationPane {
    pub fn scroll_id(&self) -> Id {
        Id::from(format!("roinroin-conversation:{}", self.anchor.key()))
    }
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
    ModifiersChanged(keyboard::Modifiers),
    ServerInfo(String, Result<net::ServerInfo, String>),
    BoardPosts(String, Result<Vec<Post>, String>),
    Sse(String, net::SseEvent),
    SelectTribune(TribuneKey),
    PostClicked(PostKey),
    PostRightClicked(PostKey),
    ConversationScrolled(PostKey, bool),
    CloseConversation(PostKey),
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
    pub(crate) conversations: Vec<ConversationPane>,
    pub(crate) conversations_dirty: bool,
    pub(crate) shift_held: bool,
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
            conversations: Vec::new(),
            conversations_dirty: false,
            shift_held: false,
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

    /// Whether a post passes the same filters as the main timeline.
    pub fn post_visible(&self, key: &PostKey) -> bool {
        let hidden = self
            .analysis
            .info(key)
            .map(|info| info.hidden)
            .unwrap_or(false);
        !hidden || self.config.show_hidden || self.config.blocked_mode == BlockedMode::Gray
    }

    pub fn visible_keys(&self) -> Vec<PostKey> {
        let Some(active) = &self.active else {
            return Vec::new();
        };
        self.analysis
            .posts_for(active)
            .iter()
            .filter(|key| self.post_visible(key))
            .cloned()
            .collect()
    }

    /// The visible posts of the whole discussion tree containing `key`,
    /// oldest first.
    pub fn conversation_keys(&self, key: &PostKey) -> Vec<PostKey> {
        let mut keys: Vec<PostKey> = self
            .analysis
            .conversation(key)
            .into_iter()
            .filter(|key| self.post_visible(key))
            .collect();
        keys.sort_by(
            |left, right| match (self.posts.get(left), self.posts.get(right)) {
                (Some(left_post), Some(right_post)) => {
                    compare_posts(left_post, right_post).then_with(|| left.cmp(right))
                }
                _ => left.cmp(right),
            },
        );
        keys
    }

    /// Open the conversation containing `key`, unless that conversation is
    /// already displayed in a pane.
    pub(super) fn open_conversation(&mut self, key: PostKey) {
        if self
            .conversations
            .iter()
            .any(|pane| self.analysis.conversation(&pane.anchor).contains(&key))
        {
            return;
        }
        self.conversations.push(ConversationPane {
            anchor: key,
            follow: true,
            last: None,
        });
        self.conversations_dirty = true;
    }

    /// Drop panes whose post disappeared, merge panes that now belong to the
    /// same discussion tree, and keep the remaining ones following replies.
    pub(super) fn refresh_conversations(&mut self, tasks: &mut Vec<Task<Message>>) {
        let mut trees: Vec<BTreeSet<PostKey>> = Vec::new();
        self.conversations.retain(|pane| {
            if !self.posts.contains_key(&pane.anchor) {
                return false;
            }
            if trees.iter().any(|members| members.contains(&pane.anchor)) {
                return false;
            }
            trees.push(
                self.analysis
                    .conversation(&pane.anchor)
                    .into_iter()
                    .collect(),
            );
            true
        });
        let latest: Vec<(PostKey, Option<PostKey>)> = self
            .conversations
            .iter()
            .map(|pane| {
                (
                    pane.anchor.clone(),
                    self.conversation_keys(&pane.anchor).last().cloned(),
                )
            })
            .collect();
        for (anchor, latest) in latest {
            let Some(pane) = self
                .conversations
                .iter_mut()
                .find(|pane| pane.anchor == anchor)
            else {
                continue;
            };
            if pane.last == latest {
                continue;
            }
            pane.last = latest;
            if pane.follow {
                tasks.push(iced::widget::operation::snap_to_end(pane.scroll_id()));
            }
        }
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
        keyboard::Event::ModifiersChanged(modifiers) => Some(Message::ModifiersChanged(modifiers)),
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
