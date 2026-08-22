// thin facade over the full TL media decoder in mtproto::tl. exposes the
// canonical media kind + byte size pulled out of a single Message blob.

use crate::mtproto::tl::{self, MediaKindRepr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    None,
    Photo,
    Video,
    Document,
    Audio,
    Other,
}

impl From<MediaKindRepr> for MediaKind {
    fn from(value: MediaKindRepr) -> Self {
        match value {
            MediaKindRepr::None => MediaKind::None,
            MediaKindRepr::Photo => MediaKind::Photo,
            MediaKindRepr::Video => MediaKind::Video,
            MediaKindRepr::Document => MediaKind::Document,
            MediaKindRepr::Audio => MediaKind::Audio,
            MediaKindRepr::Other => MediaKind::Other,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MediaInfo {
    pub kind: MediaKind,
    pub size_bytes: u64,
}

impl MediaInfo {
    pub fn none() -> Self {
        Self {
            kind: MediaKind::None,
            size_bytes: 0,
        }
    }
}

// fully parses a single message blob (raw bytes returned by getHistory for one
// post) and returns the canonical media kind + size. when no media is present
// or the blob is unrecognised, returns MediaKind::None / size 0.
pub fn detect_media(message_blob: &[u8]) -> MediaInfo {
    match tl::extract_message_media_summary(message_blob) {
        Some(summary) => MediaInfo {
            kind: MediaKind::from(summary.kind),
            size_bytes: summary.size_bytes,
        },
        None => MediaInfo::none(),
    }
}
