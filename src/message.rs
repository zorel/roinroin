//! BML/SBML scanner.
//!
//! The wire format is the old bouchot pseudo-HTML: a handful of formatting
//! tags, raw URLs, `[:totoz]` images, `nick<` bigornos and clock references.
//! The scanner is intentionally tolerant: invalid or unclosed tags are shown
//! as text instead of breaking a post.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use chrono::{NaiveDate, NaiveTime};
use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;

use crate::model::{NorlogeKind, NorlogeRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InlineStyle {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub monospace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Text {
        text: String,
        style: InlineStyle,
        spoiler: Option<usize>,
    },
    Link {
        href: String,
        label: String,
    },
    Totoz {
        name: String,
    },
    Norloge {
        reference_index: usize,
        reference: NorlogeRef,
    },
    Bigorno {
        word: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedMessage {
    pub source: String,
    pub tokens: Vec<Token>,
    pub norloges: Vec<NorlogeRef>,
    pub bigornos: BTreeSet<String>,
    pub spoiler_count: usize,
    pub has_duck: bool,
}

impl ParsedMessage {
    pub fn plain_text(&self) -> String {
        let mut result = String::new();
        for token in &self.tokens {
            match token {
                Token::Text { text, .. } => result.push_str(text),
                Token::Link { href, .. } => result.push_str(href),
                Token::Totoz { name } => {
                    result.push_str("[:");
                    result.push_str(name);
                    result.push(']');
                }
                Token::Norloge { reference, .. } => result.push_str(&reference.raw),
                Token::Bigorno { word } => {
                    result.push_str(word);
                    result.push('<');
                }
            }
        }
        result.trim().to_owned()
    }

    pub fn has_bigorno(&self, identities: &BTreeSet<String>) -> bool {
        self.bigornos.iter().any(|word| identities.contains(word))
    }
}

pub fn parse(message: &str) -> ParsedMessage {
    parse_with_identities(message, &BTreeSet::new())
}

/// Like [`parse`], but also recognizes the given identities as bigornos even
/// when they contain spaces or punctuation that the generic scanner rejects.
pub fn parse_with_identities(message: &str, identities: &BTreeSet<String>) -> ParsedMessage {
    let cleaned = clean(message);
    let source = expand_anchors(&cleaned);
    let mut tokens: Vec<Token> = Vec::new();
    let mut norloges: Vec<NorlogeRef> = Vec::new();
    let mut bigornos = BTreeSet::new();
    let mut stack: Vec<TagEntry> = Vec::new();
    let mut spoiler_count = 0usize;
    let mut text = String::new();
    let mut style = InlineStyle::default();
    let mut active_spoiler: Option<usize> = None;

    let flush =
        |tokens: &mut Vec<Token>, text: &mut String, style: InlineStyle, spoiler: Option<usize>| {
            if text.is_empty() {
                return;
            }
            let decoded = decode_entities(text);
            text.clear();
            if decoded.is_empty() {
                return;
            }
            // Merge adjacent text tokens with the same style; this keeps the
            // rendered span count small without changing semantics.
            if let Some(Token::Text {
                text: previous,
                style: previous_style,
                spoiler: previous_spoiler,
            }) = tokens.last_mut()
            {
                if *previous_style == style && *previous_spoiler == spoiler {
                    previous.push_str(&decoded);
                    return;
                }
            }
            tokens.push(Token::Text {
                text: decoded,
                style,
                spoiler,
            });
        };

    let recompute = |stack: &[TagEntry]| {
        let mut style = InlineStyle::default();
        let mut spoiler = None;
        for entry in stack {
            match entry.name.as_str() {
                "b" => style.bold = true,
                "i" => style.italic = true,
                "u" => style.underline = true,
                "s" => style.strike = true,
                "tt" | "code" => style.monospace = true,
                "spoiler" => spoiler = entry.spoiler_id,
                _ => {}
            }
        }
        (style, spoiler)
    };

    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < source.len() {
        let code = bytes[index] as char;

        if (code == 'h' || code == 'f' || code == 'd' || code == 'H' || code == 'F' || code == 'D')
            && source[index..].get(..4).is_some()
        {
            if let Some(matched) = url_regex().find(&source[index..]) {
                if matched.start() == 0 {
                    flush(&mut tokens, &mut text, style, active_spoiler);
                    let (href, trailing) = split_trailing_punctuation(matched.as_str());
                    if !href.is_empty() {
                        tokens.push(Token::Link {
                            href: href.to_owned(),
                            label: format!("{}{trailing}", url_label(href)),
                        });
                        index += matched.as_str().len();
                        continue;
                    }
                }
            }
        }

        if code == '[' {
            if let Some(captures) = totoz_regex().captures(&source[index..]) {
                if let Some(matched) = captures.get(0) {
                    if matched.start() == 0 {
                        if let Some(name) = captures.get(1) {
                            flush(&mut tokens, &mut text, style, active_spoiler);
                            tokens.push(Token::Totoz {
                                name: name.as_str().to_owned(),
                            });
                            index += matched.as_str().len();
                            continue;
                        }
                    }
                }
            }
        }

        if code != '\n' && is_bigorno_boundary(bytes, index) {
            if let Some(found) = match_identity_bigorno(&source, index, identities)
                .or_else(|| match_bigorno(&source, index))
            {
                flush(&mut tokens, &mut text, style, active_spoiler);
                if let Some(spaces) = found.leading {
                    if let Some(Token::Text { text: previous, .. }) = tokens.last_mut() {
                        previous.push_str(spaces);
                    } else {
                        tokens.push(Token::Text {
                            text: spaces.to_owned(),
                            style,
                            spoiler: active_spoiler,
                        });
                    }
                }
                bigornos.insert(found.word.clone());
                tokens.push(Token::Bigorno { word: found.word });
                index = found.end;
                continue;
            }
        }

        if code.is_ascii_digit() || code == '#' {
            if let Some((reference, end)) = match_norloge(&source, index) {
                flush(&mut tokens, &mut text, style, active_spoiler);
                let reference_index = norloges.len();
                norloges.push(reference.clone());
                tokens.push(Token::Norloge {
                    reference_index,
                    reference,
                });
                index = end;
                continue;
            }
        }

        if code == '<' {
            if let Some(matched) = br_regex().find(&source[index..]) {
                if matched.start() == 0 {
                    flush(&mut tokens, &mut text, style, active_spoiler);
                    text.push('\n');
                    flush(&mut tokens, &mut text, style, active_spoiler);
                    index += matched.as_str().len();
                    continue;
                }
            }
            if let Some(captures) = open_tag_regex().captures(&source[index..]) {
                if let Some(matched) = captures.get(0) {
                    if matched.start() == 0 {
                        if let Some(name) = captures.get(1) {
                            flush(&mut tokens, &mut text, style, active_spoiler);
                            let name = normalize_tag(name.as_str());
                            let spoiler_id = if name == "spoiler" {
                                spoiler_count += 1;
                                Some(spoiler_count - 1)
                            } else {
                                None
                            };
                            stack.push(TagEntry { name, spoiler_id });
                            (style, active_spoiler) = recompute(&stack);
                            index += matched.as_str().len();
                            continue;
                        }
                    }
                }
            }
            if let Some(captures) = close_tag_regex().captures(&source[index..]) {
                if let Some(matched) = captures.get(0) {
                    if matched.start() == 0 {
                        if let Some(name) = captures.get(1) {
                            flush(&mut tokens, &mut text, style, active_spoiler);
                            let name = normalize_tag(name.as_str());
                            if let Some(position) =
                                stack.iter().rposition(|entry| entry.name == name)
                            {
                                stack.truncate(position);
                            }
                            (style, active_spoiler) = recompute(&stack);
                            index += matched.as_str().len();
                            continue;
                        }
                    }
                }
            }
            text.push('<');
            index += 1;
            continue;
        }

        if code == '&' {
            if let Some(matched) = entity_regex().find(&source[index..]) {
                if matched.start() == 0 {
                    text.push_str(matched.as_str());
                    index += matched.as_str().len();
                    continue;
                }
            }
        }

        // Copy one UTF-8 scalar, not one byte.
        let character = source[index..].chars().next().unwrap_or('\u{fffd}');
        text.push(character);
        index += character.len_utf8();
    }
    flush(&mut tokens, &mut text, style, active_spoiler);

    let has_duck = duck_regex().is_match(&source);
    ParsedMessage {
        source,
        tokens,
        norloges,
        bigornos,
        spoiler_count,
        has_duck,
    }
}

/// Rewrite the `@tribune` suffix of clock references when the user switches
/// tribune while composing, matching gc2/Android.
pub fn rewrite_bouchot_suffix(message: &str, old_tribune: &str, new_tribune: &str) -> String {
    if old_tribune == new_tribune {
        return message.to_owned();
    }
    let mut result = String::with_capacity(message.len() + 8);
    let mut index = 0usize;
    while index < message.len() {
        let code = message.as_bytes()[index] as char;
        if code.is_ascii_digit() || code == '#' {
            if let Some((reference, end)) = match_norloge(message, index) {
                let suffix = match &reference.target {
                    None => {
                        if old_tribune.is_empty() {
                            String::new()
                        } else {
                            format!("@{old_tribune}")
                        }
                    }
                    Some(target) if target == new_tribune => String::new(),
                    Some(target) => format!("@{target}"),
                };
                let base = strip_norloge_suffix(&reference.raw);
                result.push_str(base);
                result.push_str(&suffix);
                index = end;
                continue;
            }
        }
        if code == '[' {
            if let Some(matched) = totoz_regex().find(&message[index..]) {
                if matched.start() == 0 {
                    result.push_str(matched.as_str());
                    index += matched.as_str().len();
                    continue;
                }
            }
        }
        let character = message[index..].chars().next().unwrap_or('\u{fffd}');
        result.push(character);
        index += character.len_utf8();
    }
    result
}

fn strip_norloge_suffix(raw: &str) -> &str {
    if let Some(at) = raw.rfind('@') {
        if raw[at + 1..].chars().all(|c| c.is_ascii_lowercase()) {
            return &raw[..at];
        }
    }
    raw
}
fn clean(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\r' => result.push(' '),
            '\n' | '\t' => result.push(character),
            character if character.is_control() => {}
            character => result.push(character),
        }
    }
    result
}

fn expand_anchors(message: &str) -> String {
    anchor_regex()
        .replace_all(message, |captures: &regex::Captures<'_>| {
            let href = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            if href.starts_with("http://")
                || href.starts_with("https://")
                || href.starts_with("ftp://")
            {
                href.to_owned()
            } else {
                captures
                    .get(0)
                    .map(|m| m.as_str().to_owned())
                    .unwrap_or_default()
            }
        })
        .into_owned()
}

fn normalize_tag(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "strong" => "b".to_owned(),
        "em" => "i".to_owned(),
        "strike" | "del" => "s".to_owned(),
        other => other.to_owned(),
    }
}

fn is_message_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

/// A bigorno can start at the beginning of the message, after whitespace
/// (including a newline) or directly after a BML tag.
fn is_bigorno_boundary(bytes: &[u8], index: usize) -> bool {
    if index == 0 || is_message_whitespace(bytes[index]) {
        return true;
    }
    bytes
        .get(index - 1)
        .is_some_and(|byte| is_message_whitespace(*byte) || *byte == b'>')
}

struct BigornoMatch {
    end: usize,
    leading: Option<&'static str>,
    word: String,
}

fn match_bigorno(message: &str, index: usize) -> Option<BigornoMatch> {
    let bytes = message.as_bytes();
    let mut cursor = index;
    let mut leading = None;
    if cursor < bytes.len() && is_message_whitespace(bytes[cursor]) {
        leading = Some(" ");
        cursor += 1;
    }
    let word_start = cursor;
    while cursor < bytes.len() && is_bigorno_char(bytes[cursor] as char) {
        cursor += 1;
    }
    if cursor == word_start {
        return None;
    }
    let after = bigorno_end(message, cursor)?;
    Some(BigornoMatch {
        end: after,
        leading,
        word: message[word_start..cursor].to_owned(),
    })
}

/// Match a known identity followed by `<`, so logins and user agents that
/// contain spaces, dots or other punctuation are recognized as bigornos.
fn match_identity_bigorno(
    message: &str,
    index: usize,
    identities: &BTreeSet<String>,
) -> Option<BigornoMatch> {
    if identities.is_empty() {
        return None;
    }
    let bytes = message.as_bytes();
    let mut cursor = index;
    let mut leading = None;
    if cursor < bytes.len() && is_message_whitespace(bytes[cursor]) {
        leading = Some(" ");
        cursor += 1;
    }
    let mut best: Option<(usize, usize)> = None;
    for identity in identities {
        if identity.is_empty() || !message[cursor..].starts_with(identity.as_str()) {
            continue;
        }
        let end = cursor + identity.len();
        let Some(after) = bigorno_end(message, end) else {
            continue;
        };
        if best.is_none_or(|(length, _)| identity.len() > length) {
            best = Some((identity.len(), after));
        }
    }
    let (length, after) = best?;
    Some(BigornoMatch {
        end: after,
        leading,
        word: message[cursor..cursor + length].to_owned(),
    })
}

/// Returns the end of the bigorno marker at `cursor`, if any.
fn bigorno_end(message: &str, cursor: usize) -> Option<usize> {
    let after = if message[cursor..].starts_with('<') {
        cursor + 1
    } else if message[cursor..].starts_with("&lt;") {
        cursor + 4
    } else {
        return None;
    };
    if after < message.len() {
        let next = message[after..].chars().next()?;
        if !(is_message_whitespace(next as u8)
            || next == '<'
            || next == ';'
            || next == ':'
            || next == '['
            || next == ','
            || next == '\n')
        {
            return None;
        }
    }
    Some(after)
}

fn is_bigorno_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
}

fn match_norloge(message: &str, index: usize) -> Option<(NorlogeRef, usize)> {
    let rest = &message[index..];

    if let Some(captures) = full_datetime_regex().captures(rest) {
        if let Some(matched) = captures.get(0) {
            if matched.start() == 0 {
                let year: i32 = captures.get(1)?.as_str().parse().ok()?;
                let month: u32 = captures.get(2)?.as_str().parse().ok()?;
                let day: u32 = captures.get(3)?.as_str().parse().ok()?;
                let hour: u32 = captures.get(4)?.as_str().parse().ok()?;
                let minute: u32 = captures.get(5)?.as_str().parse().ok()?;
                let second: u32 = captures.get(6)?.as_str().parse().ok()?;
                let datetime =
                    NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, second)?;
                return finish_norloge(
                    matched.as_str(),
                    index + matched.as_str().len(),
                    NorlogeKind::FullDateTime(datetime),
                    message,
                );
            }
        }
    }

    if let Some(captures) = long_date_regex().captures(rest) {
        if let Some(matched) = captures.get(0) {
            if matched.start() == 0 {
                let month: u32 = captures.get(1)?.as_str().parse().ok()?;
                let day: u32 = captures.get(2)?.as_str().parse().ok()?;
                let hour: u32 = captures.get(3)?.as_str().parse().ok()?;
                let minute: u32 = captures.get(4)?.as_str().parse().ok()?;
                let second: u32 = captures.get(5)?.as_str().parse().ok()?;
                if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
                    return None;
                }
                if message[index + matched.as_str().len()..]
                    .chars()
                    .next()
                    .is_some_and(|next| next.is_ascii_digit())
                {
                    return None;
                }
                let clock = NaiveTime::from_hms_opt(hour, minute, second)?;
                return finish_norloge(
                    matched.as_str(),
                    index + matched.as_str().len(),
                    NorlogeKind::DateClock { month, day, clock },
                    message,
                );
            }
        }
    }

    if let Some(captures) = clock_regex().captures(rest) {
        if let Some(matched) = captures.get(0) {
            if matched.start() == 0 {
                let hour: u32 = captures.get(1)?.as_str().parse().ok()?;
                let minute: u32 = captures.get(2)?.as_str().parse().ok()?;
                let second: u32 = captures.get(3)?.as_str().parse().ok()?;
                if message[index + matched.as_str().len()..]
                    .chars()
                    .next()
                    .is_some_and(|next| next.is_ascii_digit())
                {
                    return None;
                }
                let clock = NaiveTime::from_hms_opt(hour, minute, second)?;
                return finish_norloge(
                    matched.as_str(),
                    index + matched.as_str().len(),
                    NorlogeKind::Clock(clock),
                    message,
                );
            }
        }
    }

    if let Some(captures) = short_clock_regex().captures(rest) {
        if let Some(matched) = captures.get(0) {
            if matched.start() == 0 {
                let hour: u32 = captures.get(1)?.as_str().parse().ok()?;
                let minute: u32 = captures.get(2)?.as_str().parse().ok()?;
                let rest = &message[index + matched.as_str().len()..];
                if rest.starts_with(':')
                    || rest
                        .chars()
                        .next()
                        .is_some_and(|next| next.is_ascii_digit())
                {
                    return None;
                }
                let clock = NaiveTime::from_hms_opt(hour, minute, 0)?;
                return finish_norloge(
                    matched.as_str(),
                    index + matched.as_str().len(),
                    NorlogeKind::Short(clock),
                    message,
                );
            }
        }
    }

    if message.as_bytes().get(index) == Some(&b'#') {
        if let Some(captures) = id_regex().captures(rest) {
            if let Some(matched) = captures.get(0) {
                if matched.start() == 0 {
                    let id: i64 = captures.get(1)?.as_str().parse().ok()?;
                    if id > 0 {
                        return finish_norloge(
                            matched.as_str(),
                            index + matched.as_str().len(),
                            NorlogeKind::Id(id),
                            message,
                        );
                    }
                }
            }
        }
    }

    None
}

fn finish_norloge(
    base: &str,
    base_end: usize,
    kind: NorlogeKind,
    message: &str,
) -> Option<(NorlogeRef, usize)> {
    let mut end = base_end;
    let rest = &message[base_end..];
    let mut indice = None;
    if let Some(character) = rest.chars().next() {
        if character == '^' {
            if let Some(digit) = rest.chars().nth(1).and_then(|c| c.to_digit(10)) {
                if (1..=9).contains(&digit) {
                    indice = Some(digit as u8);
                    end += 2;
                }
            }
        } else if let Some(digit) = superscript(character) {
            indice = Some(digit);
            end += character.len_utf8();
        }
    }
    let mut target = None;
    if let Some(character) = message[end..].chars().next() {
        if character == '@' {
            let after = &message[end + 1..];
            let length = after.chars().take_while(|c| c.is_ascii_lowercase()).count();
            if length > 0 {
                let name: String = after.chars().take(length).collect();
                end += 1 + name.len();
                target = Some(name);
            }
        }
    }
    Some((
        NorlogeRef {
            raw: message[base_end - base.len()..end].to_owned(),
            kind,
            target,
            indice,
        },
        end,
    ))
}

fn superscript(character: char) -> Option<u8> {
    match character {
        '¹' => Some(1),
        '²' => Some(2),
        '³' => Some(3),
        '⁴' => Some(4),
        '⁵' => Some(5),
        '⁶' => Some(6),
        '⁷' => Some(7),
        '⁸' => Some(8),
        '⁹' => Some(9),
        _ => None,
    }
}

fn split_trailing_punctuation(value: &str) -> (&str, &str) {
    let mut end = value.len();
    while end > 0 {
        let character = value[..end].chars().next_back().unwrap_or(' ');
        let should_trim = match character {
            '.' | ',' | '!' | '?' | ';' | ':' => true,
            ')' => value[..end].matches(')').count() > value[..end].matches('(').count(),
            ']' => value[..end].matches(']').count() > value[..end].matches('[').count(),
            '}' => value[..end].matches('}').count() > value[..end].matches('{').count(),
            _ => false,
        };
        if !should_trim {
            break;
        }
        end -= character.len_utf8();
    }
    (&value[..end], &value[end..])
}

fn url_label(href: &str) -> String {
    if let Ok(url) = url::Url::parse(href) {
        let host = url.host_str().unwrap_or_default();
        let path = url.path();
        let label = if path.is_empty() || path == "/" {
            host.to_owned()
        } else {
            format!("{host}{path}")
        };
        if label.graphemes(true).count() > 44 {
            let mut shortened: String = label.graphemes(true).take(43).collect();
            shortened.push('…');
            shortened
        } else if label.is_empty() {
            href.to_owned()
        } else {
            label
        }
    } else {
        href.to_owned()
    }
}

pub(crate) fn decode_entities(value: &str) -> String {
    if !value.contains('&') {
        return value.to_owned();
    }
    let mut result = String::with_capacity(value.len());
    let mut index = 0usize;
    while index < value.len() {
        if value.as_bytes()[index] == b'&' {
            if let Some(matched) = entity_regex().find(&value[index..]) {
                if matched.start() == 0 {
                    let entity = matched.as_str();
                    match entity {
                        "&lt;" => result.push('<'),
                        "&gt;" => result.push('>'),
                        "&amp;" => result.push('&'),
                        "&quot;" => result.push('"'),
                        "&apos;" => result.push('\''),
                        _ => {
                            if let Some(decoded) = decode_numeric_entity(entity) {
                                result.push(decoded);
                            } else {
                                result.push_str(entity);
                            }
                        }
                    }
                    index += entity.len();
                    continue;
                }
            }
        }
        let character = value[index..].chars().next().unwrap_or('\u{fffd}');
        result.push(character);
        index += character.len_utf8();
    }
    result
}

fn decode_numeric_entity(entity: &str) -> Option<char> {
    let number = if entity.starts_with("&#x") || entity.starts_with("&#X") {
        u32::from_str_radix(&entity[3..entity.len() - 1], 16).ok()?
    } else if entity.starts_with("&#") {
        entity[2..entity.len() - 1].parse().ok()?
    } else {
        return None;
    };
    char::from_u32(number)
}

struct TagEntry {
    name: String,
    spoiler_id: Option<usize>,
}

fn url_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)(?:https?|ftp)://[^\s<>"']+|data:[A-Za-z0-9./:;=]+,[^\s<>"']+"#)
            .expect("valid URL regex")
    })
}

fn anchor_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?is)<a\s+[^>]*href=["']([^"']+)["'][^>]*>.*?</a>"#)
            .expect("valid anchor regex")
    })
}

fn totoz_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"\[:([^\]\r\n]+)]").expect("valid totoz regex"))
}

fn open_tag_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)<(spoiler|b|strong|i|em|s|strike|del|u|tt|code)>")
            .expect("valid open tag regex")
    })
}

fn close_tag_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)</(spoiler|b|strong|i|em|s|strike|del|u|tt|code)>")
            .expect("valid close tag regex")
    })
}

fn br_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?i)<br\s*/?>").expect("valid br regex"))
}

fn entity_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"&(?:lt|gt|amp|quot|apos|#[0-9]+|#x[0-9a-fA-F]+);").expect("valid entity regex")
    })
}

fn duck_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?i)(?:\\_|>)[oO0ô°øòó@](?:<|_/)").expect("valid duck regex"))
}

fn full_datetime_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(\d{4})-(\d{2})-(\d{2})[T# ](\d{2}):(\d{2}):(\d{2})(?:\.\d{1,3})?")
            .expect("valid full datetime regex")
    })
}

fn long_date_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(\d{2})/(\d{2})#(\d{2}):(\d{2}):(\d{2})").expect("valid long date regex")
    })
}

fn clock_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(\d{2}):(\d{2}):(\d{2})").expect("valid clock regex"))
}

fn short_clock_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(\d{2}):(\d{2})").expect("valid short clock regex"))
}

fn id_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"#(\d+)").expect("valid id regex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_formatting_and_entities() {
        let parsed = parse("Hello <b>big</b> &amp; <i>world</i>");
        assert!(parsed.tokens.iter().any(
            |token| matches!(token, Token::Text { text, style, .. } if text.contains("big") && style.bold)
        ));
        assert!(parsed.plain_text().contains("Hello big & world"));
    }

    #[test]
    fn parses_clock_references() {
        let parsed = parse("see 12:34:56@dlfp and 12:34 and #42");
        assert_eq!(parsed.norloges.len(), 3);
        assert_eq!(parsed.norloges[0].raw, "12:34:56@dlfp");
        assert_eq!(parsed.norloges[0].target.as_deref(), Some("dlfp"));
        assert_eq!(parsed.norloges[2].kind, NorlogeKind::Id(42));
    }

    #[test]
    fn parses_iso_and_indices() {
        let parsed = parse("2017-06-29T13:45:01.421^2@dlfp");
        assert_eq!(parsed.norloges.len(), 1);
        assert_eq!(parsed.norloges[0].indice, Some(2));
        assert_eq!(parsed.norloges[0].target.as_deref(), Some("dlfp"));
    }

    #[test]
    fn parses_bigorno() {
        let parsed = parse("dave< hello");
        assert!(parsed.bigornos.contains("dave"));
        assert!(parsed
            .tokens
            .iter()
            .any(|token| matches!(token, Token::Bigorno { word } if word == "dave")));
    }

    #[test]
    fn parses_bigorno_with_dot() {
        let parsed = parse("my.nick< hello");
        assert!(parsed.bigornos.contains("my.nick"));
        assert!(parsed
            .tokens
            .iter()
            .any(|token| matches!(token, Token::Bigorno { word } if word == "my.nick")));
    }

    #[test]
    fn parses_identity_bigorno_with_spaces() {
        let identities = BTreeSet::from([
            "Big Nick".to_owned(),
            "Mozilla/5.0 (X11; Linux x86_64) Firefox/127.0".to_owned(),
        ]);

        let parsed = parse_with_identities("hello Big Nick< how are you", &identities);
        assert!(parsed.has_bigorno(&identities));
        assert!(parsed
            .tokens
            .iter()
            .any(|token| matches!(token, Token::Bigorno { word } if word == "Big Nick")));

        let ua = "Mozilla/5.0 (X11; Linux x86_64) Firefox/127.0<";
        let parsed = parse_with_identities(ua, &identities);
        assert!(parsed.has_bigorno(&identities));
        assert!(parsed
            .bigornos
            .contains("Mozilla/5.0 (X11; Linux x86_64) Firefox/127.0"));
    }

    #[test]
    fn parses_bigornos_after_newline_and_tags() {
        let identities = BTreeSet::from(["Big Nick".to_owned()]);

        let parsed = parse("hello\nmy.nick< how are you");
        assert!(parsed.bigornos.contains("my.nick"));

        let parsed = parse_with_identities("<b>Big Nick<</b> hi", &identities);
        assert!(parsed.has_bigorno(&identities));
        assert!(parsed
            .tokens
            .iter()
            .any(|token| matches!(token, Token::Bigorno { word } if word == "Big Nick")));
    }

    #[test]
    fn identity_without_marker_is_not_a_bigorno() {
        let identities = BTreeSet::from(["Big Nick".to_owned()]);
        let parsed = parse_with_identities("hello Big Nick", &identities);
        assert!(!parsed.has_bigorno(&identities));
    }

    #[test]
    fn parses_urls_and_totoz() {
        let parsed = parse("look https://example.com/a. and [:good]");
        assert!(parsed.tokens.iter().any(
            |token| matches!(token, Token::Link { href, .. } if href == "https://example.com/a")
        ));
        assert!(parsed
            .tokens
            .iter()
            .any(|token| matches!(token, Token::Totoz { name } if name == "good")));
    }

    #[test]
    fn rewrites_suffix() {
        let rewritten = rewrite_bouchot_suffix("see 12:34:56 and 12:34:56@dlfp", "moules", "dlfp");
        assert_eq!(rewritten, "see 12:34:56@moules and 12:34:56");
    }

    #[test]
    fn rewrite_preserves_indices() {
        let rewritten = rewrite_bouchot_suffix("12:34:56^2@dlfp", "moules", "dlfp");
        assert_eq!(rewritten, "12:34:56^2");
        let rewritten = rewrite_bouchot_suffix("12:34:56^2", "moules", "dlfp");
        assert_eq!(rewritten, "12:34:56^2@moules");
    }
}
