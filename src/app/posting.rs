//! Posting behavior for the application.

use super::*;

impl App {
    pub(super) fn send_message(&mut self) -> Vec<Task<Message>> {
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
}
