//! Conversation index: norloge resolution, reply graph, blocked-post
//! propagation and the small statistics used by the header.
//!
//! gb3 references another post with its clock (`12:34:56`, optionally suffixed
//! by `@tribune`). Posts sharing the same second are disambiguated by the
//! indice suffix (`^2`, `²`, …). The graph is only built from unambiguous
//! full-clock references and id references, like gc2 and gb3tui.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{Hash, Hasher};

use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};

use crate::message::{self, ParsedMessage};
use crate::model::{compare_posts, NorlogeKind, NorlogeRef, Post, PostKey, TribuneKey};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRef {
    pub reference_index: usize,
    pub target: Option<PostKey>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PostInfo {
    pub hidden: bool,
    pub is_mine: bool,
    pub is_bigorno: bool,
    pub is_reply: bool,
    pub has_duck: bool,
    pub parents: BTreeSet<PostKey>,
    pub children: BTreeSet<PostKey>,
    pub root: Option<PostKey>,
    pub color: Option<usize>,
    pub refs: Vec<ResolvedRef>,
}

#[derive(Debug, Clone, Default)]
pub struct Analysis {
    pub posts: BTreeMap<PostKey, PostInfo>,
    pub parsed: HashMap<PostKey, ParsedMessage>,
    pub max_id: BTreeMap<TribuneKey, i64>,
    pub by_tribune: BTreeMap<TribuneKey, Vec<PostKey>>,
}

impl Analysis {
    pub fn info(&self, key: &PostKey) -> Option<&PostInfo> {
        self.posts.get(key)
    }

    pub fn parsed(&self, key: &PostKey) -> Option<&ParsedMessage> {
        self.parsed.get(key)
    }

    pub fn posts_for(&self, tribune: &TribuneKey) -> &[PostKey] {
        self.by_tribune
            .get(tribune)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn max_id_for(&self, tribune: &TribuneKey) -> i64 {
        self.max_id.get(tribune).copied().unwrap_or(0)
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TribuneStats {
    pub presence: usize,
    pub last_post: Option<NaiveDateTime>,
    pub posts_last_hour: usize,
}

pub fn analyze(
    posts: &BTreeMap<PostKey, Post>,
    blocked: &BTreeSet<String>,
    my_identities: &BTreeSet<String>,
    now: NaiveDateTime,
) -> Analysis {
    let _ = now;
    let mut analysis = Analysis::default();

    // Stable chronological order per tribune.
    let mut by_tribune: BTreeMap<TribuneKey, Vec<PostKey>> = BTreeMap::new();
    for key in posts.keys() {
        by_tribune
            .entry(key.tribune.clone())
            .or_default()
            .push(key.clone());
    }
    for (tribune, keys) in &mut by_tribune {
        keys.sort_by(|a, b| {
            let post_a = &posts[a];
            let post_b = &posts[b];
            compare_posts(post_a, post_b).then_with(|| a.id.cmp(&b.id))
        });
        let max_id = keys.iter().map(|key| key.id).max().unwrap_or(0);
        analysis.max_id.insert(tribune.clone(), max_id);
    }

    let mut by_id: HashMap<(TribuneKey, i64), PostKey> = HashMap::new();
    let mut by_clock: HashMap<(TribuneKey, NaiveDate, String), Vec<PostKey>> = HashMap::new();
    let mut by_minute: HashMap<(TribuneKey, NaiveDate, String), Vec<PostKey>> = HashMap::new();
    let mut by_name: HashMap<String, Vec<TribuneKey>> = HashMap::new();
    for key in posts.keys() {
        by_id.insert((key.tribune.clone(), key.id), key.clone());
        by_name
            .entry(key.tribune.name.clone())
            .or_default()
            .push(key.tribune.clone());
        if let Some(time) = posts[key].time().naive {
            let date = time.date();
            let clock = format!("{:02}{:02}{:02}", time.hour(), time.minute(), time.second());
            let minute = format!("{:02}{:02}", time.hour(), time.minute());
            by_clock
                .entry((key.tribune.clone(), date, clock))
                .or_default()
                .push(key.clone());
            by_minute
                .entry((key.tribune.clone(), date, minute))
                .or_default()
                .push(key.clone());
        }
    }
    for list in by_name.values_mut() {
        list.sort();
        list.dedup();
    }

    // Parse every message once, and resolve its references.
    let mut parsed_by_key: HashMap<PostKey, ParsedMessage> = HashMap::new();
    for (key, post) in posts {
        parsed_by_key.insert(
            key.clone(),
            message::parse_with_identities(&post.message, my_identities),
        );
    }

    let mut resolved_by_key: HashMap<PostKey, Vec<ResolvedRef>> = HashMap::new();
    for (key, post) in posts {
        let parsed = &parsed_by_key[key];
        let mut resolved = Vec::with_capacity(parsed.norloges.len());
        for (index, reference) in parsed.norloges.iter().enumerate() {
            let target = resolve_reference(
                post, key, reference, posts, &by_id, &by_clock, &by_minute, &by_name,
            );
            resolved.push(ResolvedRef {
                reference_index: index,
                target,
            });
        }
        resolved_by_key.insert(key.clone(), resolved);
    }

    let mut parents_by_child: HashMap<PostKey, BTreeSet<PostKey>> = HashMap::new();
    let mut children_by_parent: HashMap<PostKey, BTreeSet<PostKey>> = HashMap::new();
    for (key, resolved) in &resolved_by_key {
        let parsed = &parsed_by_key[key];
        for item in resolved {
            let Some(target) = &item.target else {
                continue;
            };
            if target == key {
                continue;
            }
            let reference = &parsed.norloges[item.reference_index];
            // Short references are useful for jumping, but too ambiguous to
            // draw reply edges. gc2 only groups by full clocks.
            let authoritative = matches!(
                reference.kind,
                NorlogeKind::FullDateTime(_) | NorlogeKind::Clock(_) | NorlogeKind::Id(_)
            );
            if !authoritative {
                continue;
            }
            parents_by_child
                .entry(key.clone())
                .or_default()
                .insert(target.clone());
            children_by_parent
                .entry(target.clone())
                .or_default()
                .insert(key.clone());
        }
    }

    // Blocked identities propagate to every direct or indirect reply.
    let mut hidden: HashSet<PostKey> = HashSet::new();
    let mut queue: Vec<PostKey> = Vec::new();
    for (key, post) in posts {
        if blocked.contains(post.login.trim()) || blocked.contains(post.info.trim()) {
            hidden.insert(key.clone());
            queue.push(key.clone());
        }
    }
    while let Some(parent) = queue.pop() {
        if let Some(children) = children_by_parent.get(&parent) {
            for child in children {
                if hidden.insert(child.clone()) {
                    queue.push(child.clone());
                }
            }
        }
    }

    let mut is_mine_by_key: HashMap<PostKey, bool> = HashMap::new();
    for (key, post) in posts {
        let mine = (!post.info.trim().is_empty() && my_identities.contains(post.info.trim()))
            || (!post.login.trim().is_empty() && my_identities.contains(post.login.trim()));
        is_mine_by_key.insert(key.clone(), mine);
    }

    let my_identity_set: BTreeSet<String> = my_identities
        .iter()
        .cloned()
        .chain(std::iter::once("moules".to_owned()))
        .collect();

    let mut root_cache: HashMap<PostKey, Option<PostKey>> = HashMap::new();
    for key in posts.keys() {
        let parsed = &parsed_by_key[key];
        let parents = parents_by_child.get(key).cloned().unwrap_or_default();
        let children = children_by_parent.get(key).cloned().unwrap_or_default();
        let is_reply = parents
            .iter()
            .any(|parent| is_mine_by_key.get(parent).copied().unwrap_or(false));
        let is_bigorno = parsed.has_bigorno(&my_identity_set);
        let is_mine = is_mine_by_key.get(key).copied().unwrap_or(false);
        let has_edges = !parents.is_empty() || !children.is_empty();
        let root = if has_edges {
            Some(root_of(key, &parents_by_child, &mut root_cache))
        } else {
            None
        };
        let color = root.as_ref().map(color_index);
        analysis.posts.insert(
            key.clone(),
            PostInfo {
                hidden: hidden.contains(key),
                is_mine,
                is_bigorno,
                is_reply,
                has_duck: parsed.has_duck,
                parents,
                children,
                root,
                color,
                refs: resolved_by_key.get(key).cloned().unwrap_or_default(),
            },
        );
    }

    analysis.parsed = parsed_by_key;
    analysis.by_tribune = by_tribune;
    analysis
}

#[allow(clippy::too_many_arguments)]
fn resolve_reference(
    post: &Post,
    key: &PostKey,
    reference: &NorlogeRef,
    posts: &BTreeMap<PostKey, Post>,
    by_id: &HashMap<(TribuneKey, i64), PostKey>,
    by_clock: &HashMap<(TribuneKey, NaiveDate, String), Vec<PostKey>>,
    by_minute: &HashMap<(TribuneKey, NaiveDate, String), Vec<PostKey>>,
    by_name: &HashMap<String, Vec<TribuneKey>>,
) -> Option<PostKey> {
    let source_tribune = &key.tribune;
    let target_tribune = if let Some(name) = &reference.target {
        if let Some(found) = by_name.get(name) {
            found
                .iter()
                .find(|tribune| tribune.source == source_tribune.source)
                .cloned()
                .or_else(|| found.first().cloned())
        } else {
            None
        }
    } else {
        Some(source_tribune.clone())
    }?;

    let post_time = post.time().naive?;
    let mut candidates: Vec<PostKey> = match &reference.kind {
        NorlogeKind::Id(id) => by_id
            .get(&(target_tribune.clone(), *id))
            .cloned()
            .into_iter()
            .collect(),
        NorlogeKind::FullDateTime(datetime) => {
            let clock = format!(
                "{:02}{:02}{:02}",
                datetime.hour(),
                datetime.minute(),
                datetime.second()
            );
            by_clock
                .get(&(target_tribune.clone(), datetime.date(), clock))
                .cloned()
                .unwrap_or_default()
        }
        NorlogeKind::DateClock { month, day, clock } => {
            let clock_key = format!(
                "{:02}{:02}{:02}",
                clock.hour(),
                clock.minute(),
                clock.second()
            );
            let mut found = Vec::new();
            for (tribune, date, candidate_clock) in by_clock.keys() {
                if tribune != &target_tribune
                    || date.month() != *month
                    || date.day() != *day
                    || candidate_clock != &clock_key
                {
                    continue;
                }
                found.extend(
                    by_clock
                        .get(&(tribune.clone(), *date, clock_key.clone()))
                        .cloned()
                        .unwrap_or_default(),
                );
            }
            sort_candidates(&mut found, posts);
            select_past_or_future(found, post_time, posts)
        }
        NorlogeKind::Clock(clock) => {
            let clock_key = format!(
                "{:02}{:02}{:02}",
                clock.hour(),
                clock.minute(),
                clock.second()
            );
            let same_day = by_clock
                .get(&(target_tribune.clone(), post_time.date(), clock_key.clone()))
                .cloned()
                .unwrap_or_default();
            if !same_day.is_empty() {
                same_day
            } else {
                let mut found = Vec::new();
                for (tribune, date, candidate_clock) in by_clock.keys() {
                    if tribune == &target_tribune && candidate_clock == &clock_key {
                        found.extend(
                            by_clock
                                .get(&(tribune.clone(), *date, clock_key.clone()))
                                .cloned()
                                .unwrap_or_default(),
                        );
                    }
                }
                sort_candidates(&mut found, posts);
                select_past_or_future(found, post_time, posts)
            }
        }
        NorlogeKind::Short(clock) => {
            let minute_key = format!("{:02}{:02}", clock.hour(), clock.minute());
            let same_day = by_minute
                .get(&(target_tribune.clone(), post_time.date(), minute_key.clone()))
                .cloned()
                .unwrap_or_default();
            if !same_day.is_empty() {
                same_day
            } else {
                let mut found = Vec::new();
                for (tribune, date, candidate_minute) in by_minute.keys() {
                    if tribune == &target_tribune && candidate_minute == &minute_key {
                        found.extend(
                            by_minute
                                .get(&(tribune.clone(), *date, minute_key.clone()))
                                .cloned()
                                .unwrap_or_default(),
                        );
                    }
                }
                sort_candidates(&mut found, posts);
                select_past_or_future(found, post_time, posts)
            }
        }
    };

    if candidates.is_empty() {
        return None;
    }
    sort_candidates(&mut candidates, posts);
    if let Some(indice) = reference.indice {
        let index = usize::from(indice).saturating_sub(1);
        if let Some(candidate) = candidates.get(index) {
            return Some(candidate.clone());
        }
    }
    // Latest owner of a clock/second wins, as in gb3tui/gc2.
    candidates.last().cloned()
}

fn select_past_or_future(
    candidates: Vec<PostKey>,
    post_time: NaiveDateTime,
    posts: &BTreeMap<PostKey, Post>,
) -> Vec<PostKey> {
    if candidates.is_empty() {
        return candidates;
    }
    let past: Vec<PostKey> = candidates
        .iter()
        .filter(|key| {
            posts[key]
                .time()
                .naive
                .is_some_and(|candidate| candidate <= post_time)
        })
        .cloned()
        .collect();
    if !past.is_empty() {
        past
    } else {
        candidates
    }
}

fn sort_candidates(candidates: &mut Vec<PostKey>, posts: &BTreeMap<PostKey, Post>) {
    candidates.sort_by(|a, b| compare_posts(&posts[a], &posts[b]).then_with(|| a.id.cmp(&b.id)));
    candidates.dedup();
}

fn root_of(
    key: &PostKey,
    parents_by_child: &HashMap<PostKey, BTreeSet<PostKey>>,
    cache: &mut HashMap<PostKey, Option<PostKey>>,
) -> PostKey {
    if let Some(cached) = cache.get(key) {
        return cached.clone().unwrap_or_else(|| key.clone());
    }
    let mut seen = HashSet::new();
    let mut current = key.clone();
    loop {
        if !seen.insert(current.clone()) {
            break;
        }
        let Some(parent) = parents_by_child
            .get(&current)
            .and_then(|parents| parents.iter().next())
        else {
            break;
        };
        current = parent.clone();
    }
    cache.insert(key.clone(), Some(current.clone()));
    current
}

pub fn color_index(key: &PostKey) -> usize {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.key().hash(&mut hasher);
    (hasher.finish() as usize) % CONVERSATION_COLORS
}

pub const CONVERSATION_COLORS: usize = 8;

#[cfg(test)]
pub fn stats(
    analysis: &Analysis,
    posts: &BTreeMap<PostKey, Post>,
    tribune: &TribuneKey,
    now: NaiveDateTime,
) -> TribuneStats {
    let keys = analysis.posts_for(tribune);
    let mut presence = BTreeSet::new();
    let mut posts_last_hour = 0usize;
    let mut last_post = None;
    for key in keys.iter().rev() {
        let post = &posts[key];
        let time = post.time().naive;
        if let Some(time) = time {
            if last_post.is_none() {
                last_post = Some(time);
            }
            let age = now.signed_duration_since(time).num_seconds();
            if (0..=3_600).contains(&age) {
                posts_last_hour += 1;
            }
            if (0..=900).contains(&age) {
                presence.insert(post.display_identity());
            }
        }
    }

    TribuneStats {
        presence: presence.len(),
        last_post,
        posts_last_hour,
    }
}

/// Give a stable pseudo-random plop replacement for a post author in the plop
/// list, matching gc2's deliberately silly behaviour.
pub fn plop_for(key: &PostKey) -> &'static str {
    const WORDS: &[&str] = &["pika", "plop", "prout", "grouik", "meuh"];
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.key().hash(&mut hasher);
    WORDS[hasher.finish() as usize % WORDS.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Post, SourceId};

    fn post(tribune: &str, id: i64, time: &str, info: &str, message: &str) -> (PostKey, Post) {
        let key = PostKey::new(
            TribuneKey::new(SourceId::Direct(tribune.to_owned()), tribune),
            id,
        );
        (
            key,
            Post {
                id,
                time: time.to_owned(),
                info: info.to_owned(),
                login: String::new(),
                message: message.to_owned(),
                tribune: tribune.to_owned(),
                source_label: String::new(),
            },
        )
    }

    #[test]
    fn builds_reply_edges() {
        let mut posts = BTreeMap::new();
        let (a, pa) = post("t", 1, "20240629120000", "dave", "hello");
        let (b, pb) = post("t", 2, "20240629120100", "bob", "12:00:00 hi");
        posts.insert(a.clone(), pa);
        posts.insert(b.clone(), pb);
        let analysis = analyze(
            &posts,
            &BTreeSet::new(),
            &BTreeSet::new(),
            chrono::NaiveDate::from_ymd_opt(2024, 6, 29)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        );
        assert!(analysis.posts[&b].parents.contains(&a));
        assert!(analysis.posts[&a].children.contains(&b));
        assert_eq!(analysis.posts[&a].root, Some(a.clone()));
        assert_eq!(analysis.posts[&b].root, Some(a));
    }

    #[test]
    fn propagates_hidden_to_replies() {
        let mut posts = BTreeMap::new();
        let (a, pa) = post("t", 1, "20240629120000", "troll", "hello");
        let (b, pb) = post("t", 2, "20240629120100", "bob", "12:00:00 hi");
        posts.insert(a, pa);
        posts.insert(b.clone(), pb);
        let blocked = BTreeSet::from(["troll".to_owned()]);
        let analysis = analyze(
            &posts,
            &blocked,
            &BTreeSet::new(),
            chrono::NaiveDate::from_ymd_opt(2024, 6, 29)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        );
        assert!(analysis.posts[&b].hidden);
    }

    #[test]
    fn marks_bigornos_for_own_identities_with_dots_and_spaces() {
        let mut posts = BTreeMap::new();
        let (dotted, dotted_post) = post("t", 1, "20240629120000", "bob", "my.nick< look at this");
        let (spaced, spaced_post) = post("t", 2, "20240629120100", "bob", "Big Nick< et voilà");
        posts.insert(dotted.clone(), dotted_post);
        posts.insert(spaced.clone(), spaced_post);
        let identities = BTreeSet::from(["my.nick".to_owned(), "Big Nick".to_owned()]);
        let analysis = analyze(
            &posts,
            &BTreeSet::new(),
            &identities,
            chrono::NaiveDate::from_ymd_opt(2024, 6, 29)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        );
        assert!(analysis.posts[&dotted].is_bigorno);
        assert!(analysis.posts[&spaced].is_bigorno);
    }

    #[test]
    fn resolves_id_reference() {
        let mut posts = BTreeMap::new();
        let (a, pa) = post("t", 42, "20240629120000", "dave", "hello");
        let (b, pb) = post("t", 43, "20240629120100", "bob", "#42 hi");
        posts.insert(a.clone(), pa);
        posts.insert(b.clone(), pb);
        let analysis = analyze(
            &posts,
            &BTreeSet::new(),
            &BTreeSet::new(),
            chrono::NaiveDate::from_ymd_opt(2024, 6, 29)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap(),
        );
        assert_eq!(analysis.posts[&b].refs[0].target, Some(a));
    }
}

#[cfg(test)]
mod stats_tests {
    use super::*;
    use crate::model::{Post, SourceId, TribuneKey};

    #[test]
    fn old_posts_are_not_in_last_hour() {
        let mut posts = BTreeMap::new();
        for id in 1..=5 {
            let key = PostKey::new(TribuneKey::new(SourceId::Direct("t".into()), "t"), id);
            posts.insert(
                key,
                Post {
                    id,
                    time: format!("2026102817{:02}00", id),
                    info: "dave".into(),
                    login: String::new(),
                    message: "hi".into(),
                    tribune: "t".into(),
                    source_label: String::new(),
                },
            );
        }
        let now = chrono::NaiveDate::from_ymd_opt(2026, 10, 29)
            .unwrap()
            .and_hms_opt(15, 7, 0)
            .unwrap();
        let analysis = analyze(&posts, &BTreeSet::new(), &BTreeSet::new(), now);
        let stats = stats(
            &analysis,
            &posts,
            &TribuneKey::new(SourceId::Direct("t".into()), "t"),
            now,
        );
        assert_eq!(stats.posts_last_hour, 0);
    }
}
