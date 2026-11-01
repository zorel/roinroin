//! Composer behavior for the application.

use unicode_segmentation::UnicodeSegmentation;

use super::*;

impl App {
    /// Place the caret at the end of the draft and focus the entry field.
    pub(super) fn focus_composer(&mut self) -> Task<Message> {
        self.composer_caret = self.composer.graphemes(true).count();
        focus_text_input(&self.composer_id)
    }

    /// Insert text at the caret and place the caret right after it, so clock
    /// quotes, totoz and emoji insertions land where the user is typing.
    pub(super) fn insert_composer(&mut self, text: &str) -> Task<Message> {
        let (composer, caret) = insert_at_grapheme(&self.composer, self.composer_caret, text);
        self.composer = composer;
        self.composer_caret = caret;
        self.draft_dirty = true;
        self.totoz_suggest.clear();
        focus_text_input_at(&self.composer_id, self.composer_caret)
    }

    /// Detect the unfinished `[:…` token at the end of the composer and start
    /// a (debounced) totoz search when it changed. Anything shorter than
    /// [`TOTOZ_SUGGEST_MIN_CHARS`] characters closes the suggestion list.
    pub(super) fn refresh_totoz_suggest(&mut self) -> Vec<Task<Message>> {
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

    pub(super) fn apply_totoz_suggestions(
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
    pub(super) fn fetch_totoz_suggest_images(&mut self) -> Vec<Task<Message>> {
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
    pub(super) fn move_totoz_suggestion(&mut self, delta: isize) {
        let count = self.totoz_suggest.results.len();
        if count == 0 {
            return;
        }
        let current = self.totoz_suggest.selected.unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(count as isize) as usize;
        self.totoz_suggest.selected = Some(next);
    }

    pub(super) fn pick_totoz_suggestion(&mut self) -> Vec<Task<Message>> {
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
    pub(super) fn pick_totoz_suggestion_named(&mut self, name: &str) -> Vec<Task<Message>> {
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
        vec![self.focus_composer()]
    }
}

/// Insert `insertion` at the given grapheme index and return the new text
/// together with the caret placed right after the inserted part.
fn insert_at_grapheme(text: &str, caret: usize, insertion: &str) -> (String, usize) {
    let caret = caret.min(text.graphemes(true).count());
    let byte = text
        .grapheme_indices(true)
        .nth(caret)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len());
    let mut result = String::with_capacity(text.len() + insertion.len());
    result.push_str(&text[..byte]);
    result.push_str(insertion);
    result.push_str(&text[byte..]);
    (result, caret + insertion.graphemes(true).count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_at_the_caret_instead_of_the_end() {
        assert_eq!(
            insert_at_grapheme("hello world", 5, " there"),
            ("hello there world".to_owned(), 11)
        );
        assert_eq!(
            insert_at_grapheme("hello", 0, "> "),
            ("> hello".to_owned(), 2)
        );
    }

    #[test]
    fn insertion_respects_grapheme_boundaries() {
        let (text, caret) = insert_at_grapheme("a🇫🇷b", 2, "12:34:56 ");
        // The flag is one grapheme but several bytes.
        assert_eq!(text, "a🇫🇷12:34:56 b");
        assert_eq!(caret, 11);
    }

    #[test]
    fn caret_is_clamped_to_the_draft() {
        assert_eq!(insert_at_grapheme("hi", 99, "!"), ("hi!".to_owned(), 3));
    }
}
