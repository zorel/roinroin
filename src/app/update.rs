//! Message handling for the application.

use super::*;

impl App {
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
                        self.shift_held = false;
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
            Message::ModifiersChanged(modifiers) => {
                self.shift_held = modifiers.shift();
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
            Message::PostRightClicked(key) => {
                if self.shift_held {
                    self.open_conversation(key);
                }
            }
            Message::ConversationScrolled(anchor, at_bottom) => {
                if let Some(pane) = self
                    .conversations
                    .iter_mut()
                    .find(|pane| pane.anchor == anchor)
                {
                    pane.follow = at_bottom;
                }
            }
            Message::CloseConversation(anchor) => {
                self.conversations.retain(|pane| pane.anchor != anchor);
                self.conversations_dirty = true;
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
                self.composer = auto_close_formatting_tag(&self.composer, &value).unwrap_or(value);
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

        let analysis_changed = self.ensure_analysis();
        if analysis_changed || self.conversations_dirty {
            self.refresh_conversations(&mut tasks);
            self.conversations_dirty = false;
        }
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
}
