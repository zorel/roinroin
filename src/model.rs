//! Core data types shared by the network layer, the conversation engine and
//! the user interface.

use std::cmp::Ordering;
use std::fmt;

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Utc};
use chrono_tz::Europe::Paris;
use serde::{Deserialize, Serialize};

/// Where a tribune comes from. A gb2c server aggregates several bouchots;
/// a direct board is read and written without any intermediary.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "lowercase")]
pub enum SourceId {
    Gb2c(String),
    Direct(String),
}

impl SourceId {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Gb2c(_) => "gb2c",
            Self::Direct(_) => "direct",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Gb2c(url) => url,
            Self::Direct(name) => name,
        }
    }

    pub fn key(&self) -> String {
        format!("{}:{}", self.kind(), self.id())
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.kind(), self.id())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TribuneKey {
    pub source: SourceId,
    pub name: String,
}

impl TribuneKey {
    pub fn new(source: SourceId, name: impl Into<String>) -> Self {
        Self {
            source,
            name: name.into(),
        }
    }

    /// Stable key used for last-read markers, hidden tribunes and UI ids.
    pub fn key(&self) -> String {
        format!("{}|{}", self.source.key(), self.name)
    }

    /// Parse a key produced by [`TribuneKey::key`].
    pub fn from_key(key: &str) -> Option<Self> {
        let (source, name) = key.split_once('|')?;
        let (kind, id) = source.split_once(':')?;
        let source = match kind {
            "gb2c" => SourceId::Gb2c(id.to_owned()),
            "direct" => SourceId::Direct(id.to_owned()),
            _ => return None,
        };
        Some(Self::new(source, name))
    }
}

impl fmt::Display for TribuneKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PostKey {
    pub tribune: TribuneKey,
    pub id: i64,
}

impl PostKey {
    pub fn new(tribune: TribuneKey, id: i64) -> Self {
        Self { tribune, id }
    }

    pub fn key(&self) -> String {
        format!("{}|{}", self.tribune.key(), self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Post {
    pub id: i64,
    pub time: String,
    pub info: String,
    pub login: String,
    pub message: String,
    pub tribune: String,
    #[serde(default)]
    pub source_label: String,
}

impl Post {
    pub fn key(&self, source: &SourceId) -> PostKey {
        PostKey::new(
            TribuneKey::new(source.clone(), self.tribune.clone()),
            self.id,
        )
    }

    pub fn time(&self) -> PostTime {
        PostTime::parse(&self.time)
    }

    pub fn display_identity(&self) -> String {
        if !self.login.trim().is_empty() {
            trim_identity(self.login.trim(), 24)
        } else {
            trim_identity(&short_browser_user_agent(&self.info), 24)
        }
    }

    /// The login or user agent used to address this post with a bigorno.
    pub fn identity(&self) -> &str {
        if !self.login.trim().is_empty() {
            self.login.trim()
        } else {
            self.info.trim()
        }
    }
}

/// A timestamp on a tribune. Bouchots use `yyyyMMddHHmmss` in Europe/Paris;
/// the parser also accepts a few common variants from non-conforming boards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTime {
    pub raw: String,
    pub naive: Option<NaiveDateTime>,
}

impl PostTime {
    pub fn parse(raw: &str) -> Self {
        let raw = raw.trim().to_owned();
        let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
        let naive = if digits.len() >= 14 {
            let value = &digits[..14];
            NaiveDate::from_ymd_opt(
                value[0..4].parse().unwrap_or(1970),
                value[4..6].parse().unwrap_or(1),
                value[6..8].parse().unwrap_or(1),
            )
            .and_then(|date| {
                date.and_hms_opt(
                    value[8..10].parse().unwrap_or(0),
                    value[10..12].parse().unwrap_or(0),
                    value[12..14].parse().unwrap_or(0),
                )
            })
        } else {
            parse_loose_datetime(&raw)
        };
        Self { raw, naive }
    }

    pub fn clock(&self) -> String {
        self.naive
            .map(|dt| format!("{:02}:{:02}:{:02}", dt.hour(), dt.minute(), dt.second()))
            .unwrap_or_else(|| self.raw.clone())
    }

    pub fn short_clock(&self) -> String {
        self.naive
            .map(|dt| format!("{:02}:{:02}", dt.hour(), dt.minute()))
            .unwrap_or_else(|| self.raw.clone())
    }

    pub fn date(&self) -> Option<NaiveDate> {
        self.naive.map(|dt| dt.date())
    }

    pub fn iso_date(&self) -> String {
        self.date()
            .map(|date| format!("{:04}-{:02}-{:02}", date.year(), date.month(), date.day()))
            .unwrap_or_default()
    }

    pub fn is_today(&self) -> bool {
        let today = Paris
            .from_utc_datetime(&Utc::now().naive_utc())
            .date_naive();
        self.date() == Some(today)
    }

    pub fn age_seconds(&self) -> Option<i64> {
        let naive = self.naive?;
        let local = Paris.from_local_datetime(&naive).single()?;
        Some((Utc::now() - local.with_timezone(&Utc)).num_seconds())
    }
}

fn parse_loose_datetime(raw: &str) -> Option<NaiveDateTime> {
    const FORMATS: &[&str] = &[
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y/%m/%d %H:%M:%S",
        "%d/%m/%Y %H:%M:%S",
    ];
    for format in FORMATS {
        if let Ok(datetime) = NaiveDateTime::parse_from_str(raw, format) {
            return Some(datetime);
        }
    }
    None
}

/// The clock reference inserted when quoting a post, matching gb3tui/gc2:
/// use the date when the post is not from today, otherwise a bare clock.
pub fn quote_clock(post: &Post) -> String {
    let time = post.time();
    if time.date().is_some() && !time.is_today() {
        format!("{} {}", time.iso_date(), time.clock())
    } else {
        time.clock()
    }
}

pub fn trim_identity(identity: &str, max_chars: usize) -> String {
    let count = identity.chars().count();
    if count <= max_chars {
        return identity.to_owned();
    }
    let mut result: String = identity.chars().take(max_chars.saturating_sub(1)).collect();
    result.push('…');
    result
}

/// Turn a browser user agent into the short label used by gc2.
pub fn short_browser_user_agent(user_agent: &str) -> String {
    let ua = user_agent.trim();
    if ua.is_empty() {
        return "coward".to_owned();
    }
    let candidates: &[(&str, &str)] = &[
        ("Edge", "Edg"),
        ("Edge", "EdgiOS"),
        ("Edge", "EdgA"),
        ("Opera", "OPR"),
        ("Opera", "Opera"),
        ("Firefox", "Firefox"),
        ("Firefox", "FxiOS"),
        ("Chrome", "Chrome"),
        ("Chrome", "CriOS"),
    ];
    for (name, token) in candidates {
        if let Some(version) = version_after(ua, token) {
            let platform = browser_platform(ua);
            return if platform.is_empty() {
                format!("{name} {version}")
            } else {
                format!("{name} {version} · {platform}")
            };
        }
    }
    if ua.contains("Safari/") {
        if let Some(position) = ua.find("Version/") {
            let version: String = ua[position + 8..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if !version.is_empty() {
                let platform = browser_platform(ua);
                return if platform.is_empty() {
                    format!("Safari {version}")
                } else {
                    format!("Safari {version} · {platform}")
                };
            }
        }
    }
    ua.to_owned()
}

fn version_after<'a>(ua: &'a str, token: &str) -> Option<&'a str> {
    let needle = format!("{token}/");
    let position = ua.find(&needle)?;
    let rest = &ua[position + needle.len()..];
    let version: &str = rest.split(|c: char| !c.is_ascii_digit()).next()?;
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

fn browser_platform(ua: &str) -> &'static str {
    if ua.contains("Android") {
        "Android"
    } else if ua.contains("iPad") {
        "iPadOS"
    } else if ua.contains("iPhone") || ua.contains("iPod") {
        "iOS"
    } else if ua.contains("Windows") {
        "Windows"
    } else if ua.contains("CrOS") {
        "ChromeOS"
    } else if ua.contains("Macintosh") || ua.contains("Mac OS X") {
        "macOS"
    } else if ua.contains("Linux") {
        "Linux"
    } else {
        ""
    }
}

/// A resolved date/time reference. Used by the conversation engine and by
/// clickable norloges in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NorlogeKind {
    Id(i64),
    FullDateTime(NaiveDateTime),
    DateClock {
        month: u32,
        day: u32,
        clock: NaiveTime,
    },
    Clock(NaiveTime),
    Short(NaiveTime),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NorlogeRef {
    pub raw: String,
    pub kind: NorlogeKind,
    pub target: Option<String>,
    pub indice: Option<u8>,
}

/// Compare two timestamps chronologically, tolerating unparsable times.
pub fn compare_posts(a: &Post, b: &Post) -> Ordering {
    let time_a = a.time();
    let time_b = b.time();
    match (time_a.naive, time_b.naive) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_compact_time() {
        let time = PostTime::parse("20240629134501");
        assert_eq!(time.clock(), "13:45:01");
        assert_eq!(time.iso_date(), "2024-06-29");
    }

    #[test]
    fn shortens_user_agents() {
        let ua = "Mozilla/5.0 (X11; Linux x86_64) Firefox/127.0";
        assert_eq!(short_browser_user_agent(ua), "Firefox 127 · Linux");
    }

    #[test]
    fn trims_with_ellipsis() {
        assert_eq!(trim_identity("abcdefghij", 5), "abcd…");
    }

    #[test]
    fn prefers_login_over_user_agent_for_identity() {
        let mut post = Post {
            id: 1,
            time: String::new(),
            info: " Mozilla/5.0 ".to_owned(),
            login: " dave ".to_owned(),
            message: String::new(),
            tribune: String::new(),
            source_label: String::new(),
        };
        assert_eq!(post.identity(), "dave");
        post.login.clear();
        assert_eq!(post.identity(), "Mozilla/5.0");
    }
}
