// high-level check functions operating on MtpClient
// each function performs a specific check and returns structured data

use super::analysis;
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;

#[derive(Debug, Default, Clone)]
pub struct DialogStats {
    pub subscribed_channels: u32,
    pub subscribed_groups: u32,
    pub has_send_bot: bool,
    pub has_xrocket_bot: bool,
    pub owned_channels: Vec<OwnedChannel>,
    pub owned_groups: Vec<OwnedChannel>,
    pub total_dialogs: u32,
}

#[derive(Debug, Default, Clone)]
pub struct OwnedChannel {
    pub channel_id: i64,
    pub access_hash: i64,
    pub title: String,
    pub username: String,
    pub participants_count: u32,
    pub is_broadcast: bool,
    #[allow(dead_code)]
    pub is_creator: bool,
}

pub async fn check_spambot(client: &mut MtpClient) -> Result<String, String> {
    dbg_log!("checker::check_spambot resolving @SpamBot...");

    let resolve_req = tl::build_resolve_username("SpamBot");
    let resolve_resp = client.invoke(&resolve_req).await?;
    let (bot_id, bot_hash) = tl::parse_resolved_peer(&resolve_resp)?;

    let unblock_req = tl::build_unblock_peer(bot_id, bot_hash);
    if let Err(e) = client.invoke(&unblock_req).await {
        dbg_log!("разблокировка @SpamBot не удалась: {e}");
    }

    let mute_req = tl::build_mute_peer(bot_id, bot_hash);
    if let Err(e) = client.invoke(&mute_req).await {
        dbg_log!("отключение уведомлений @SpamBot не удалось: {e}");
    }

    let random_id: i64 = rand::Rng::gen(&mut rand::thread_rng());
    let send_req = tl::build_send_message(bot_id, bot_hash, "/start", random_id);
    let send_result = client.invoke(&send_req).await;

    if let Err(ref e) = send_result {
        if e.contains("YOU_BLOCKED_USER") || e.contains("USER_IS_BLOCKED") {
            let unblock_req = tl::build_unblock_peer(bot_id, bot_hash);
            if let Err(e) = client.invoke(&unblock_req).await {
                dbg_log!("повторная разблокировка @SpamBot не удалась: {e}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;

            let random_id2: i64 = rand::Rng::gen(&mut rand::thread_rng());
            let retry_req = tl::build_send_message(bot_id, bot_hash, "/start", random_id2);
            let _ = client.invoke(&retry_req).await?;
        } else {
            return Err(send_result.unwrap_err());
        }
    }

    let mut messages: Vec<tl::ParsedMessage> = Vec::new();
    for _ in 0..25 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let history_req = tl::build_get_history(bot_id, bot_hash, 3);
        if let Ok(history_resp) = client.invoke(&history_req).await {
            let msgs = tl::parse_messages_structured(&history_resp).unwrap_or_default();
            let has_response = msgs
                .iter()
                .any(|m| !m.text.contains("/start") && m.text.len() > 10);
            if has_response {
                messages = msgs;
                break;
            }
        }
    }

    let status = analysis::analyze_spambot_response(&messages);

    if status == crate::i18n::t("status_clean") {
        if let Some(msg) = messages.first() {
            if let Some(ref btn_data) = msg.first_button_data {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let click_req = tl::build_bot_callback_answer(bot_id, bot_hash, msg.id, btn_data);
                if let Err(e) = client.invoke(&click_req).await {
                    dbg_log!("нажатие кнопки подтверждения @SpamBot не удалось: {e}");
                }
            }
        }
    }

    let delete_req = tl::build_delete_history(bot_id, bot_hash);
    if let Err(e) = client.invoke(&delete_req).await {
        dbg_log!("удаление истории с @SpamBot не удалось: {e}");
    }

    Ok(status)
}

pub async fn check_2fa(client: &mut MtpClient) -> Result<bool, String> {
    let req = tl::build_get_password();
    let resp = client.invoke(&req).await?;
    let info = tl::parse_account_password(&resp)?;
    Ok(info.has_password)
}

pub async fn check_2fa_with_hint(client: &mut MtpClient) -> Result<(bool, String), String> {
    let req = tl::build_get_password();
    let resp = client.invoke(&req).await?;
    let info = tl::parse_account_password(&resp)?;
    Ok((info.has_password, info.hint))
}

pub async fn get_stars_balance(client: &mut MtpClient) -> Result<i64, String> {
    let req = tl::build_get_stars_status();
    let resp = client.invoke(&req).await?;
    tl::parse_stars_status(&resp)
}

pub async fn get_peer_stars_balance(
    client: &mut MtpClient,
    channel_id: i64,
    access_hash: i64,
) -> Result<i64, String> {
    let req = tl::build_get_stars_status_peer(channel_id, access_hash);
    let resp = client.invoke(&req).await?;
    tl::parse_stars_status(&resp)
}

pub async fn get_peer_ton_balance(
    client: &mut MtpClient,
    channel_id: i64,
    access_hash: i64,
) -> Result<i64, String> {
    let req = tl::build_get_ton_status_peer(channel_id, access_hash);
    let resp = client.invoke(&req).await?;
    tl::parse_stars_status(&resp)
}

pub async fn get_premium_until(client: &mut MtpClient) -> Result<Option<i64>, String> {
    let req = tl::build_get_premium_promo();
    let resp = client.invoke(&req).await?;
    let status_text = tl::parse_premium_status_text(&resp)?;
    Ok(analysis::extract_premium_date_from_status(&status_text))
}

pub async fn get_saved_gifts(client: &mut MtpClient) -> Result<(u32, Vec<String>), String> {
    let req = tl::build_get_saved_star_gifts();
    let resp = client.invoke(&req).await?;
    tl::parse_saved_star_gifts(&resp)
}

pub async fn get_dialog_stats(client: &mut MtpClient) -> Result<DialogStats, String> {
    let req_main = tl::build_get_dialogs_with_folder(0, 500);
    let resp_main = client.invoke(&req_main).await?;
    let raw = tl::parse_dialog_stats(&resp_main)?;
    let mut stats = convert_dialog_stats(raw);

    let req_archive = tl::build_get_dialogs_with_folder(1, 500);
    if let Ok(resp_archive) = client.invoke(&req_archive).await {
        if let Ok(archive_raw) = tl::parse_dialog_stats(&resp_archive) {
            let archive = convert_dialog_stats(archive_raw);
            stats.subscribed_channels += archive.subscribed_channels;
            stats.subscribed_groups += archive.subscribed_groups;
            stats.total_dialogs += archive.total_dialogs;
            if archive.has_send_bot {
                stats.has_send_bot = true;
            }
            if archive.has_xrocket_bot {
                stats.has_xrocket_bot = true;
            }
            stats.owned_channels.extend(archive.owned_channels);
            stats.owned_groups.extend(archive.owned_groups);
        }
    }

    // Deduplicate channels and groups by channel_id
    stats.owned_channels.sort_by_key(|c| c.channel_id);
    stats.owned_channels.dedup_by_key(|c| c.channel_id);
    stats.owned_groups.sort_by_key(|g| g.channel_id);
    stats.owned_groups.dedup_by_key(|g| g.channel_id);

    Ok(stats)
}

pub async fn get_saved_messages(
    client: &mut MtpClient,
    limit: i32,
) -> Result<Vec<tl::SavedMessage>, String> {
    let req = tl::build_get_history_self(limit);
    let resp = client.invoke(&req).await?;
    tl::parse_saved_messages(&resp)
}

pub async fn download_document(
    client: &mut MtpClient,
    doc: &tl::SavedDocument,
) -> Result<Vec<u8>, String> {
    if doc.size > 5 * 1024 * 1024 {
        return Err("file too large (>5MB)".into());
    }

    let chunk_size: i32 = 512 * 1024;
    let mut result = Vec::new();
    let mut offset: i64 = 0;

    loop {
        let req = tl::build_upload_get_file(
            doc.id,
            doc.access_hash,
            &doc.file_reference,
            offset,
            chunk_size,
        );
        let resp = client.invoke(&req).await?;
        let chunk = tl::parse_upload_file(&resp)?;

        if chunk.is_empty() {
            break;
        }

        let chunk_len = chunk.len() as i64;
        result.extend(chunk);
        offset += chunk_len;

        if chunk_len < chunk_size as i64 || offset >= doc.size {
            break;
        }
    }

    Ok(result)
}

// convert from mtproto-level DialogStats to checker-level DialogStats
fn convert_dialog_stats(raw: crate::mtproto::client::DialogStats) -> DialogStats {
    DialogStats {
        subscribed_channels: raw.subscribed_channels,
        subscribed_groups: raw.subscribed_groups,
        has_send_bot: raw.has_send_bot,
        has_xrocket_bot: raw.has_xrocket_bot,
        total_dialogs: raw.total_dialogs,
        owned_channels: raw
            .owned_channels
            .into_iter()
            .map(|c| OwnedChannel {
                channel_id: c.channel_id,
                access_hash: c.access_hash,
                title: c.title,
                username: c.username,
                participants_count: c.participants_count,
                is_broadcast: c.is_broadcast,
                is_creator: c.is_creator,
            })
            .collect(),
        owned_groups: raw
            .owned_groups
            .into_iter()
            .map(|c| OwnedChannel {
                channel_id: c.channel_id,
                access_hash: c.access_hash,
                title: c.title,
                username: c.username,
                participants_count: c.participants_count,
                is_broadcast: c.is_broadcast,
                is_creator: c.is_creator,
            })
            .collect(),
    }
}
