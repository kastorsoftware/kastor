// text transforms: replacements, skip keywords, link filtering decisions.

use super::config::ClonerConfig;
use crate::mtproto::tl::ParsedMessage;
use crate::i18n::t;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum SkipReason {
    KeywordMatch,
    DocumentDisabled,
    PhotoDisabled,
    VideoDisabled,
    VideoMessageDisabled,
    ExternalLinkDisabled,
    TelegramLinkDisabled,
    OversizedFile,
    OversizedVideo,
    OversizedPhoto,
}

impl SkipReason {
    pub fn ru(self) -> String {
        match self {
            Self::KeywordMatch => t("cloner_skip_keyword"),
            Self::DocumentDisabled => t("cloner_skip_documents"),
            Self::PhotoDisabled => t("cloner_skip_photos"),
            Self::VideoDisabled => t("cloner_skip_videos"),
            Self::VideoMessageDisabled => t("cloner_skip_video_msg"),
            Self::ExternalLinkDisabled => t("cloner_skip_ext_link"),
            Self::TelegramLinkDisabled => t("cloner_skip_tg_link"),
            Self::OversizedFile => t("cloner_skip_file_size"),
            Self::OversizedVideo => t("cloner_skip_video_size"),
            Self::OversizedPhoto => t("cloner_skip_photo_size"),
        }
    }
}

// applies all configured (from, to) replacements to the source text.
// returns (new_text, modified) where modified=true means an editMessage call
// is required to push the change to the destination channel.
pub fn apply_replacements(text: &str, replacements: &[(String, String)]) -> (String, bool) {
    if replacements.is_empty() || text.is_empty() {
        return (text.to_string(), false);
    }
    let mut current = text.to_string();
    let mut changed = false;
    for (from, to) in replacements {
        if from.is_empty() { continue; }
        if current.contains(from.as_str()) {
            current = current.replace(from.as_str(), to.as_str());
            changed = true;
        }
    }
    (current, changed)
}

// case-insensitive whole-substring match; matches are broken down by punctuation
// to support "#реклама" style keywords and avoid matching inside other words
// when the keyword starts with a letter.
pub fn contains_skip_keyword(text: &str, keywords: &[String]) -> bool {
    if keywords.is_empty() || text.is_empty() {
        return false;
    }
    let lower = text.to_lowercase();
    keywords.iter().any(|k| {
        let kl = k.to_lowercase();
        if kl.is_empty() { return false; }
        if kl.starts_with('#') || kl.starts_with('@') {
            // hashtags / mentions: substring match is fine
            return lower.contains(&kl);
        }
        // word-bounded match
        let bytes = lower.as_bytes();
        let needle = kl.as_bytes();
        if needle.len() > bytes.len() { return false; }
        let mut i = 0;
        while i + needle.len() <= bytes.len() {
            if &bytes[i..i + needle.len()] == needle {
                let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
                let after = i + needle.len();
                let after_ok = after == bytes.len() || !bytes[after].is_ascii_alphanumeric();
                if before_ok && after_ok { return true; }
            }
            i += 1;
        }
        false
    })
}

// returns true if the message text contains any URL (http/https/t.me/@user/etc).
pub fn has_external_link(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("http://") || lower.contains("https://") || lower.contains("www.")
}

pub fn has_telegram_link(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("t.me/") || lower.contains("telegram.me/") || lower.contains("@")
}

pub fn classify_skip(msg: &ParsedMessage, cfg: &ClonerConfig) -> Option<SkipReason> {
    if contains_skip_keyword(&msg.text, &cfg.skip_keywords) {
        return Some(SkipReason::KeywordMatch);
    }

    // link gating only fires if the text actually has a link of that kind
    if !cfg.forward_telegram_links && has_telegram_link(&msg.text) {
        return Some(SkipReason::TelegramLinkDisabled);
    }
    if !cfg.forward_external_links && has_external_link(&msg.text) {
        return Some(SkipReason::ExternalLinkDisabled);
    }

    // ParsedMessage only carries text + reply_markup info today, so we cannot
    // gate by media type without a richer parser. media-type gating happens
    // in runner.rs once we obtain the original message body via channels.getMessages.
    None
}

// the text we send to editMessage — the forwarded copy still carries the original
// caption/text, so editing is only needed when at least one replacement matched.
pub fn build_edited_text(original: &str, replacements: &[(String, String)]) -> Option<String> {
    let (new_text, changed) = apply_replacements(original, replacements);
    if changed { Some(new_text) } else { None }
}


