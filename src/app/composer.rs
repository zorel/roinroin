//! Composer behavior for the application.

use unicode_segmentation::UnicodeSegmentation;

use super::*;

impl App {
    /// Place the caret at the end of the draft and focus the entry field.
    pub(super) fn focus_composer(&mut self) -> Task<Message> {
        self.composer_caret = self.composer.graphemes(true).count();
        self.composer_selection = None;
        focus_text_input(&self.composer_id)
    }

    /// Insert text at the caret (or replace the selection) and place the
    /// caret right after it, so clock quotes, totoz and emoji insertions land
    /// where the user is typing.
    pub(super) fn insert_composer(&mut self, text: &str) -> Task<Message> {
        let (start, end) = self.selection_or_caret();
        let (composer, caret) = replace_graphemes(&self.composer, start, end, text);
        self.apply_composer(composer, caret)
    }

    /// Wrap the selection, or insert a pair of formatting tags with the caret
    /// between them when nothing is selected.
    pub(super) fn insert_formatting(&mut self, open: &str, close: &str) -> Task<Message> {
        let (composer, caret) = wrap_or_insert(
            &self.composer,
            self.composer_caret,
            self.composer_selection,
            open,
            close,
        );
        self.apply_composer(composer, caret)
    }

    /// Insert a moment marker. Without a selection the word "Moment" stays
    /// editable between the tags; with a selection it is replaced by the
    /// selected text.
    pub(super) fn insert_moment(&mut self) -> Task<Message> {
        let (composer, caret) =
            insert_moment_at(&self.composer, self.composer_caret, self.composer_selection);
        self.apply_composer(composer, caret)
    }

    fn selection_or_caret(&self) -> (usize, usize) {
        let length = self.composer.graphemes(true).count();
        valid_selection(&self.composer, self.composer_selection).unwrap_or((
            self.composer_caret.min(length),
            self.composer_caret.min(length),
        ))
    }

    fn apply_composer(&mut self, composer: String, caret: usize) -> Task<Message> {
        self.composer = composer;
        self.composer_caret = caret;
        self.composer_selection = None;
        self.draft_dirty = true;
        self.totoz_suggest.clear();
        focus_text_input_at(&self.composer_id, caret)
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

/// Width of `text` in grapheme clusters, as used by the text input cursor.
fn grapheme_len(text: &str) -> usize {
    text.graphemes(true).count()
}

/// Byte offset of a grapheme index in `text`, clamped to its end.
fn grapheme_byte_index(text: &str, index: usize) -> usize {
    text.grapheme_indices(true)
        .nth(index)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

fn grapheme_slice(text: &str, start: usize, end: usize) -> &str {
    &text[grapheme_byte_index(text, start)..grapheme_byte_index(text, end)]
}

/// Normalize a selection to a non-empty range inside `text`.
fn valid_selection(text: &str, selection: Option<(usize, usize)>) -> Option<(usize, usize)> {
    let length = grapheme_len(text);
    selection.and_then(|(start, end)| {
        let start = start.min(length);
        let end = end.min(length);
        (start < end).then_some((start, end))
    })
}

/// Replace a grapheme range and place the caret after the replacement.
fn replace_graphemes(text: &str, start: usize, end: usize, replacement: &str) -> (String, usize) {
    let length = grapheme_len(text);
    let start = start.min(length);
    let end = end.clamp(start, length);
    let start_byte = grapheme_byte_index(text, start);
    let end_byte = grapheme_byte_index(text, end);
    let mut result = String::with_capacity(text.len() + replacement.len());
    result.push_str(&text[..start_byte]);
    result.push_str(replacement);
    result.push_str(&text[end_byte..]);
    (result, start + grapheme_len(replacement))
}

/// Wrap the selection, or insert `open` and `close` with the caret between
/// them when there is no selection.
fn wrap_or_insert(
    text: &str,
    caret: usize,
    selection: Option<(usize, usize)>,
    open: &str,
    close: &str,
) -> (String, usize) {
    if let Some((start, end)) = valid_selection(text, selection) {
        let inner = grapheme_slice(text, start, end);
        return replace_graphemes(text, start, end, &format!("{open}{inner}{close}"));
    }

    let caret = caret.min(grapheme_len(text));
    let (result, _) = replace_graphemes(text, caret, caret, &format!("{open}{close}"));
    (result, caret + grapheme_len(open))
}

/// Build a moment marker, keeping the caret just before the bold closing tag.
/// The word "Moment" stays in place; selected text is appended after it.
fn insert_moment_at(
    text: &str,
    caret: usize,
    selection: Option<(usize, usize)>,
) -> (String, usize) {
    const OPEN: &str = "====> <b>Moment ";
    const CLOSE: &str = "</b> <====";
    if let Some((start, end)) = valid_selection(text, selection) {
        let inner = grapheme_slice(text, start, end);
        let (result, _) = replace_graphemes(text, start, end, &format!("{OPEN}{inner}{CLOSE}"));
        let caret = start + grapheme_len(OPEN) + grapheme_len(inner);
        return (result, caret);
    }

    let caret = caret.min(grapheme_len(text));
    let (result, _) = replace_graphemes(text, caret, caret, &format!("{OPEN}{CLOSE}"));
    (result, caret + grapheme_len(OPEN))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inserts_at_the_caret_instead_of_the_end() {
        assert_eq!(
            replace_graphemes("hello world", 5, 5, " there"),
            ("hello there world".to_owned(), 11)
        );
        assert_eq!(
            replace_graphemes("hello", 0, 0, "> "),
            ("> hello".to_owned(), 2)
        );
    }

    #[test]
    fn insertion_respects_grapheme_boundaries() {
        let (text, caret) = replace_graphemes("a🇫🇷b", 2, 2, "12:34:56 ");
        // The flag is one grapheme but several bytes.
        assert_eq!(text, "a🇫🇷12:34:56 b");
        assert_eq!(caret, 11);
    }

    #[test]
    fn caret_is_clamped_to_the_draft() {
        assert_eq!(replace_graphemes("hi", 99, 99, "!"), ("hi!".to_owned(), 3));
    }

    #[test]
    fn formatting_tags_keep_the_caret_between_them() {
        assert_eq!(
            wrap_or_insert("hello", 5, None, "<b>", "</b>"),
            ("hello<b></b>".to_owned(), 8)
        );
        assert_eq!(
            wrap_or_insert("hello", 3, None, "<i>", "</i>"),
            ("hel<i></i>lo".to_owned(), 6)
        );
    }

    #[test]
    fn formatting_tags_wrap_the_selection() {
        assert_eq!(
            wrap_or_insert("hello world", 0, Some((6, 11)), "<b>", "</b>"),
            ("hello <b>world</b>".to_owned(), 18)
        );
    }

    #[test]
    fn moment_keeps_the_caret_before_the_closing_tag() {
        assert_eq!(
            insert_moment_at("", 0, None),
            ("====> <b>Moment </b> <====".to_owned(), 16)
        );
        assert_eq!(
            insert_moment_at("abc", 3, None),
            ("abc====> <b>Moment </b> <====".to_owned(), 19)
        );
    }

    #[test]
    fn moment_keeps_its_label_before_the_selection() {
        assert_eq!(
            insert_moment_at("hello", 0, Some((0, 5))),
            ("====> <b>Moment hello</b> <====".to_owned(), 21)
        );
    }
}
