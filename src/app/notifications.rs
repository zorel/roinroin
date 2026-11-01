//! Notifications behavior for the application.

use super::*;

impl App {
    pub(super) fn dispatch_notifications(&mut self) -> Vec<Task<Message>> {
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
