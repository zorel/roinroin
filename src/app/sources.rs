//! Sources behavior for the application.

use super::*;

impl App {
    pub(super) fn ensure_analysis(&mut self) {
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

    pub(super) fn prune_posts(&mut self) {
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

    pub(super) fn sync_sources(&mut self) {
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

    pub(super) fn retain_current_sources(&mut self) {
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

    pub(super) fn ingest(
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
    pub(super) fn baseline_history(&mut self) {
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

    pub(super) fn refresh_tasks(&mut self) -> Vec<Task<Message>> {
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

    pub(super) fn board_task(&mut self, board: Board) -> Task<Message> {
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

    pub(super) fn poll_boards(&mut self) -> Vec<Task<Message>> {
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
}
