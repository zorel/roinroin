//! HTTP, SSE and backend clients for gb2c and direct bouchots.
//!
//! Everything here is plain async Rust. The UI turns each operation into an
//! iced `Task` or `Subscription`; no blocking call is made on the GUI thread.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use bytes::BytesMut;
use futures::{SinkExt, Stream, StreamExt};
use quick_xml::events::Event as XmlEvent;
use quick_xml::Reader;
use regex::Regex;
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use crate::config::{Board, LinuxfrSession};
use crate::model::Post;
use crate::tsv;

pub const USER_AGENT: &str = concat!("roinroin/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Default)]
pub struct ServerInfo {
    pub tribunes: Vec<String>,
    pub blocked: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Totoz {
    pub name: String,
    pub image: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Emoji {
    pub name: String,
    pub characters: String,
}

/// Metadata fetched for a URL found in a post: the page title for HTML, the
/// raw bytes for an image.
#[derive(Debug, Clone)]
pub enum LinkMetadata {
    Title(String),
    Image(Vec<u8>),
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(90))
        .user_agent(USER_AGENT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub const DEBUG_HTTP_ENV: &str = "ROINROIN_DEBUG_HTTP";

fn debug_http_enabled() -> bool {
    std::env::var_os(DEBUG_HTTP_ENV)
        .map(|value| {
            let value = value.to_string_lossy();
            !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
        })
        .unwrap_or(false)
}

/// Print the status line and headers of an HTTP response to stdout when
/// `--debug-http` is enabled. Bodies are never printed: they can be large and
/// may contain private posts.
fn log_http_response(response: &reqwest::Response) {
    if !debug_http_enabled() {
        return;
    }
    println!(
        "[http] {:?} {} {}",
        response.version(),
        response.status(),
        response.url()
    );
    for (name, value) in response.headers() {
        println!(
            "[http] {name}: {}",
            value.to_str().unwrap_or("<valeur non UTF-8>")
        );
    }
    println!("[http]");
}

fn trim_url(url: &str) -> &str {
    url.trim_end_matches('/')
}

fn api_url(base: &str, path: &str) -> String {
    format!("{}{}", trim_url(base), path)
}

pub async fn fetch_server_info(
    client: &reqwest::Client,
    base_url: &str,
) -> Result<ServerInfo, String> {
    let list_url = api_url(base_url, "/gb2c/list");
    let config_url = api_url(base_url, "/gb2c/config");
    let (list, config) = tokio::join!(
        client
            .get(&list_url)
            .timeout(Duration::from_secs(20))
            .send(),
        client
            .get(&config_url)
            .timeout(Duration::from_secs(20))
            .send()
    );

    let response = list.map_err(|error| format!("gb2c injoignable : {error}"))?;
    log_http_response(&response);
    if !response.status().is_success() {
        return Err(format!("gb2c /list : HTTP {}", response.status()));
    }
    let text =
        String::from_utf8_lossy(&read_bounded(response, 4 * 1024 * 1024).await?).into_owned();
    let mut tribunes: Vec<String> =
        serde_json::from_str(&text).map_err(|error| format!("gb2c /list illisible : {error}"))?;
    tribunes.retain(|name| {
        let name = name.trim();
        !name.is_empty() && !name.chars().any(|c| c == '|' || c.is_control())
    });
    tribunes.sort();
    tribunes.dedup();

    let mut blocked = BTreeSet::new();
    if let Ok(response) = config {
        log_http_response(&response);
        if response.status().is_success() {
            if let Ok(body) = read_bounded(response, 256 * 1024).await {
                let text = String::from_utf8_lossy(&body);
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(array) = value.get("blockedIdentities").and_then(|v| v.as_array()) {
                        for item in array {
                            if let Some(identity) = item.as_str() {
                                if !identity.trim().is_empty() {
                                    blocked.insert(identity.trim().to_owned());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(ServerInfo { tribunes, blocked })
}

pub async fn fetch_board(client: &reqwest::Client, board: &Board) -> Result<Vec<Post>, String> {
    board.validate()?;
    let response = client
        .get(&board.backend_url)
        .timeout(Duration::from_secs(25))
        .header(
            reqwest::header::ACCEPT,
            "text/tab-separated-values, application/xml, text/xml, text/plain, */*",
        )
        .send()
        .await
        .map_err(|error| format!("{} : {error}", board.name))?;
    log_http_response(&response);
    if !response.status().is_success() {
        return Err(format!("{} : HTTP {}", board.name, response.status()));
    }
    let bytes = read_bounded(response, 8 * 1024 * 1024)
        .await
        .map_err(|error| format!("{} : {error}", board.name))?;
    Ok(tsv::parse_backend(&bytes, &board.name))
}

pub async fn post_gb2c(
    client: &reqwest::Client,
    base_url: &str,
    tribune: &str,
    info: &str,
    message: &str,
    access_token: Option<&str>,
) -> Result<(), String> {
    let message = message.trim();
    if message.is_empty() {
        return Err("message vide".into());
    }
    if message.chars().count() > crate::config::MAX_MESSAGE_LENGTH {
        return Err("message trop long".into());
    }
    let mut request = client.post(api_url(base_url, "/gb2c/post")).form(&[
        ("tribune", tribune),
        ("info", info),
        ("message", message),
    ]);
    if let Some(token) = access_token.filter(|token| !token.trim().is_empty()) {
        request = request.header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", token.trim()),
        );
    }
    let response = request
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| format!("envoi impossible : {error}"))?;
    log_http_response(&response);
    if !response.status().is_success() {
        return Err(format!("gb2c : HTTP {}", response.status()));
    }
    Ok(())
}

pub async fn post_board(
    client: &reqwest::Client,
    board: &Board,
    info: &str,
    message: &str,
    access_token: Option<&str>,
) -> Result<(), String> {
    board.validate()?;
    let message = message.trim();
    if message.is_empty() {
        return Err("message vide".into());
    }
    if message.chars().count() > crate::config::MAX_MESSAGE_LENGTH {
        return Err("message trop long".into());
    }
    let identity = info.replace(['\r', '\n'], " ").trim().to_owned();
    let mut form = vec![(board.post_field.as_str(), message)];
    if !board.info_field.is_empty() {
        form.push((board.info_field.as_str(), identity.as_str()));
    }
    let user_agent = if board.info_field.is_empty() {
        // HTTP header values are ASCII. Keep a readable fallback for accented
        // nicknames instead of letting reqwest reject the request.
        identity
            .chars()
            .map(|character| {
                if character.is_ascii() && !character.is_control() {
                    character
                } else {
                    '?'
                }
            })
            .collect::<String>()
    } else {
        USER_AGENT.to_owned()
    };
    let mut request = client
        .post(&board.post_url)
        .header(reqwest::header::USER_AGENT, &user_agent)
        .form(&form);
    if board.requires_oauth() {
        let token = access_token
            .filter(|token| !token.trim().is_empty())
            .ok_or_else(|| format!("{} demande un jeton LinuxFR", board.name))?;
        request = request.header(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", token.trim()),
        );
    }
    let response = request
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|error| format!("envoi vers {} impossible : {error}", board.name))?;
    log_http_response(&response);
    if !response.status().is_success() {
        return Err(format!("{} : HTTP {}", board.name, response.status()));
    }
    Ok(())
}

pub async fn search_totoz(
    client: &reqwest::Client,
    server: &str,
    terms: &str,
) -> Result<Vec<Totoz>, String> {
    let base =
        crate::config::normalize_url(server).ok_or_else(|| "serveur totoz invalide".to_owned())?;
    let mut url = url::Url::parse(&format!("{}/search.xml", base.trim_end_matches('/')))
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut().append_pair("terms", terms);
    let response = client
        .get(url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| format!("totoz : {error}"))?;
    log_http_response(&response);
    if !response.status().is_success() {
        return Err(format!("totoz : HTTP {}", response.status()));
    }
    let bytes = read_bounded(response, 2 * 1024 * 1024).await?;
    let names = parse_totoz_xml(&bytes)?;
    let mut results = Vec::with_capacity(names.len());
    for name in names {
        if let Some(image) = totoz_image_url(&base, &name) {
            results.push(Totoz { name, image });
        }
    }
    Ok(results)
}

pub async fn search_totoz_gb2c(
    client: &reqwest::Client,
    base_url: &str,
    terms: &str,
) -> Result<Vec<Totoz>, String> {
    let mut url = url::Url::parse(&api_url(base_url, "/gb2c/totoz/search"))
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut().append_pair("terms", terms);
    let response = client
        .get(url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| format!("totoz gb2c : {error}"))?;
    log_http_response(&response);
    if !response.status().is_success() {
        return Err(format!("totoz gb2c : HTTP {}", response.status()));
    }
    let text =
        String::from_utf8_lossy(&read_bounded(response, 2 * 1024 * 1024).await?).into_owned();
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|error| format!("totoz gb2c : {error}"))?;
    let mut results = Vec::new();
    if let Some(array) = value.get("totozes").and_then(|value| value.as_array()) {
        for item in array {
            let name = item
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim();
            if name.is_empty() {
                continue;
            }
            let image = item
                .get("image")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim();
            if image.is_empty() {
                continue;
            }
            results.push(Totoz {
                name: name.to_owned(),
                image: image.to_owned(),
            });
        }
    }
    Ok(results)
}

fn parse_totoz_xml(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut reader = Reader::from_reader(bytes);
    let mut names = Vec::new();
    let mut in_name = false;
    let mut buffer = String::new();
    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(element)) => {
                in_name = element.name().as_ref().ends_with(b"name");
                buffer.clear();
            }
            Ok(XmlEvent::Text(text)) => {
                if in_name {
                    buffer.push_str(&String::from_utf8_lossy(&text.into_inner()));
                }
            }
            Ok(XmlEvent::End(element)) => {
                if element.name().as_ref().ends_with(b"name") && in_name {
                    let name = buffer.trim();
                    if !name.is_empty() && name.len() <= 200 {
                        names.push(name.to_owned());
                    }
                    in_name = false;
                }
            }
            Ok(XmlEvent::Eof) => break,
            Err(error) => return Err(format!("XML totoz : {error}")),
            _ => {}
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

pub fn totoz_image_url(server: &str, name: &str) -> Option<String> {
    let mut url = url::Url::parse(&format!("{}/img", server.trim_end_matches('/'))).ok()?;
    url.path_segments_mut().ok()?.push(name);
    Some(url.to_string())
}

pub async fn fetch_totoz_image(
    client: &reqwest::Client,
    server: &str,
    name: &str,
) -> Result<Vec<u8>, String> {
    let url = totoz_image_url(server, name).ok_or_else(|| "URL totoz invalide".to_owned())?;
    let response = client
        .get(url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| format!("totoz : {error}"))?;
    log_http_response(&response);
    if !response.status().is_success() {
        return Err(format!("totoz : HTTP {}", response.status()));
    }
    read_bounded(response, 2 * 1024 * 1024).await
}

const MAX_LINK_HTML_BYTES: usize = 512 * 1024;
const MAX_LINK_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_LINK_TITLE_CHARS: usize = 200;

pub async fn fetch_link_metadata(
    client: &reqwest::Client,
    href: &str,
) -> Result<LinkMetadata, String> {
    let url = url::Url::parse(href).map_err(|error| format!("lien : {error}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("lien non pris en charge".into());
    }
    let response = client
        .get(url)
        .timeout(Duration::from_secs(15))
        .header(
            reqwest::header::ACCEPT,
            "text/html,application/xhtml+xml,image/*;q=0.9,*/*;q=0.5",
        )
        .send()
        .await
        .map_err(|error| format!("lien : {error}"))?;

    log_http_response(&response);
    let status = response.status();
    if matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
    ) {
        return Err("authentification requise".into());
    }
    if !status.is_success() {
        return Err(format!("lien : HTTP {status}"));
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if content_type.contains("text/html") || content_type.contains("application/xhtml+xml") {
        // The title lives in <head>, so a truncated body is enough and avoids
        // failing on pages larger than the cap.
        let bytes = read_prefix(response, MAX_LINK_HTML_BYTES).await?;
        let html = String::from_utf8_lossy(&bytes);
        let title = html_title(&html).ok_or_else(|| "titre introuvable".to_owned())?;
        return Ok(LinkMetadata::Title(title));
    }

    if content_type.starts_with("image/") {
        let bytes = read_bounded(response, MAX_LINK_IMAGE_BYTES).await?;
        return Ok(LinkMetadata::Image(bytes));
    }

    // Servers often omit or mislabel the content type. Sniff the body so an
    // image is still recognized, then fall back to an HTML title.
    let bytes = read_bounded(response, MAX_LINK_IMAGE_BYTES).await?;
    if ::image::guess_format(&bytes).is_ok() {
        return Ok(LinkMetadata::Image(bytes));
    }
    if let Some(title) = html_title(&String::from_utf8_lossy(&bytes)) {
        return Ok(LinkMetadata::Title(title));
    }
    Err("contenu non pris en charge".into())
}

fn html_title(html: &str) -> Option<String> {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    let regex = REGEX.get_or_init(|| {
        Regex::new(r"(?is)<title[^>]*>(.*?)</title\s*>").expect("valid title regex")
    });
    let raw = regex.captures(html)?.get(1)?.as_str();
    let decoded = crate::message::decode_entities(raw);
    let title = decoded.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return None;
    }
    let mut graphemes = title.graphemes(true);
    let shortened: String = graphemes.by_ref().take(MAX_LINK_TITLE_CHARS).collect();
    Some(if graphemes.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    })
}

pub async fn search_emoji(
    client: &reqwest::Client,
    base_url: &str,
    terms: &str,
) -> Result<Vec<Emoji>, String> {
    let mut url = url::Url::parse(&api_url(base_url, "/gb2c/emoji/search"))
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut().append_pair("terms", terms);
    let response = client
        .get(url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| format!("emoji : {error}"))?;
    log_http_response(&response);
    if !response.status().is_success() {
        return Err(format!("emoji : HTTP {}", response.status()));
    }
    let text = String::from_utf8_lossy(&read_bounded(response, 1024 * 1024).await?).into_owned();
    let items: Vec<Emoji> = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    Ok(items
        .into_iter()
        .filter(|item| !item.characters.is_empty() && !item.name.is_empty())
        .take(100)
        .collect())
}

#[derive(Debug, Deserialize)]
struct LinuxfrRefresh {
    access_token: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    expires_in: i64,
}

pub async fn refresh_linuxfr(
    client: &reqwest::Client,
    base_url: &str,
    refresh_token: &str,
) -> Result<LinuxfrSession, String> {
    let response = client
        .post(api_url(base_url, "/gb2c/linuxfr/refresh"))
        .form(&[("refresh_token", refresh_token)])
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|error| format!("linuxfr : {error}"))?;
    log_http_response(&response);
    if !response.status().is_success() {
        return Err(format!("linuxfr : HTTP {}", response.status()));
    }
    let data: LinuxfrRefresh = response
        .json()
        .await
        .map_err(|error| format!("linuxfr : {error}"))?;
    if data.access_token.trim().is_empty() {
        return Err("linuxfr : jeton vide".into());
    }
    Ok(LinuxfrSession {
        access_token: data.access_token,
        refresh_token: if data.refresh_token.is_empty() {
            refresh_token.to_owned()
        } else {
            data.refresh_token
        },
        expires_at: chrono::Utc::now().timestamp() + data.expires_in.max(60),
        login: String::new(),
    })
}

async fn read_prefix(response: reqwest::Response, max_bytes: usize) -> Result<Vec<u8>, String> {
    let mut stream = response.bytes_stream();
    let mut output = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.to_string())?;
        let remaining = max_bytes.saturating_sub(output.len());
        if chunk.len() >= remaining {
            output.extend_from_slice(&chunk[..remaining]);
            break;
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

async fn read_bounded(response: reqwest::Response, max_bytes: usize) -> Result<Vec<u8>, String> {
    if let Some(length) = response.content_length() {
        if length as usize > max_bytes {
            return Err("réponse trop volumineuse".into());
        }
    }
    let mut stream = response.bytes_stream();
    let mut output = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| error.to_string())?;
        if output.len() + chunk.len() > max_bytes {
            return Err("réponse trop volumineuse".into());
        }
        output.extend_from_slice(&chunk);
    }
    Ok(output)
}

// ---------------------------------------------------------------------------
// Server-sent events

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SseConfig {
    pub server_url: String,
    pub filter: Vec<String>,
    /// Incremented by the UI to force a fresh connection.
    pub epoch: u64,
}

#[derive(Debug, Clone)]
pub enum SseEvent {
    Connected,
    /// A connection attempt failed. Normal stream closes are not reported:
    /// gb2c closes every poll after an hour, and proxy/NAT idle timeouts are
    /// routine, so flagging them made the status blink red for no reason.
    Disconnected(String),
    /// Posts parsed from one network chunk. A reconnecting stream replays the
    /// whole history, so delivering one message per post made the UI rebuild
    /// its entire widget tree thousands of times in a row. Grouping by chunk
    /// keeps the per-post parsing but collapses the rendering work.
    Posts(Vec<Post>),
}

/// gb2c sends no heartbeat on a quiet tribune. Stop waiting after this long so
/// a half-open connection (proxy idle timeout, NAT expiry, suspended laptop)
/// is replaced instead of leaving the UI connected to a stream that will never
/// deliver anything again. The bound is close to common one-minute proxy idle
/// timeouts: a clean proxy close ends the stream by itself, while a silently
/// dropped connection is recovered shortly after.
const SSE_IDLE_TIMEOUT: Duration = Duration::from_secs(75);

/// A stream that stayed open this long is healthy even when it ends without an
/// error (gb2c's one-hour limit or an idle reconnect). Such closes keep the
/// reconnect delay at its minimum instead of growing the backoff.
const SSE_HEALTHY_AFTER: Duration = Duration::from_secs(10);

/// Build the long-lived gb2c poll stream. gb2c closes each connection after
/// one hour, so the loop transparently reconnects with exponential backoff.
pub fn sse_stream(config: SseConfig) -> impl Stream<Item = SseEvent> {
    sse_stream_with(
        config,
        SSE_IDLE_TIMEOUT,
        SSE_HEALTHY_AFTER,
        Duration::from_secs(1),
    )
}

fn sse_stream_with(
    config: SseConfig,
    idle_timeout: Duration,
    healthy_after: Duration,
    retry_delay: Duration,
) -> impl Stream<Item = SseEvent> {
    iced::stream::channel(512, move |mut output| async move {
        let client = client();
        let mut attempt = 0u32;
        // gb2c replays the latest posts of every filtered tribune on each
        // connection. Remember the highest id already delivered so a normal
        // reconnect only hands genuinely new posts to the UI.
        let mut seen = BTreeMap::<String, i64>::new();
        loop {
            let started = Instant::now();
            match poll_once(&client, &config, &mut output, &mut seen, idle_timeout).await {
                Ok(()) if started.elapsed() >= healthy_after => {
                    // Server-side one-hour close or deliberate idle reconnect:
                    // reconnect quickly and keep the last known UI state.
                    attempt = 0;
                }
                Ok(()) => {
                    // The stream opened and closed almost immediately. Treat
                    // it as a failure so a misbehaving server cannot keep the
                    // client in a one-second reconnect loop.
                    attempt = (attempt + 1).min(6);
                    if output
                        .send(SseEvent::Disconnected("flux fermé, reconnexion".into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    attempt = (attempt + 1).min(6);
                    if output.send(SseEvent::Disconnected(error)).await.is_err() {
                        break;
                    }
                }
            }
            let delay = match attempt {
                0 => retry_delay,
                1 => retry_delay * 2,
                2 => retry_delay * 4,
                3 => retry_delay * 8,
                4 => retry_delay * 16,
                _ => retry_delay * 30,
            };
            tokio::time::sleep(delay).await;
        }
    })
}

async fn poll_once(
    client: &reqwest::Client,
    config: &SseConfig,
    output: &mut futures::channel::mpsc::Sender<SseEvent>,
    seen: &mut BTreeMap<String, i64>,
    idle_timeout: Duration,
) -> Result<(), String> {
    let mut url = url::Url::parse(&api_url(&config.server_url, "/gb2c/poll"))
        .map_err(|error| format!("URL gb2c invalide : {error}"))?;
    if !config.filter.is_empty() {
        url.query_pairs_mut()
            .append_pair("filter", &config.filter.join(","));
    }
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .header(reqwest::header::CACHE_CONTROL, "no-cache")
        .send()
        .await
        .map_err(|error| format!("connexion gb2c : {error}"))?;
    log_http_response(&response);
    if !response.status().is_success() {
        return Err(format!("gb2c /poll : HTTP {}", response.status()));
    }
    output
        .send(SseEvent::Connected)
        .await
        .map_err(|_| "subscription fermée".to_owned())?;

    let mut stream = response.bytes_stream();
    let mut buffer = BytesMut::with_capacity(16 * 1024);
    let mut data = String::new();
    let mut posts = Vec::new();
    loop {
        let chunk = match tokio::time::timeout(idle_timeout, stream.next()).await {
            Ok(Some(chunk)) => chunk.map_err(|error| format!("flux gb2c : {error}"))?,
            Ok(None) => break,
            // No event and no comment arrived for a long time. Reconnect
            // rather than waiting forever on a connection a middlebox may
            // already have discarded.
            Err(_) => break,
        };
        buffer.extend_from_slice(&chunk);
        if buffer.len() > 4 * 1024 * 1024 {
            return Err("flux gb2c trop volumineux".into());
        }
        while let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
            let line = buffer.split_to(position + 1);
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if !data.is_empty() {
                    push_new_post(&data, seen, &mut posts);
                    data.clear();
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            if let Some(value) = line.strip_prefix("data:") {
                let value = value.strip_prefix(' ').unwrap_or(value);
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value);
            } else if let Some(value) = line.strip_prefix("event:") {
                let event = value.trim();
                if event != "message" && !event.is_empty() {
                    // gb2c only sends unnamed messages. Ignore anything else
                    // rather than letting a future extension break parsing.
                    data.clear();
                }
            }
        }
        if !posts.is_empty() {
            output
                .send(SseEvent::Posts(std::mem::take(&mut posts)))
                .await
                .map_err(|_| "subscription fermée".to_owned())?;
        }
    }
    // The server may have closed right after the last data line without a
    // trailing blank line; do not silently discard that final event.
    if !data.is_empty() {
        push_new_post(&data, seen, &mut posts);
    }
    if !posts.is_empty() {
        output
            .send(SseEvent::Posts(posts))
            .await
            .map_err(|_| "subscription fermée".to_owned())?;
    }
    Ok(())
}

/// Remember only posts that have not been delivered yet. gb2c ignores resume
/// offsets and replays its whole retained history on every connection; without
/// this filter a reconnect re-parsed and re-ingested up to hundreds of posts
/// per tribune, rebuilding the conversation analysis for nothing.
fn push_new_post(data: &str, seen: &mut BTreeMap<String, i64>, posts: &mut Vec<Post>) {
    let Some(post) = parse_post_json(data) else {
        return;
    };
    let previous = seen.get(&post.tribune).copied().unwrap_or(0);
    if post.id > previous {
        seen.insert(post.tribune.clone(), post.id);
        posts.push(post);
    }
}

pub fn parse_post_json(data: &str) -> Option<Post> {
    let value: serde_json::Value = serde_json::from_str(data).ok()?;
    let object = value.as_object()?;
    let id = object
        .get("id")
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))?;
    if id <= 0 {
        return None;
    }
    let tribune = object
        .get("tribune")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_owned();
    if tribune.is_empty() {
        return None;
    }
    Some(Post {
        id,
        time: object
            .get("time")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned(),
        info: object
            .get("info")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned(),
        login: object
            .get("login")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned(),
        message: object
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_owned(),
        tribune,
        source_label: String::new(),
    })
}

/// A small list used when no gb2c server is available for emoji search.
pub fn builtin_emoji() -> Vec<Emoji> {
    const EMOJI: &[(&str, &str)] = &[
        ("smile", "😄"),
        ("grin", "😁"),
        ("joy", "😂"),
        ("rofl", "🤣"),
        ("wink", "😉"),
        ("heart eyes", "😍"),
        ("kiss", "😘"),
        ("thinking", "🤔"),
        ("neutral", "😐"),
        ("sad", "🙁"),
        ("cry", "😢"),
        ("angry", "😠"),
        ("rage", "😡"),
        ("scream", "😱"),
        ("cool", "😎"),
        ("nerd", "🤓"),
        ("sleep", "😴"),
        ("sick", "🤒"),
        ("wave", "👋"),
        ("thumbs up", "👍"),
        ("thumbs down", "👎"),
        ("ok", "👌"),
        ("clap", "👏"),
        ("pray", "🙏"),
        ("muscle", "💪"),
        ("point right", "👉"),
        ("point left", "👈"),
        ("point up", "👆"),
        ("point down", "👇"),
        ("heart", "❤️"),
        ("broken heart", "💔"),
        ("fire", "🔥"),
        ("star", "⭐"),
        ("sparkles", "✨"),
        ("boom", "💥"),
        ("zap", "⚡"),
        ("sun", "☀️"),
        ("moon", "🌙"),
        ("cloud", "☁️"),
        ("rain", "🌧️"),
        ("snow", "❄️"),
        ("rainbow", "🌈"),
        ("duck", "🦆"),
        ("cat", "🐱"),
        ("dog", "🐶"),
        ("mouse", "🐭"),
        ("hamster", "🐹"),
        ("rabbit", "🐰"),
        ("fox", "🦊"),
        ("bear", "🐻"),
        ("panda", "🐼"),
        ("koala", "🐨"),
        ("tiger", "🐯"),
        ("lion", "🦁"),
        ("cow", "🐮"),
        ("pig", "🐷"),
        ("frog", "🐸"),
        ("monkey", "🐵"),
        ("chicken", "🐔"),
        ("penguin", "🐧"),
        ("bird", "🐦"),
        ("fish", "🐟"),
        ("whale", "🐳"),
        ("octopus", "🐙"),
        ("bug", "🐛"),
        ("bee", "🐝"),
        ("snail", "🐌"),
        ("pizza", "🍕"),
        ("burger", "🍔"),
        ("fries", "🍟"),
        ("taco", "🌮"),
        ("sushi", "🍣"),
        ("cake", "🍰"),
        ("cookie", "🍪"),
        ("chocolate", "🍫"),
        ("coffee", "☕"),
        ("tea", "🍵"),
        ("beer", "🍺"),
        ("wine", "🍷"),
        ("cocktail", "🍸"),
        ("apple", "🍎"),
        ("banana", "🍌"),
        ("cherry", "🍒"),
        ("strawberry", "🍓"),
        ("avocado", "🥑"),
        ("croissant", "🥐"),
        ("baguette", "🥖"),
        ("cheese", "🧀"),
        ("egg", "🥚"),
        ("rocket", "🚀"),
        ("computer", "💻"),
        ("phone", "📱"),
        ("keyboard", "⌨️"),
        ("mail", "✉️"),
        ("package", "📦"),
        ("book", "📖"),
        ("pencil", "✏️"),
        ("scissors", "✂️"),
        ("wrench", "🔧"),
        ("hammer", "🔨"),
        ("gear", "⚙️"),
        ("link", "🔗"),
        ("lock", "🔒"),
        ("key", "🔑"),
        ("bell", "🔔"),
        ("megaphone", "📢"),
        ("check", "✅"),
        ("cross", "❌"),
        ("warning", "⚠️"),
        ("question", "❓"),
        ("exclamation", "❗"),
        ("no entry", "⛔"),
        ("recycle", "♻️"),
        ("trophy", "🏆"),
        ("medal", "🏅"),
        ("dice", "🎲"),
        ("music", "🎵"),
        ("guitar", "🎸"),
        ("headphones", "🎧"),
        ("camera", "📷"),
        ("tv", "📺"),
        ("clock", "🕐"),
        ("alarm", "⏰"),
        ("hourglass", "⌛"),
        ("calendar", "📅"),
        ("pin", "📌"),
        ("paperclip", "📎"),
        ("trash", "🗑️"),
        ("flag fr", "🇫🇷"),
        ("flag eu", "🇪🇺"),
        ("skull", "💀"),
        ("alien", "👽"),
        ("ghost", "👻"),
        ("robot", "🤖"),
        ("poop", "💩"),
        ("100", "💯"),
        ("troll", "🧌"),
    ];
    EMOJI
        .iter()
        .map(|(name, characters)| Emoji {
            name: (*name).to_owned(),
            characters: (*characters).to_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_html_titles() {
        assert_eq!(
            html_title("<html><head><title> Hello &amp; world </title></head></html>").as_deref(),
            Some("Hello & world")
        );
        assert_eq!(
            html_title("<TITLE>\n  spaced\n title \n</TITLE>").as_deref(),
            Some("spaced title")
        );
        assert_eq!(html_title("<title>   </title>"), None);
    }

    #[test]
    fn parses_poll_payload() {
        let post = parse_post_json(
            r#"{"id":42,"time":"20240629134501","info":"dave","login":"","message":"hi","tribune":"t"}"#,
        )
        .unwrap();
        assert_eq!(post.id, 42);
        assert_eq!(post.tribune, "t");
    }

    #[test]
    fn rejects_bad_poll_payloads() {
        assert!(parse_post_json("{}").is_none());
        assert!(parse_post_json(r#"{"id":0,"tribune":"t"}"#).is_none());
    }

    #[test]
    fn parses_totoz_xml() {
        let body = br#"<?xml version="1.0" encoding="UTF-8"?>
            <totozes><totoz><name>good</name></totoz><totoz><name>duck</name></totoz></totozes>"#;
        let names = parse_totoz_xml(body).unwrap();
        assert_eq!(names, vec!["duck".to_owned(), "good".to_owned()]);
    }

    #[test]
    fn builtin_emoji_has_duck() {
        assert!(builtin_emoji().iter().any(|item| item.characters == "🦆"));
    }

    async fn one_request(response: &'static [u8]) -> (String, tokio::task::JoinHandle<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("addr");
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut buffer = vec![0u8; 8192];
            let count = socket.read(&mut buffer).await.expect("read");
            let request = String::from_utf8_lossy(&buffer[..count]).into_owned();
            socket.write_all(response).await.expect("write");
            request
        });
        (format!("http://{address}"), handle)
    }

    #[tokio::test]
    async fn gb2c_post_sends_form_and_token() {
        let (base, handle) = one_request(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
        let client = client();
        post_gb2c(&client, &base, "trib", "dave", "hello world", Some("tok"))
            .await
            .expect("post");
        let request = handle.await.expect("server");
        assert!(request.starts_with("POST /gb2c/post"));
        assert!(
            request.contains("authorization: Bearer tok")
                || request.contains("Authorization: Bearer tok")
        );
        assert!(request.contains("message=hello+world"));
        assert!(request.contains("tribune=trib"));
    }

    #[tokio::test]
    async fn direct_post_sends_form_and_user_agent() {
        let (base, handle) = one_request(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n").await;
        let client = client();
        let board = Board::new("test", format!("{base}/tsv"), format!("{base}/post"));
        post_board(&client, &board, "dave", "hello", None)
            .await
            .expect("post");
        let request = handle.await.expect("server");
        assert!(request.starts_with("POST /post"));
        assert!(request.contains("message=hello"));
        assert!(request.contains("user-agent: dave") || request.contains("User-Agent: dave"));
    }

    #[test]
    fn replay_only_keeps_posts_never_delivered() {
        let mut seen = BTreeMap::new();
        let mut posts = Vec::new();
        push_new_post(r#"{"id":1,"tribune":"a"}"#, &mut seen, &mut posts);
        push_new_post(r#"{"id":1,"tribune":"a"}"#, &mut seen, &mut posts);
        push_new_post(r#"{"id":2,"tribune":"a"}"#, &mut seen, &mut posts);
        push_new_post(r#"{"id":1,"tribune":"b"}"#, &mut seen, &mut posts);
        assert_eq!(
            posts
                .iter()
                .map(|post| (post.tribune.as_str(), post.id))
                .collect::<Vec<_>>(),
            vec![("a", 1), ("a", 2), ("b", 1)]
        );
    }

    #[tokio::test]
    async fn idle_stream_reconnects_and_skips_replayed_posts() {
        use futures::StreamExt as _;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        const EVENT_STREAM_HEADER: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n";
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("addr");
        let server = tokio::spawn(async move {
            // Keep the first socket open: the client must notice the silence
            // and reconnect on its own.
            let mut held = Vec::new();
            for body in [
                format!(
                    "{EVENT_STREAM_HEADER}data: {{\"id\":1,\"tribune\":\"t\"}}\n\n"
                ),
                format!(
                    "{EVENT_STREAM_HEADER}data: {{\"id\":1,\"tribune\":\"t\"}}\n\ndata: {{\"id\":2,\"tribune\":\"t\"}}\n\n"
                ),
            ] {
                let (mut socket, _) = listener.accept().await.expect("accept");
                let mut request = [0u8; 4096];
                let count = socket.read(&mut request).await.expect("read");
                assert!(count > 0, "empty request");
                socket.write_all(body.as_bytes()).await.expect("write");
                socket.flush().await.expect("flush");
                held.push(socket);
            }
        });

        let config = SseConfig {
            server_url: format!("http://{address}"),
            filter: Vec::new(),
            epoch: 0,
        };
        let mut stream = Box::pin(sse_stream_with(
            config,
            Duration::from_millis(50),
            Duration::from_millis(10),
            Duration::from_millis(10),
        ));
        let mut posts = Vec::new();
        let mut disconnected = Vec::new();
        for _ in 0..8 {
            let event = tokio::time::timeout(Duration::from_secs(2), stream.next())
                .await
                .expect("event timeout")
                .expect("event");
            match event {
                SseEvent::Connected => {}
                SseEvent::Disconnected(error) => disconnected.push(error),
                SseEvent::Posts(batch) => posts.extend(batch),
            }
            if posts.iter().any(|post| post.id == 2) {
                break;
            }
        }
        server.abort();
        assert_eq!(
            posts.iter().map(|post| post.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(disconnected.is_empty(), "{disconnected:?}");
    }
}
