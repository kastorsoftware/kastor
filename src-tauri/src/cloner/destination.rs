// resolves the destination channel: either creates a new one (cloning
// title / description / photo from the source) or looks up an existing one
// the account already owns/admins.

use tauri::Emitter;

use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;

use super::config::{DestinationSpec, NewChannelVisibility};

#[derive(Debug, Clone, Copy)]
pub struct ChannelHandle {
    pub channel_id: i64,
    pub access_hash: i64,
}

pub async fn resolve_or_create_destination(
    client: &mut MtpClient,
    spec: &DestinationSpec,
    source: &SourceContext,
    app: &tauri::AppHandle,
) -> Result<ChannelHandle, String> {
    match spec {
        DestinationSpec::Existing { id_or_link } => resolve_existing(client, id_or_link).await,
        DestinationSpec::NewChannel {
            visibility,
            username,
            copy_title,
            copy_description,
            copy_photo,
        } => {
            create_new_channel(
                client,
                visibility,
                username,
                *copy_title,
                *copy_description,
                *copy_photo,
                source,
                app,
            )
            .await
        }
    }
}

#[derive(Debug, Clone)]
pub struct SourceContext {
    pub channel_id: i64,
    pub access_hash: i64,
    pub title: String,
    pub about: String,
    pub photo: Option<SourcePhoto>,
    pub joined_now: bool,
    pub noforwards: bool,
}

#[derive(Debug, Clone)]
pub struct SourcePhoto {
    pub photo_id: i64,
    pub access_hash: i64,
    pub file_reference: Vec<u8>,
}

async fn resolve_existing(
    client: &mut MtpClient,
    id_or_link: &str,
) -> Result<ChannelHandle, String> {
    let trimmed = id_or_link.trim();
    let username = trimmed
        .trim_start_matches("https://t.me/")
        .trim_start_matches("http://t.me/")
        .trim_start_matches("t.me/")
        .trim_start_matches('@')
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .trim_end_matches('/');

    if username.is_empty() {
        return Err(crate::i18n::t("cloner_dest_parse_error"));
    }

    // numeric -1001234567890 form is not currently supported because we'd need
    // to reach into the account's dialog list to recover access_hash. require
    // a username/link instead — frontend tooltip already mentions this.
    if username.chars().all(|c| c == '-' || c.is_ascii_digit()) {
        return Err(crate::i18n::t("cloner_dest_numeric_unsupported"));
    }

    let req = tl::build_resolve_username(username);
    let resp = client
        .invoke(&req)
        .await
        .map_err(|e| format!("resolve {username}: {e}"))?;
    let (channel_id, access_hash) =
        tl::parse_resolved_peer(&resp).map_err(|e| format!("parse destination peer: {e}"))?;
    Ok(ChannelHandle {
        channel_id,
        access_hash,
    })
}

async fn create_new_channel(
    client: &mut MtpClient,
    visibility: &NewChannelVisibility,
    username: &str,
    copy_title: bool,
    copy_description: bool,
    copy_photo: bool,
    source: &SourceContext,
    app: &tauri::AppHandle,
) -> Result<ChannelHandle, String> {
    let title = if copy_title && !source.title.is_empty() {
        source.title.clone()
    } else {
        "Cloned channel".to_string()
    };
    let about = if copy_description && !source.about.is_empty() {
        source.about.clone()
    } else {
        String::new()
    };

    let req = tl::build_create_channel(&title, &about, true, false);
    let resp = client
        .invoke(&req)
        .await
        .map_err(|e| format!("createChannel: {e}"))?;
    let (channel_id, access_hash) =
        tl::parse_created_channel(&resp).map_err(|e| format!("parse created channel: {e}"))?;

    if matches!(visibility, NewChannelVisibility::Public) && !username.is_empty() {
        let check = tl::build_channel_check_username(channel_id, access_hash, username);
        let check_resp = client
            .invoke(&check)
            .await
            .map_err(|e| format!("checkUsername: {e}"))?;
        let ctor = if check_resp.len() >= 4 {
            u32::from_le_bytes([check_resp[0], check_resp[1], check_resp[2], check_resp[3]])
        } else {
            0
        };
        // Bool true = BOOL_TRUE
        if ctor != crate::mtproto::tl_gen::BOOL_TRUE {
            return Err(crate::i18n::t_with(
                "cloner_dest_username_taken",
                &[("username", username)],
            ));
        }
        let upd = tl::build_channel_update_username(channel_id, access_hash, username);
        client
            .invoke(&upd)
            .await
            .map_err(|e| format!("updateUsername: {e}"))?;
    }

    if copy_photo {
        if let Some(ref src_photo) = source.photo {
            let req = tl::build_channel_edit_photo_existing(
                channel_id,
                access_hash,
                src_photo.photo_id,
                src_photo.access_hash,
                &src_photo.file_reference,
            );
            match client.invoke(&req).await {
                Ok(_) => {
                    let _ = app.emit("cloner-log", crate::i18n::t("cloner_dest_avatar_copied"));
                }
                Err(e) => {
                    let _ = app.emit(
                        "cloner-log",
                        crate::i18n::t_with("cloner_dest_avatar_error", &[("error", &e)]),
                    );
                }
            }
        } else {
            let _ = app.emit("cloner-log", crate::i18n::t("cloner_dest_no_avatar"));
        }
    }

    Ok(ChannelHandle {
        channel_id,
        access_hash,
    })
}
