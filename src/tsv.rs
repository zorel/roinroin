//! Tolerant parser for the two standard bouchot backends.
//!
//! Remote boards are untrusted: truncated rows, HTML error pages, absurdly
//! long lines, control characters and invalid ids are all expected. Malformed
//! rows are skipped, extra TSV columns are ignored (like gb2c), and every field
//! is bounded before it reaches the UI.

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::model::Post;

const MAX_POSTS: usize = 2_000;
const MAX_LINE_BYTES: usize = 512 * 1024;
const MAX_LINES: usize = 20_000;
const MAX_TIME_CHARS: usize = 40;
const MAX_INFO_CHARS: usize = 2_048;
const MAX_LOGIN_CHARS: usize = 512;
const MAX_MESSAGE_CHARS: usize = 65_536;

/// Parse a backend response, auto-detecting the XML and TSV formats.
pub fn parse_backend(body: &[u8], tribune: &str) -> Vec<Post> {
    let text = String::from_utf8_lossy(body);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with("<?xml") || trimmed.starts_with("<board") {
        parse_xml(&text, tribune)
    } else {
        parse_tsv(&text, tribune)
    }
}

pub fn parse_tsv(body: &str, tribune: &str) -> Vec<Post> {
    if tribune.trim().is_empty() {
        return Vec::new();
    }
    let mut posts = Vec::new();
    let mut seen_lines = 0usize;
    for raw_line in body.split('\n') {
        seen_lines += 1;
        if seen_lines > MAX_LINES {
            break;
        }
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() || line.len() > MAX_LINE_BYTES {
            continue;
        }
        if let Some(post) = parse_tsv_line(line, tribune) {
            posts.push(post);
        }
    }
    // Boards return posts oldest first. If an id is repeated, the last
    // occurrence wins, as in gb2c.
    posts.sort_by_key(|post| post.id);
    posts.dedup_by_key(|post| post.id);
    if posts.len() > MAX_POSTS {
        posts.drain(..posts.len() - MAX_POSTS);
    }
    posts
}

fn parse_tsv_line(line: &str, tribune: &str) -> Option<Post> {
    let mut fields = [""; 5];
    let mut rest = line;
    for field in fields.iter_mut().take(4) {
        let tab = rest.find('\t')?;
        *field = &rest[..tab];
        rest = &rest[tab + 1..];
    }
    // Anything after the fifth column is ignored.
    fields[4] = rest.split('\t').next().unwrap_or_default();

    let id = fields[0]
        .trim()
        .trim_start_matches('\u{feff}')
        .parse::<i64>()
        .ok()?
        .checked_abs()?;
    if id <= 0 {
        return None;
    }
    let time = sanitize(fields[1], MAX_TIME_CHARS).trim().to_owned();
    if time.is_empty() {
        return None;
    }
    let info = sanitize(fields[2], MAX_INFO_CHARS).trim().to_owned();
    let login = sanitize(fields[3], MAX_LOGIN_CHARS).trim().to_owned();
    let message = sanitize(fields[4], MAX_MESSAGE_CHARS);
    if message.is_empty() && login.is_empty() && info.is_empty() {
        return None;
    }
    Some(Post {
        id,
        time,
        info,
        login,
        message,
        tribune: tribune.to_owned(),
        source_label: String::new(),
    })
}

pub fn parse_xml(body: &str, tribune: &str) -> Vec<Post> {
    if tribune.trim().is_empty() {
        return Vec::new();
    }
    let mut reader = Reader::from_str(body);
    reader.config_mut().trim_text(false);
    let mut posts = Vec::new();
    let mut current: Option<XmlPost> = None;
    let mut text_target: Option<XmlField> = None;
    let mut buffer = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(element)) => {
                let element_name = element.name();
                let name = local_name(element_name.as_ref());
                if name == b"post" {
                    let mut post = XmlPost::default();
                    for attribute in element.attributes().flatten() {
                        let key = local_name(attribute.key.as_ref());
                        let value = decode_xml(&attribute.value);
                        match key {
                            b"id" => post.id = value.trim().parse::<i64>().ok(),
                            b"time" => post.time = value,
                            _ => {}
                        }
                    }
                    current = Some(post);
                } else if current.is_some() {
                    text_target = match name {
                        b"info" => Some(XmlField::Info),
                        b"login" => Some(XmlField::Login),
                        b"message" => Some(XmlField::Message),
                        _ => None,
                    };
                    buffer.clear();
                }
            }
            Ok(Event::Text(text)) => {
                if text_target.is_some() {
                    buffer.push_str(&decode_xml(&text.into_inner()));
                }
            }
            Ok(Event::CData(data)) => {
                if text_target.is_some() {
                    buffer.push_str(&String::from_utf8_lossy(&data.into_inner()));
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                if text_target.is_some() {
                    let name = String::from_utf8_lossy(&reference.into_inner()).into_owned();
                    let entity = format!("&{name};");
                    buffer.push_str(
                        &quick_xml::escape::unescape(&entity)
                            .map(|value| value.into_owned())
                            .unwrap_or(entity),
                    );
                }
            }
            Ok(Event::End(element)) => {
                let element_name = element.name();
                let name = local_name(element_name.as_ref());
                if name == b"post" {
                    if let Some(xml) = current.take() {
                        if let Some(post) = xml.finish(tribune) {
                            posts.push(post);
                        }
                    }
                    text_target = None;
                } else if let Some(field) = text_target.take() {
                    if let Some(xml) = current.as_mut() {
                        let value = std::mem::take(&mut buffer);
                        match field {
                            XmlField::Info => xml.info = value,
                            XmlField::Login => xml.login = value,
                            XmlField::Message => xml.message = value,
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
    }

    posts.sort_by_key(|post| post.id);
    posts.dedup_by_key(|post| post.id);
    if posts.len() > MAX_POSTS {
        posts.drain(..posts.len() - MAX_POSTS);
    }
    posts
}

fn decode_xml(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes);
    quick_xml::escape::unescape(&raw)
        .map(|value| value.into_owned())
        .unwrap_or_else(|_| raw.into_owned())
}

fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|byte| *byte == b':').next().unwrap_or(name)
}

#[derive(Default)]
struct XmlPost {
    id: Option<i64>,
    time: String,
    info: String,
    login: String,
    message: String,
}

impl XmlPost {
    fn finish(self, tribune: &str) -> Option<Post> {
        let id = self.id?.abs();
        if id <= 0 {
            return None;
        }
        let time = sanitize(&self.time, MAX_TIME_CHARS).trim().to_owned();
        if time.is_empty() {
            return None;
        }
        let info = sanitize(&self.info, MAX_INFO_CHARS).trim().to_owned();
        let login = sanitize(&self.login, MAX_LOGIN_CHARS).trim().to_owned();
        let message = sanitize(&self.message, MAX_MESSAGE_CHARS);
        if message.is_empty() && login.is_empty() && info.is_empty() {
            return None;
        }
        Some(Post {
            id,
            time,
            info,
            login,
            message,
            tribune: tribune.to_owned(),
            source_label: String::new(),
        })
    }
}

enum XmlField {
    Info,
    Login,
    Message,
}

/// Replace tabs/newlines by spaces, drop other control characters, and bound
/// the result in characters (not bytes) without splitting UTF-8 sequences.
fn sanitize(value: &str, max_chars: usize) -> String {
    let mut result = String::with_capacity(value.len().min(max_chars));
    for character in value.chars().take(max_chars) {
        match character {
            '\t' | '\n' | '\r' => result.push(' '),
            character if character.is_control() => {}
            character => result.push(character),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tsv_rows() {
        let body = "12\t20240629134501\tdave\t\tHello\tignored\n\
                    bad\trow\n\
                    13\t20240629134502\tbob\tbob\tHi\n";
        let posts = parse_tsv(body, "devnewton");
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].id, 12);
        assert_eq!(posts[0].message, "Hello");
        assert_eq!(posts[1].login, "bob");
    }

    #[test]
    fn sanitizes_control_characters() {
        let body = "1\t20240629134501\ta\u{0007}b\t\tline\u{0001}\n";
        let posts = parse_tsv(body, "x");
        assert_eq!(posts[0].info, "ab");
        assert_eq!(posts[0].message, "line");
    }

    #[test]
    fn parses_xml_board() {
        let body = r#"<?xml version="1.0" encoding="UTF-8"?>
            <board site="https://example.com" timezone="Europe/Paris">
              <post id="42" time="20240629134501">
                <info>dave</info>
                <message>Hello &lt;b&gt;world&lt;/b&gt;</message>
                <login/>
              </post>
            </board>"#;
        let posts = parse_backend(body.as_bytes(), "x");
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].message, "Hello <b>world</b>");
    }
}
