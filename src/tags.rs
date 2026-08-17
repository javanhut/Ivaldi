//! Tags: stable names pointing at a specific seal.
//!
//! Storage mirrors timelines exactly — a `.ivaldi/refs/tags/<name>` marker
//! plus the authoritative record in redb — so the same tooling that scans
//! refs sees tags, and `verify` can cross-check the two.
//!
//! Tags arrive from `ivaldi download --include-tags`, are created with
//! `ivaldi tag create`, and go back out with `ivaldi upload --tags`.

use crate::hash::B3Hash;

/// A tag pointing to a specific seal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    /// MMR index of the seal this tag names.
    pub target_index: u64,
    pub kind: TagKind,
    /// Annotation message (annotated tags only).
    pub message: Option<String>,
    /// Tagger identity (annotated tags only).
    pub tagger: Option<String>,
    /// Unix timestamp (annotated tags only).
    pub timestamp: Option<i64>,
    /// Tagger's UTC offset, e.g. `-0500` (annotated tags only). Preserved
    /// because a git tag object's SHA-1 covers it: dropping the offset would
    /// mint a different object on push than the one that was fetched.
    pub tagger_tz: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagKind {
    Lightweight,
    Annotated,
}

impl Tag {
    /// A tag that carries nothing but a name and a target.
    pub fn lightweight(name: &str, target_index: u64) -> Self {
        Self {
            name: name.to_string(),
            target_index,
            kind: TagKind::Lightweight,
            message: None,
            tagger: None,
            timestamp: None,
            tagger_tz: None,
        }
    }

    /// A tag carrying an annotation, as git's tag objects do.
    pub fn annotated(
        name: &str,
        target_index: u64,
        message: &str,
        tagger: &str,
        timestamp: i64,
        tagger_tz: &str,
    ) -> Self {
        Self {
            name: name.to_string(),
            target_index,
            kind: TagKind::Annotated,
            message: Some(message.to_string()),
            tagger: Some(tagger.to_string()),
            timestamp: Some(timestamp),
            tagger_tz: Some(tagger_tz.to_string()),
        }
    }

    /// Encode the stored record (everything except the name, which is the
    /// key). Header lines then a blank line then the verbatim message, so a
    /// message containing blank lines or `key value` text can't be mistaken
    /// for a header — the same framing git uses for commit and tag objects.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = format!("index {}\n", self.target_index);
        out.push_str(match self.kind {
            TagKind::Lightweight => "kind lightweight\n",
            TagKind::Annotated => "kind annotated\n",
        });
        if let Some(tagger) = &self.tagger {
            out.push_str(&format!("tagger {}\n", tagger));
        }
        if let Some(timestamp) = &self.timestamp {
            out.push_str(&format!("time {}\n", timestamp));
        }
        if let Some(tz) = &self.tagger_tz {
            out.push_str(&format!("tz {}\n", tz));
        }
        out.push('\n');
        if let Some(message) = &self.message {
            out.push_str(message);
        }
        out.into_bytes()
    }
}

/// Decode a stored tag record. `name` comes from the store key.
///
/// Store-facing rather than network-facing, but a corrupt record must still
/// fail closed instead of silently producing a tag that points at the wrong
/// seal — so an unparseable index is an error, not a default.
pub fn parse_tag(name: &str, data: &[u8]) -> Result<Tag, TagError> {
    let text = std::str::from_utf8(data).map_err(|_| TagError::Corrupt(name.to_string()))?;
    let (header, message) = match text.split_once("\n\n") {
        Some((header, message)) => (header, Some(message.to_string())),
        None => (text.trim_end_matches('\n'), None),
    };

    let mut target_index = None;
    let mut kind = TagKind::Lightweight;
    let mut tagger = None;
    let mut timestamp = None;
    let mut tagger_tz = None;
    for line in header.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        match key {
            "index" => target_index = value.parse::<u64>().ok(),
            "kind" if value == "annotated" => kind = TagKind::Annotated,
            "kind" => kind = TagKind::Lightweight,
            "tagger" => tagger = Some(value.to_string()),
            "time" => timestamp = value.parse::<i64>().ok(),
            "tz" => tagger_tz = Some(value.to_string()),
            _ => {}
        }
    }

    Ok(Tag {
        name: name.to_string(),
        target_index: target_index.ok_or_else(|| TagError::Corrupt(name.to_string()))?,
        kind,
        // An empty trailing section means "no message", not an empty one.
        message: message.filter(|m| !m.is_empty()),
        tagger,
        timestamp,
        tagger_tz,
    })
}

/// Resolve a tag's target seal hash through the repository.
pub fn target_hash(repo: &crate::repo::Repo, tag: &Tag) -> Option<B3Hash> {
    repo.get_leaf(tag.target_index).ok().flatten().map(|l| l.hash())
}

#[derive(Debug, thiserror::Error)]
pub enum TagError {
    #[error("tag already exists: {0}")]
    AlreadyExists(String),
    #[error("tag not found: {0}")]
    NotFound(String),
    #[error("corrupt tag record for {0}")]
    Corrupt(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lightweight_round_trip() {
        let tag = Tag::lightweight("v1.0", 7);
        let parsed = parse_tag("v1.0", &tag.canonical_bytes()).unwrap();
        assert_eq!(parsed, tag);
        assert_eq!(parsed.kind, TagKind::Lightweight);
        assert!(parsed.message.is_none());
    }

    #[test]
    fn annotated_round_trip_preserves_a_multi_paragraph_message() {
        // A release note with a blank line in it must not be re-read as
        // headers — that's what the header/blank-line/body framing is for.
        let message = "Release 2.0\n\nindex 999\nkind lightweight\n";
        let tag = Tag::annotated("v2.0", 3, message, "Alice <a@x>", 1_700_000_000, "-0500");
        let parsed = parse_tag("v2.0", &tag.canonical_bytes()).unwrap();
        assert_eq!(parsed, tag);
        assert_eq!(parsed.target_index, 3);
        assert_eq!(parsed.message.as_deref(), Some(message));
        assert_eq!(parsed.tagger.as_deref(), Some("Alice <a@x>"));
        assert_eq!(parsed.timestamp, Some(1_700_000_000));
        assert_eq!(parsed.tagger_tz.as_deref(), Some("-0500"));
    }

    #[test]
    fn record_without_an_index_is_refused() {
        let err = parse_tag("v1", b"kind annotated\n\nhi").unwrap_err();
        assert!(matches!(err, TagError::Corrupt(_)));
    }
}
