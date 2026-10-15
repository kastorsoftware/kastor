// cloner configuration types — mirror the payload sent by the frontend.

use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct ClonerConfigPayload {
    pub source_channel: String,
    pub source_from_id: i32,
    pub source_to_id: i32,

    pub copy_documents: bool,
    pub copy_photos: bool,
    pub copy_videos: bool,
    pub copy_messages_with_video: bool,

    pub show_link_preview: bool,
    pub forward_external_links: bool,
    pub forward_telegram_links: bool,

    pub max_video_size_mb: u64,
    pub max_file_size_mb: u64,
    pub max_photo_size_mb: u64,

    pub destination_mode: String, // "new_channel" | "existing"
    pub existing_channel_id: String,
    pub new_channel_visibility: String, // "public" | "private"
    pub new_channel_username: String,
    pub copy_title: bool,
    pub copy_description: bool,
    pub copy_photo: bool,

    // (from, to) replacement pairs
    pub replacements: Vec<(String, String)>,
    pub skip_keywords: Vec<String>,

    pub delay_min_sec: u32,
    pub delay_max_sec: u32,
    pub preserve_replies: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ClonerConfig {
    pub source_channel: String,
    pub from_id: i32,
    pub to_id: i32,

    pub copy_documents: bool,
    pub copy_photos: bool,
    pub copy_videos: bool,
    pub copy_messages_with_video: bool,

    pub show_link_preview: bool,
    pub forward_external_links: bool,
    pub forward_telegram_links: bool,

    pub max_video_bytes: u64,
    pub max_file_bytes: u64,
    pub max_photo_bytes: u64,

    pub destination: DestinationSpec,

    pub replacements: Vec<(String, String)>,
    pub skip_keywords: Vec<String>,

    pub delay_min_ms: u64,
    pub delay_max_ms: u64,
    pub preserve_replies: bool,
}

#[derive(Debug, Clone)]
pub enum DestinationSpec {
    NewChannel {
        visibility: NewChannelVisibility,
        username: String,
        copy_title: bool,
        copy_description: bool,
        copy_photo: bool,
    },
    Existing {
        id_or_link: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewChannelVisibility {
    Public,
    Private,
}

impl ClonerConfig {
    pub fn from_payload(p: ClonerConfigPayload) -> Result<Self, String> {
        // clamp pacing per spec (3..60 sec)
        let dmin = p.delay_min_sec.clamp(3, 60) as u64 * 1000;
        let dmax = p.delay_max_sec.clamp(3, 60) as u64 * 1000;
        let (dmin, dmax) = if dmin > dmax {
            (dmax, dmax)
        } else {
            (dmin, dmax)
        };

        let mb = |v: u64| {
            if v == 0 {
                0
            } else {
                v.saturating_mul(1024 * 1024)
            }
        };

        let destination = match p.destination_mode.as_str() {
            "new_channel" => {
                let visibility = match p.new_channel_visibility.as_str() {
                    "public" => NewChannelVisibility::Public,
                    "private" => NewChannelVisibility::Private,
                    other => return Err(format!("unknown visibility: {other}")),
                };
                if visibility == NewChannelVisibility::Public
                    && p.new_channel_username.trim().is_empty()
                {
                    return Err(crate::i18n::t("cloner_cfg_public_no_username"));
                }
                DestinationSpec::NewChannel {
                    visibility,
                    username: p.new_channel_username.trim().to_string(),
                    copy_title: p.copy_title,
                    copy_description: p.copy_description,
                    copy_photo: p.copy_photo,
                }
            }
            "existing" => {
                if p.existing_channel_id.trim().is_empty() {
                    return Err(crate::i18n::t("cloner_cfg_no_existing_id"));
                }
                DestinationSpec::Existing {
                    id_or_link: p.existing_channel_id.trim().to_string(),
                }
            }
            other => return Err(format!("unknown destination_mode: {other}")),
        };

        Ok(Self {
            source_channel: p.source_channel.trim().to_string(),
            from_id: p.source_from_id.max(0),
            to_id: p.source_to_id.max(0),
            copy_documents: p.copy_documents,
            copy_photos: p.copy_photos,
            copy_videos: p.copy_videos,
            copy_messages_with_video: p.copy_messages_with_video,
            show_link_preview: p.show_link_preview,
            forward_external_links: p.forward_external_links,
            forward_telegram_links: p.forward_telegram_links,
            max_video_bytes: mb(p.max_video_size_mb),
            max_file_bytes: mb(p.max_file_size_mb),
            max_photo_bytes: mb(p.max_photo_size_mb),
            destination,
            replacements: p.replacements,
            skip_keywords: p
                .skip_keywords
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            delay_min_ms: dmin,
            delay_max_ms: dmax,
            preserve_replies: p.preserve_replies,
        })
    }

    pub fn random_delay_ms(&self) -> u64 {
        if self.delay_max_ms <= self.delay_min_ms {
            return self.delay_min_ms;
        }
        let span = self.delay_max_ms - self.delay_min_ms;
        self.delay_min_ms + (rand::random::<u64>() % (span + 1))
    }
}
