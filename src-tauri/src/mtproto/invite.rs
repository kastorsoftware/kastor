// shared link-to-channel resolver. accepts a link in any supported form
// (@username, t.me/<user>, t.me/+<hash>, t.me/joinchat/<hash>) and returns
// (channel_id, access_hash). joins the channel via invite hash when needed,
// rejecting request-only channels up front so callers can fail fast.

use crate::mtproto::client::MtpClient;
use crate::mtproto::text_parse;
use crate::mtproto::tl;

#[derive(Debug, Clone)]
pub struct ResolvedChannel {
    pub channel_id: i64,
    pub access_hash: i64,
    // public username extracted from the link; None for private invites
    pub username_hint: Option<String>,
    // title extracted from checkChatInvite; empty for public links
    pub title_hint: String,
    // true when the helper performed importChatInvite during this call.
    // callers can use this to decide whether to leaveChannel after work.
    pub joined_now: bool,
    // broadcast channel (one-way feed). false for megagroups and basic chats.
    pub is_broadcast: bool,
}

pub async fn resolve_channel_link(
    client: &mut MtpClient,
    link: &str,
) -> Result<ResolvedChannel, String> {
    let trimmed = link.trim();
    if trimmed.is_empty() {
        return Err(crate::i18n::t("invite_empty_link"));
    }

    // private invite link: t.me/+xxx or t.me/joinchat/xxx
    if let Some((kind, body)) = text_parse::parse_invite_link(trimmed) {
        match kind {
            "private" => return join_private(client, &body).await,
            "addlist" => return Err(crate::i18n::t("invite_addlist_not_channel")),
            _ => {}
        }
    }

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
        return Err(crate::i18n::t("invite_parse_error"));
    }

    let req = tl::build_resolve_username(username);
    let resp = client
        .invoke(&req)
        .await
        .map_err(|e| format!("resolve {username}: {e}"))?;
    let (channel_id, access_hash) =
        tl::parse_resolved_peer(&resp).map_err(|e| format!("parse peer: {e}"))?;
    let is_broadcast = tl::scan_channel_is_broadcast(&resp, channel_id);

    Ok(ResolvedChannel {
        channel_id,
        access_hash,
        username_hint: Some(username.to_string()),
        title_hint: String::new(),
        joined_now: false,
        is_broadcast,
    })
}

// joins a private channel via invite hash. handles request-only channels and
// the "already a member" case (both at the checkChatInvite stage and at the
// importChatInvite stage where it surfaces as USER_ALREADY_PARTICIPANT).
async fn join_private(client: &mut MtpClient, hash: &str) -> Result<ResolvedChannel, String> {
    let check_req = tl::build_check_chat_invite(hash);
    let check_resp = client
        .invoke(&check_req)
        .await
        .map_err(|e| format!("checkChatInvite: {e}"))?;
    let summary = tl::parse_chat_invite_summary(&check_resp)
        .map_err(|e| format!("parse checkChatInvite: {e}"))?;

    // already a member — channel info is embedded in chatInviteAlready/chatInvitePeek
    if !summary.is_chat_invite {
        if let (Some(id), Some(ah)) = (summary.channel_id, summary.access_hash) {
            // re-scan chat object to detect broadcast vs megagroup; for "already"/"peek"
            // the chatInvite flags are zero so we can't trust them.
            let is_broadcast = tl::scan_channel_is_broadcast(&check_resp, id);
            return Ok(ResolvedChannel {
                channel_id: id,
                access_hash: ah,
                username_hint: None,
                title_hint: summary.title,
                joined_now: false,
                is_broadcast,
            });
        }
        return Err(crate::i18n::t("invite_already_member_no_hash"));
    }

    if summary.request_needed {
        let label = if summary.title.is_empty() {
            hash.to_string()
        } else {
            summary.title
        };
        return Err(crate::i18n::t_with(
            "invite_request_needed",
            &[("label", &label)],
        ));
    }

    let import_req = tl::build_import_chat_invite(hash);
    match client.invoke(&import_req).await {
        Ok(resp) => {
            let (id, ah) = tl::parse_first_accessible_channel(&resp)
                .map_err(|e| format!("parse private invite: {e}"))?;
            // prefer flags from the joined chat object; fall back to the
            // chatInvite flags returned by checkChatInvite.
            let is_broadcast = if let Some(b) = tl::scan_channel_is_broadcast_opt(&resp, id) {
                b
            } else {
                summary.broadcast && !summary.megagroup
            };
            Ok(ResolvedChannel {
                channel_id: id,
                access_hash: ah,
                username_hint: None,
                title_hint: summary.title,
                joined_now: true,
                is_broadcast,
            })
        }
        Err(e) => {
            if e.contains("INVITE_REQUEST_SENT") {
                return Err(crate::i18n::t("invite_request_sent"));
            }
            if e.contains("USER_ALREADY_PARTICIPANT") {
                // race: we joined between check and import, or another session did. recheck to recover handles.
                let recheck = client
                    .invoke(&check_req)
                    .await
                    .map_err(|e2| format!("recheckChatInvite: {e2}"))?;
                if let Ok(s) = tl::parse_chat_invite_summary(&recheck) {
                    if let (Some(id), Some(ah)) = (s.channel_id, s.access_hash) {
                        let is_broadcast = tl::scan_channel_is_broadcast(&recheck, id);
                        return Ok(ResolvedChannel {
                            channel_id: id,
                            access_hash: ah,
                            username_hint: None,
                            title_hint: s.title,
                            joined_now: false,
                            is_broadcast,
                        });
                    }
                }
                return Err(crate::i18n::t("invite_already_no_hash"));
            }
            Err(format!("importChatInvite: {e}"))
        }
    }
}
