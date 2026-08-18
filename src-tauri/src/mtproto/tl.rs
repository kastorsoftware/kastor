use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Cursor, Read};
use super::tl_gen;
use super::client::MtpClient;

// tl constructors - core (re-exported from tl_gen where available)
pub use super::tl_gen::VECTOR;
// mtproto service-layer ctors live in the mtproto module (not the api schema)
pub use super::service_ctors::RPC_ERROR;
pub use super::service_ctors::GZIP_PACKED;

// peers
pub use super::tl_gen::INPUT_PEER_SELF;
pub use super::tl_gen::INPUT_PEER_CHANNEL;

// reply_markup constructors
pub use super::tl_gen::REPLY_INLINE_MARKUP;
pub use super::tl_gen::REPLY_KEYBOARD_MARKUP;
pub use super::tl_gen::KEYBOARD_BUTTON_ROW;
pub use super::tl_gen::KEYBOARD_BUTTON_CALLBACK;

// account actions

pub fn serialize_string(s: &str) -> Vec<u8> {
    serialize_bytes(s.as_bytes())
}

pub fn serialize_bytes(data: &[u8]) -> Vec<u8> {
    let len = data.len();
    let mut buf = Vec::new();

    if len < 254 {
        buf.push(len as u8);
        buf.extend_from_slice(data);
        let total = 1 + len;
        let padding = (4 - (total % 4)) % 4;
        buf.extend(std::iter::repeat(0u8).take(padding));
    } else {
        buf.push(254);
        buf.push((len & 0xff) as u8);
        buf.push(((len >> 8) & 0xff) as u8);
        buf.push(((len >> 16) & 0xff) as u8);
        buf.extend_from_slice(data);
        let total = 4 + len;
        let padding = (4 - (total % 4)) % 4;
        buf.extend(std::iter::repeat(0u8).take(padding));
    }

    buf
}

pub fn deserialize_string(cursor: &mut Cursor<&[u8]>) -> Result<String, String> {
    let bytes = deserialize_bytes(cursor)?;
    String::from_utf8(bytes).map_err(|e| format!("invalid utf8: {e}"))
}

pub fn deserialize_bytes(cursor: &mut Cursor<&[u8]>) -> Result<Vec<u8>, String> {
    let first = read_u8(cursor)?;
    let (len, header_size) = if first < 254 {
        (first as usize, 1usize)
    } else {
        let b1 = read_u8(cursor)? as usize;
        let b2 = read_u8(cursor)? as usize;
        let b3 = read_u8(cursor)? as usize;
        (b1 | (b2 << 8) | (b3 << 16), 4usize)
    };

    let mut data = vec![0u8; len];
    cursor.read_exact(&mut data).map_err(|e| format!("read bytes: {e}"))?;

    let total = header_size + len;
    let padding = (4 - (total % 4)) % 4;
    let mut skip = vec![0u8; padding];
    cursor.read_exact(&mut skip).map_err(|e| format!("read padding: {e}"))?;

    Ok(data)
}

fn read_u8(cursor: &mut Cursor<&[u8]>) -> Result<u8, String> {
    let mut buf = [0u8; 1];
    cursor.read_exact(&mut buf).map_err(|e| format!("read u8: {e}"))?;
    Ok(buf[0])
}

// build users.getUsers([inputUserSelf]) wrapped in initConnection + invokeWithLayer
pub fn build_get_me_request(
    api_id: i32,
    device: &str,
    system: &str,
    app_version: &str,
    system_lang: &str,
    lang: &str,
) -> Vec<u8> {
    tl_gen::build_get_me(api_id, device, system, app_version, system_lang, lang)
}

#[derive(Debug, Clone)]
pub struct UserInfo {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub phone: String,
    pub username: String,
    pub premium: bool,
    pub nft_usernames: Vec<String>,
}

pub fn parse_rpc_response(data: &[u8]) -> Result<Vec<u8>, String> {
    tl_gen::unwrap_rpc(data)
}

pub fn parse_users_response(data: &[u8]) -> Result<UserInfo, String> {
    let mut cursor = Cursor::new(data);
    let ctor = cursor.read_u32::<LittleEndian>().map_err(|e| format!("read ctor: {e}"))?;

    if ctor == GZIP_PACKED {
        let compressed = deserialize_bytes(&mut cursor)?;
        let decompressed = decompress_gzip(&compressed)?;
        return parse_users_response(&decompressed);
    }

    if ctor == RPC_ERROR {
        let error_code = cursor.read_i32::<LittleEndian>().map_err(|_| "read error_code")?;
        let error_msg = deserialize_string(&mut cursor)?;
        return Err(format!("rpc error {error_code}: {error_msg}"));
    }

    if ctor != VECTOR {
        return Err(format!("expected vector, got 0x{ctor:08x}"));
    }

    let count = cursor.read_u32::<LittleEndian>().map_err(|_| "read vector count")?;
    if count == 0 {
        return Err("empty users vector".into());
    }

    parse_user_object(&mut cursor)
}

fn parse_user_object(cursor: &mut Cursor<&[u8]>) -> Result<UserInfo, String> {
    let user = tl_gen::TlUser::deserialize(cursor)?;
    tl_user_to_info(user)
}

fn tl_user_to_info(user: tl_gen::TlUser) -> Result<UserInfo, String> {
    match user {
        tl_gen::TlUser::Empty { .. } => Err("user is empty (invalid session)".into()),
        tl_gen::TlUser::User {
            premium,
            id,
            first_name,
            last_name,
            username,
            phone,
            usernames,
            ..
        } => {
            let first_name = first_name.unwrap_or_default();
            let last_name = last_name.unwrap_or_default();
            let phone = phone.unwrap_or_default();
            let username = username.unwrap_or_default();

            // deserialize raw Username objects to extract active usernames
            let mut nft_usernames = Vec::new();
            if let Some(raw_vec) = usernames {
                for raw in &raw_vec {
                    if let Ok(uname) = tl_gen::deserialize_tl_obj::<tl_gen::TlUsername>(raw) {
                        if uname.active && !uname.username.is_empty() {
                            nft_usernames.push(uname.username);
                        }
                    }
                }
            }

            let final_username = if username.is_empty() && !nft_usernames.is_empty() {
                nft_usernames[0].clone()
            } else {
                username
            };

            Ok(UserInfo {
                id,
                first_name,
                last_name,
                phone,
                username: final_username,
                premium,
                nft_usernames,
            })
        }
    }
}

pub fn decompress_gzip(data: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut decoder = flate2::read::GzDecoder::new(data);
    let mut result = Vec::new();
    decoder.read_to_end(&mut result).map_err(|e| format!("gzip decompress: {e}"))?;
    Ok(result)
}

// === spambot check TL builders ===

// contacts.resolveUsername#725afbbc flags:# username:string referer:flags.0?string = contacts.ResolvedPeer
pub fn build_resolve_username(username: &str) -> Vec<u8> {
    tl_gen::build_contacts_resolveUsername(username, None)
}

// contacts.search#11f812d8 q:string limit:int = contacts.Found
pub fn build_contacts_search(query: &str, limit: i32) -> Vec<u8> {
    tl_gen::build_contacts_search(false, false, query, limit)
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FoundEntry {
    pub username: String,
    pub is_channel: bool,
    pub is_group: bool,
    pub is_user: bool,
}

#[allow(dead_code)]
pub fn parse_contacts_found(data: &[u8]) -> Result<Vec<FoundEntry>, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let mut cursor = Cursor::new(inner.as_slice());
    let _ctor = cursor.read_u32::<LittleEndian>().map_err(|_| "read ctor")?;
    let found = tl_gen::TlContactsFound::deserialize(&mut cursor)?;
    let mut entries = Vec::new();
    for raw in &found.users {
        if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
            if let tl_gen::TlUser::User { username, .. } = user {
                if let Some(uname) = username {
                    if !uname.is_empty() {
                        entries.push(FoundEntry { username: uname, is_channel: false, is_group: false, is_user: true });
                    }
                }
            }
        }
    }
    for raw in &found.chats {
        if let Ok(chat) = tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(raw) {
            match chat {
                tl_gen::TlChat::Channel { broadcast, megagroup, username, .. } => {
                    if let Some(uname) = username {
                        if !uname.is_empty() {
                            entries.push(FoundEntry { username: uname, is_channel: broadcast && !megagroup, is_group: megagroup, is_user: false });
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(entries)
}

// messages.sendMessage (simplified, text only, no_webpage + silent)
pub fn build_send_message(peer_id: i64, access_hash: i64, message: &str, random_id: i64) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_user(peer_id, access_hash);
    tl_gen::build_messages_sendMessage(
        true, true, false, false, false, false, false, false,
        &peer, None, message, random_id, None, None, None, None, None, None, None, None, None, None,
    )
}

// messages.getHistory#4423e6c5 peer:InputPeer offset_id:int offset_date:int
//   add_offset:int limit:int max_id:int min_id:int hash:long
pub fn build_get_history(peer_id: i64, access_hash: i64, limit: i32) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_user(peer_id, access_hash);
    tl_gen::build_messages_getHistory(&peer, 0, 0, 0, limit, 0, 0, 0)
}

// messages.deleteHistory#b08f922a flags:# just_clear:flags.0 revoke:flags.1
//   peer:InputPeer max_id:int min_date:flags.2 max_date:flags.3
pub fn build_delete_history(peer_id: i64, access_hash: i64) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_user(peer_id, access_hash);
    tl_gen::build_messages_deleteHistory(true, true, &peer, i32::MAX, None, None)
}

// contacts.unblock#b550d328 id:InputPeer = Bool
pub fn build_unblock_peer(peer_id: i64, access_hash: i64) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_user(peer_id, access_hash);
    tl_gen::build_contacts_unblock(false, &peer)
}

// account.updateNotifySettings - mute peer
pub fn build_mute_peer(peer_id: i64, access_hash: i64) -> Vec<u8> {
    let inner_peer = tl_gen::serialize_input_peer_user(peer_id, access_hash);
    let notify_peer = tl_gen::serialize_inputNotifyPeer(&inner_peer);
    let settings = tl_gen::serialize_inputPeerNotifySettings(
        None, None, Some(2147483647), None, None, None, None,
    );
    tl_gen::build_account_updateNotifySettings(&notify_peer, &settings)
}

// parse contacts.resolvedPeer response to get user_id + access_hash
pub fn parse_resolved_peer(data: &[u8]) -> Result<(i64, i64), String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let mut cursor = Cursor::new(inner.as_slice());

    // read contacts.resolvedPeer ctor
    let _ctor = cursor.read_u32::<LittleEndian>().map_err(|_| "read ctor")?;

    let resolved = tl_gen::TlContactsResolvedPeer::deserialize(&mut cursor)?;

    // read peer from raw bytes
    let mut peer_cursor = Cursor::new(resolved.peer.as_slice());
    let peer = tl_gen::read_peer(&mut peer_cursor)?;

    match peer {
        tl_gen::Peer::User(uid) => {
            for raw in &resolved.users {
                if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
                    if let tl_gen::TlUser::User { id, access_hash, .. } = user {
                        if id == uid {
                            return Ok((uid, access_hash.unwrap_or(0)));
                        }
                    }
                }
            }
            Err("user not found in resolvedPeer".into())
        }
        tl_gen::Peer::Channel(cid) => {
            for raw in &resolved.chats {
                if let Ok(chat) = tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(raw) {
                    match chat {
                        tl_gen::TlChat::Channel { id, access_hash, .. } if id == cid => {
                            return Ok((cid, access_hash.unwrap_or(0)));
                        }
                        _ => continue,
                    }
                }
            }
            Err("channel not found in resolvedPeer".into())
        }
        tl_gen::Peer::Chat(cid) => Ok((cid, 0)),
    }
}

// parse messages from getHistory response - returns message texts
pub fn parse_messages_history(data: &[u8]) -> Result<Vec<String>, String> {
    let parsed = parse_messages_structured(data)?;
    Ok(parsed.into_iter().map(|m| m.text).collect())
}

// structured message with reply_markup info
#[derive(Debug, Clone)]
pub struct ParsedMessage {
    pub id: i32,
    pub text: String,
    pub reply_markup_rows: u32,
    pub buttons: Vec<ParsedButton>,
    pub first_button_data: Option<Vec<u8>>,
    pub first_button_text: Option<String>,
    pub is_service: bool,
    pub reply_to_msg_id: Option<i32>,
    // urls carried by messageEntityTextUrl entities (e.g. the clickable
    // "Telegram Terms of Service" link points to telegram.org/tos)
    pub entity_urls: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ParsedButton {
    pub text: String,
    pub data: Option<Vec<u8>>,
}

// parse messages from getHistory response via tl_gen deserialization
pub fn parse_messages_structured(data: &[u8]) -> Result<Vec<ParsedMessage>, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let mut cursor = Cursor::new(inner.as_slice());

    let resp = tl_gen::TlMessagesMessages::deserialize(&mut cursor)
        .map_err(|e| format!("deserialize messages response: {e}"))?;

    let raw_messages = match resp {
        tl_gen::TlMessagesMessages::NotModified { .. } => return Ok(Vec::new()),
        tl_gen::TlMessagesMessages::Messages { messages, .. } => messages,
        tl_gen::TlMessagesMessages::Slice { messages, .. } => messages,
        tl_gen::TlMessagesMessages::ChannelMessages { messages, .. } => messages,
    };

    let mut results = Vec::new();
    for raw in &raw_messages {
        let mut msg_cursor = Cursor::new(raw.as_slice());
        match tl_gen::TlMessage::deserialize(&mut msg_cursor) {
            Ok(msg) => results.push(tl_message_to_parsed(msg)),
            Err(_) => break,
        }
    }
    Ok(results)
}

fn tl_message_to_parsed(msg: tl_gen::TlMessage) -> ParsedMessage {
    match msg {
        tl_gen::TlMessage::Empty { id, .. } => ParsedMessage {
            id, text: String::new(), reply_markup_rows: 0,
            buttons: Vec::new(),
            first_button_data: None, first_button_text: None, is_service: false,
            reply_to_msg_id: None, entity_urls: Vec::new(),
        },
        tl_gen::TlMessage::Message { id, message, reply_markup, entities, reply_to, .. } => {
            let (rows, buttons) = match reply_markup {
                Some(ref rm) => extract_markup_info(rm),
                None => (0, Vec::new()),
            };
            let (btn_text, btn_data) = match buttons.iter().find_map(|button| {
                button.data.as_ref().map(|data| (button.text.clone(), data.clone()))
            }) {
                Some((text, data)) => (Some(text), Some(data)),
                None => (None, None),
            };
            let entity_urls = entities.as_ref()
                .map(|ents| extract_entity_urls(ents))
                .unwrap_or_default();
            let reply_to_msg_id = reply_to.as_ref().and_then(|rt| {
                let mut cursor = Cursor::new(rt.as_slice());
                tl_gen::TlMessageReplyHeader::deserialize(&mut cursor).ok().and_then(|h| {
                    match h {
                        tl_gen::TlMessageReplyHeader::MessageReplyHeader { reply_to_msg_id, .. } => reply_to_msg_id,
                        _ => None,
                    }
                })
            });
            ParsedMessage {
                id, text: message, reply_markup_rows: rows,
                buttons,
                first_button_data: btn_data, first_button_text: btn_text,
                is_service: false, reply_to_msg_id, entity_urls,
            }
        },
        tl_gen::TlMessage::Service { id, .. } => ParsedMessage {
            id, text: String::new(), reply_markup_rows: 0,
            buttons: Vec::new(),
            first_button_data: None, first_button_text: None, is_service: true,
            reply_to_msg_id: None, entity_urls: Vec::new(),
        },
    }
}

// collect URLs from messageEntityTextUrl entities of a message
fn extract_entity_urls(entities: &[Vec<u8>]) -> Vec<String> {
    let mut urls = Vec::new();
    for raw in entities {
        if let Ok(tl_gen::TlMessageEntity::TextUrl { url, .. }) =
            tl_gen::deserialize_tl_obj::<tl_gen::TlMessageEntity>(raw)
        {
            urls.push(url);
        }
    }
    urls
}

fn extract_markup_info(raw: &[u8]) -> (u32, Vec<ParsedButton>) {
    if raw.len() < 8 { return (0, Vec::new()); }
    let ctor = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
    let mut pos = 4usize;

    // replyKeyboardMarkup#85dd99d1 has flags before rows
    if ctor == REPLY_KEYBOARD_MARKUP {
        if pos + 4 > raw.len() { return (0, Vec::new()); }
        pos += 4; // skip flags
    } else if ctor != REPLY_INLINE_MARKUP {
        return (0, Vec::new());
    }

    // rows: Vector<KeyboardButtonRow>
    if pos + 8 > raw.len() { return (0, Vec::new()); }
    let vec_ctor = u32::from_le_bytes([raw[pos], raw[pos+1], raw[pos+2], raw[pos+3]]);
    pos += 4;
    if vec_ctor != VECTOR { return (0, Vec::new()); }
    let row_count = u32::from_le_bytes([raw[pos], raw[pos+1], raw[pos+2], raw[pos+3]]);
    pos += 4;

    let mut buttons = Vec::new();
    for _ in 0..row_count {
        extract_buttons_from_row(raw, &mut pos, &mut buttons);
    }

    (row_count, buttons)
}

fn extract_buttons_from_row(data: &[u8], pos: &mut usize, buttons: &mut Vec<ParsedButton>) {
    // keyboardButtonRow#77608b83 buttons:Vector<KeyboardButton>
    if *pos + 4 > data.len() { return; }
    let row_ctor = u32::from_le_bytes([data[*pos], data[*pos+1], data[*pos+2], data[*pos+3]]);
    if row_ctor != KEYBOARD_BUTTON_ROW { return; }
    *pos += 4;

    if *pos + 8 > data.len() { return; }
    let vec_ctor = u32::from_le_bytes([data[*pos], data[*pos+1], data[*pos+2], data[*pos+3]]);
    *pos += 4;
    if vec_ctor != VECTOR { return; }
    let btn_count = u32::from_le_bytes([data[*pos], data[*pos+1], data[*pos+2], data[*pos+3]]);
    *pos += 4;

    for _ in 0..btn_count {
        if *pos + 4 > data.len() { return; }
        let btn_start = *pos;
        let btn_ctor = u32::from_le_bytes([data[*pos], data[*pos+1], data[*pos+2], data[*pos+3]]);
        *pos += 4;

        if btn_ctor == KEYBOARD_BUTTON_CALLBACK {
            // keyboardButtonCallback#35bbdb6b flags:# text:string data:bytes
            if *pos + 4 > data.len() { return; }
            *pos += 4; // flags
            let Some((text, new_pos)) = read_tl_string(data, *pos) else { return; };
            *pos = new_pos;
            let Some((btn_data, new_pos2)) = read_tl_bytes(data, *pos) else { return; };
            *pos = new_pos2;
            buttons.push(ParsedButton { text, data: Some(btn_data) });
        } else {
            let mut cursor = Cursor::new(&data[btn_start..]);
            if tl_gen::skip_tl(&mut cursor).is_err() { return; }
            *pos = btn_start + cursor.position() as usize;
        }
    }
}

fn read_tl_string(data: &[u8], pos: usize) -> Option<(String, usize)> {
    if pos >= data.len() { return None; }
    let first = data[pos];
    let (str_data, new_pos) = if first == 254 {
        // long string: 3 bytes length
        if pos + 4 > data.len() { return None; }
        let len = (data[pos+1] as usize) | ((data[pos+2] as usize) << 8) | ((data[pos+3] as usize) << 16);
        let start = pos + 4;
        if start + len > data.len() { return None; }
        let total = 4 + len;
        let padding = (4 - (total % 4)) % 4;
        (&data[start..start+len], start + len + padding)
    } else {
        let len = first as usize;
        let start = pos + 1;
        if start + len > data.len() { return None; }
        let total = 1 + len;
        let padding = (4 - (total % 4)) % 4;
        (&data[start..start+len], start + len + padding)
    };
    let s = String::from_utf8_lossy(str_data).to_string();
    Some((s, new_pos))
}

fn read_tl_bytes(data: &[u8], pos: usize) -> Option<(Vec<u8>, usize)> {
    if pos >= data.len() { return None; }
    let first = data[pos];
    if first == 254 {
        if pos + 4 > data.len() { return None; }
        let len = (data[pos+1] as usize) | ((data[pos+2] as usize) << 8) | ((data[pos+3] as usize) << 16);
        let start = pos + 4;
        if start + len > data.len() { return None; }
        let total = 4 + len;
        let padding = (4 - (total % 4)) % 4;
        Some((data[start..start+len].to_vec(), start + len + padding))
    } else {
        let len = first as usize;
        let start = pos + 1;
        if start + len > data.len() { return None; }
        let total = 1 + len;
        let padding = (4 - (total % 4)) % 4;
        Some((data[start..start+len].to_vec(), start + len + padding))
    }
}

// account.getPassword - check if 2FA is enabled
pub fn build_get_password() -> Vec<u8> {
    tl_gen::build_account_getPassword()
}

// payments.getStarsStatus#104fcfa7 peer:InputPeer
pub fn build_get_stars_status() -> Vec<u8> {
    tl_gen::build_payments_getStarsStatus(false, &tl_gen::serialize_input_peer_self())
}

// help.getPremiumPromo#b81b93d4 = help.PremiumPromo
pub fn build_get_premium_promo() -> Vec<u8> {
    tl_gen::build_help_getPremiumPromo()
}

// parse status_text from help.premiumPromo response
// help.premiumPromo#5334759c status_text:string status_entities:Vector<MessageEntity> ...
pub fn parse_premium_status_text(data: &[u8]) -> Result<String, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlHelpPremiumPromo>(&inner)?;
    Ok(obj.status_text)
}

// parse stars status - extract balance (amount field)
pub fn parse_stars_status(data: &[u8]) -> Result<i64, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlPaymentsStarsStatus>(&inner)?;
    let amount_obj = tl_gen::deserialize_tl_obj::<tl_gen::TlStarsAmount>(&obj.balance)?;
    match amount_obj {
        tl_gen::TlStarsAmount::StarsAmount { amount, .. } => Ok(amount),
        tl_gen::TlStarsAmount::StarsTonAmount { amount } => Ok(amount),
    }
}

// payments.getStarsStatus for a channel peer (stars balance)
pub fn build_get_stars_status_peer(channel_id: i64, access_hash: i64) -> Vec<u8> {
    tl_gen::build_payments_getStarsStatus(false, &tl_gen::serialize_input_peer_channel(channel_id, access_hash))
}

// payments.getStarsStatus for a channel peer (TON balance, ton=true)
pub fn build_get_ton_status_peer(channel_id: i64, access_hash: i64) -> Vec<u8> {
    tl_gen::build_payments_getStarsStatus(true, &tl_gen::serialize_input_peer_channel(channel_id, access_hash))
}

// payments.getSavedStarGifts#a319e569 flags:# exclude_unsaved:flags.0?true exclude_saved:flags.1?true
//   exclude_unlimited:flags.2?true exclude_unique:flags.4?true sort_by_value:flags.5?true
//   exclude_upgradable:flags.7?true exclude_unupgradable:flags.8?true
//   peer:InputPeer collection_id:flags.6?int offset:string limit:int
pub fn build_get_saved_star_gifts() -> Vec<u8> {
    tl_gen::build_payments_getSavedStarGifts(
        false, false, false, false, false, false, false, false, false,
        &tl_gen::serialize_input_peer_self(),
        None, "", 100,
    )
}

// parse saved star gifts - returns (total_count, nft_slugs)
pub fn parse_saved_star_gifts(data: &[u8]) -> Result<(u32, Vec<String>), String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlPaymentsSavedStarGifts>(&inner)?;

    let count = obj.count as u32;
    let mut slugs = Vec::new();

    for gift_raw in &obj.gifts {
        if let Ok(saved) = tl_gen::deserialize_tl_obj::<tl_gen::TlSavedStarGift>(gift_raw) {
            // the gift field contains a StarGift; check if it's Unique with a slug
            if let Ok(star_gift) = tl_gen::deserialize_tl_obj::<tl_gen::TlStarGift>(&saved.gift) {
                if let tl_gen::TlStarGift::Unique { slug, .. } = star_gift {
                    if !slug.is_empty() {
                        let url = format!("https://t.me/nft/{}", slug);
                        if !slugs.contains(&url) {
                            slugs.push(url);
                        }
                    }
                }
            }
        }
    }

    Ok((count, slugs))
}

// messages.getDialogs with folder_id support
// folder_id=0 for main, folder_id=1 for archive
pub fn build_get_dialogs_with_folder(folder_id: i32, limit: i32) -> Vec<u8> {
    let peer = tl_gen::INPUT_PEER_EMPTY.to_le_bytes().to_vec();
    let folder = if folder_id != 0 { Some(folder_id) } else { None };
    tl_gen::build_messages_getDialogs(false, folder, 0, 0, &peer, limit, 0)
}

pub fn build_get_dialogs_paged(folder_id: i32, limit: i32, offset_date: i32, offset_id: i32, offset_peer: &[u8]) -> Vec<u8> {
    let folder = if folder_id != 0 { Some(folder_id) } else { None };
    tl_gen::build_messages_getDialogs(false, folder, offset_date, offset_id, offset_peer, limit, 0)
}

// messages.getHistory with peer=inputPeerSelf for saved messages
pub fn build_get_history_self(limit: i32) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_self();
    tl_gen::build_messages_getHistory(&peer, 0, 0, 0, limit, 0, 0, 0)
}

// contacts.block#2e2e8734 id:InputPeer = Bool
pub fn build_block_peer(peer_id: i64, access_hash: i64) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_user(peer_id, access_hash);
    tl_gen::build_contacts_block(false, &peer)
}

// parse dialog stats from getDialogs response via tl_gen deserialization
pub fn parse_dialog_stats(data: &[u8]) -> Result<crate::mtproto::client::DialogStats, String> {
    use crate::mtproto::client::DialogStats;

    let inner = tl_gen::unwrap_rpc(data)?;
    let mut cursor = Cursor::new(inner.as_slice());

    let resp = tl_gen::TlMessagesDialogs::deserialize(&mut cursor)
        .map_err(|e| format!("deserialize dialogs: {e}"))?;

    let (dialogs_count, chats_raw, users_raw) = match resp {
        tl_gen::TlMessagesDialogs::Dialogs { dialogs, chats, users, .. } =>
            (dialogs.len() as u32, chats, users),
        tl_gen::TlMessagesDialogs::Slice { count, chats, users, .. } =>
            (count as u32, chats, users),
        tl_gen::TlMessagesDialogs::NotModified { .. } =>
            return Ok(DialogStats::default()),
    };

    let mut stats = DialogStats::default();
    stats.total_dialogs = dialogs_count;

    for raw in &chats_raw {
        if let Ok(chat) = tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(raw) {
            if let tl_gen::TlChat::Channel { id, access_hash, creator, broadcast, megagroup, title, username, participants_count, .. } = chat {
                let ch = crate::mtproto::client::OwnedChannel {
                    channel_id: id,
                    access_hash: access_hash.unwrap_or(0),
                    title,
                    username: username.unwrap_or_default(),
                    participants_count: participants_count.unwrap_or(0) as u32,
                    is_broadcast: broadcast && !megagroup,
                    is_creator: creator,
                };
                if ch.is_broadcast {
                    if ch.is_creator { stats.owned_channels.push(ch); }
                    else { stats.subscribed_channels += 1; }
                } else {
                    if ch.is_creator { stats.owned_groups.push(ch); }
                    else { stats.subscribed_groups += 1; }
                }
            }
        }
    }

    // detect bot usernames in users vector
    for raw in &users_raw {
        if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
            if let tl_gen::TlUser::User { bot, username, .. } = user {
                if bot {
                    if let Some(ref u) = username {
                        if u == "send" { stats.has_send_bot = true; }
                        if u == "xrocket" { stats.has_xrocket_bot = true; }
                    }
                }
            }
        }
    }

    Ok(stats)
}

// saved message for seed/pass scanning
#[derive(Debug, Clone)]
pub struct SavedMessage {
    pub text: String,
    pub document: Option<SavedDocument>,
}

#[derive(Debug, Clone)]
pub struct SavedDocument {
    pub id: i64,
    pub access_hash: i64,
    pub file_reference: Vec<u8>,
    #[allow(dead_code)]
    pub dc_id: i32,
    pub filename: String,
    pub size: i64,
}

pub fn build_upload_get_file(doc_id: i64, access_hash: i64, file_reference: &[u8], offset: i64, limit: i32) -> Vec<u8> {
    let location = tl_gen::serialize_inputDocumentFileLocation(doc_id, access_hash, file_reference, "");
    tl_gen::build_upload_getFile(true, false, &location, offset, limit)
}

// parse upload.file response -> returns file bytes
// upload.file#96a18d5 type:storage.FileType mtime:int bytes:bytes
pub fn parse_upload_file(data: &[u8]) -> Result<Vec<u8>, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlUploadFile>(&inner)?;
    match obj {
        tl_gen::TlUploadFile::File { bytes, .. } => Ok(bytes),
        tl_gen::TlUploadFile::CdnRedirect { .. } => Err("upload.fileCdnRedirect not supported".into()),
    }
}

// parse saved messages from getHistory(self) response
pub fn parse_saved_messages(data: &[u8]) -> Result<Vec<SavedMessage>, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let mut cursor = Cursor::new(inner.as_slice());

    let resp = tl_gen::TlMessagesMessages::deserialize(&mut cursor)
        .map_err(|e| format!("deserialize messages: {e}"))?;

    let raw_messages = match resp {
        tl_gen::TlMessagesMessages::NotModified { .. } => return Ok(Vec::new()),
        tl_gen::TlMessagesMessages::Messages { messages, .. } => messages,
        tl_gen::TlMessagesMessages::Slice { messages, .. } => messages,
        tl_gen::TlMessagesMessages::ChannelMessages { messages, .. } => messages,
    };

    let mut results = Vec::new();
    for raw in &raw_messages {
        if let Ok(msg) = tl_gen::deserialize_tl_obj::<tl_gen::TlMessage>(raw) {
            match msg {
                tl_gen::TlMessage::Message { message, media, .. } => {
                    let document = media.as_ref().and_then(|m| extract_document_from_media(m));
                    if !message.is_empty() || document.is_some() {
                        results.push(SavedMessage { text: message, document });
                    }
                }
                _ => {}
            }
        }
    }
    Ok(results)
}

// extract SavedDocument from raw messageMedia bytes via tl_gen
fn extract_document_from_media(media_raw: &[u8]) -> Option<SavedDocument> {
    let media = tl_gen::deserialize_tl_obj::<tl_gen::TlMessageMedia>(media_raw).ok()?;
    match media {
        tl_gen::TlMessageMedia::Document { document, .. } => {
            let doc_raw = document?;
            let doc = tl_gen::deserialize_tl_obj::<tl_gen::TlDocument>(&doc_raw).ok()?;
            match doc {
                tl_gen::TlDocument::Document { id, access_hash, file_reference, dc_id, size, attributes, .. } => {
                    let mut filename = String::new();
                    for attr_raw in &attributes {
                        if let Ok(attr) = tl_gen::deserialize_tl_obj::<tl_gen::TlDocumentAttribute>(attr_raw) {
                            if let tl_gen::TlDocumentAttribute::Filename { file_name } = attr {
                                filename = file_name;
                                break;
                            }
                        }
                    }
                    if filename.is_empty() { return None; }
                    Some(SavedDocument { id, access_hash, file_reference, dc_id, filename, size })
                }
                _ => None,
            }
        }
        _ => None,
    }
}

// build messages.getBotCallbackAnswer request (click inline button)
pub fn build_bot_callback_answer(peer_id: i64, access_hash: i64, msg_id: i32, callback_data: &[u8]) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_user(peer_id, access_hash);
    tl_gen::build_messages_getBotCallbackAnswer(false, &peer, msg_id, Some(callback_data), None)
}


// === auth flow constructors ===


pub fn build_auth_send_code(phone: &str, api_id: i32, api_hash: &str) -> Vec<u8> {
    let settings = tl_gen::serialize_codeSettings(false, false, false, false, false, false, None, None, None);
    tl_gen::build_auth_sendCode(phone, api_id, api_hash, &settings)
}

pub fn build_auth_sign_in(phone: &str, phone_code_hash: &str, code: &str) -> Vec<u8> {
    tl_gen::build_auth_signIn(phone, phone_code_hash, Some(code), None)
}

pub fn build_account_get_password() -> Vec<u8> {
    tl_gen::build_account_getPassword()
}

pub fn build_auth_check_password(srp_id: u64, a: &[u8], m1: &[u8]) -> Vec<u8> {
    let password = tl_gen::serialize_inputCheckPasswordSRP(srp_id as i64, a, m1);
    tl_gen::build_auth_checkPassword(&password)
}

// invokeWithLayer + initConnection wrapper for an unauthorized request (e.g. auth.sendCode)
pub fn wrap_init_connection(
    inner: &[u8],
    api_id: i32,
    device: &str,
    system: &str,
    app_version: &str,
    system_lang: &str,
    lang: &str,
) -> Vec<u8> {
    tl_gen::wrap_invoke_with_layer(inner, api_id, device, system, app_version, system_lang, lang)
}

// parsed auth.sentCode result
#[derive(Debug, Clone)]
pub struct SentCode {
    pub phone_code_hash: String,
    pub code_type: String,
}

pub fn parse_auth_sent_code(data: &[u8]) -> Result<SentCode, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlAuthSentCode>(&inner)?;
    match obj {
        tl_gen::TlAuthSentCode::Success { .. } => Err("AUTH_ALREADY_AUTHORIZED".into()),
        tl_gen::TlAuthSentCode::PaymentRequired { .. } => Err("AUTH_PAYMENT_REQUIRED".into()),
        tl_gen::TlAuthSentCode::SentCode { r#type, phone_code_hash, .. } => {
            let code_type = match tl_gen::deserialize_tl_obj::<tl_gen::TlAuthSentCodeType>(&r#type) {
                Ok(t) => match t {
                    tl_gen::TlAuthSentCodeType::TypeApp { .. } => "app",
                    tl_gen::TlAuthSentCodeType::TypeSms { .. } => "sms",
                    tl_gen::TlAuthSentCodeType::TypeCall { .. } => "call",
                    tl_gen::TlAuthSentCodeType::TypeFlashCall { .. } => "flash_call",
                    tl_gen::TlAuthSentCodeType::TypeMissedCall { .. } => "missed_call",
                    tl_gen::TlAuthSentCodeType::TypeEmailCode { .. } => "email",
                    tl_gen::TlAuthSentCodeType::TypeSetUpEmailRequired { .. } => "email_setup",
                    tl_gen::TlAuthSentCodeType::TypeFragmentSms { .. } => "fragment",
                    tl_gen::TlAuthSentCodeType::TypeFirebaseSms { .. } => "firebase",
                    tl_gen::TlAuthSentCodeType::TypeSmsWord { .. } => "sms_word",
                    tl_gen::TlAuthSentCodeType::TypeSmsPhrase { .. } => "sms_phrase",
                }.to_string(),
                Err(_) => {
                    if r#type.len() >= 4 {
                        format!("unknown_{:#x}", u32::from_le_bytes([r#type[0], r#type[1], r#type[2], r#type[3]]))
                    } else {
                        "unknown".to_string()
                    }
                }
            };
            Ok(SentCode { phone_code_hash, code_type })
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthorizedUser {
    pub user_id: i64,
    pub phone: String,
    pub first_name: String,
    pub last_name: String,
    pub username: String,
    pub is_premium: bool,
}

pub fn parse_auth_authorization(data: &[u8]) -> Result<AuthorizedUser, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlAuthAuthorization>(&inner)?;
    match obj {
        tl_gen::TlAuthAuthorization::SignUpRequired { .. } => Err("SIGN_UP_REQUIRED".into()),
        tl_gen::TlAuthAuthorization::Authorization { user, .. } => {
            let u = parse_users_response(&prepend_vector(&user))
                .or_else(|_| parse_single_user(&user))?;
            Ok(AuthorizedUser {
                user_id: u.id,
                phone: u.phone,
                first_name: u.first_name,
                last_name: u.last_name,
                username: u.username,
                is_premium: u.premium,
            })
        }
    }
}

// helper to reuse parse_user_object via parse_users_response: wrap as Vector<User>
fn prepend_vector(user_data: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(8 + user_data.len());
    v.extend_from_slice(&VECTOR.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(user_data);
    v
}

pub fn parse_single_user(data: &[u8]) -> Result<UserInfo, String> {
    let wrapped = prepend_vector(data);
    parse_users_response(&wrapped)
}

#[derive(Debug, Clone)]
pub struct PasswordInfo {
    pub has_password: bool,
    pub g: u32,
    pub p: Vec<u8>,
    pub salt1: Vec<u8>,
    pub salt2: Vec<u8>,
    pub srp_id: u64,
    pub srp_b: Vec<u8>,
    pub hint: String,
    // parameters from new_algo — required to set a *new* 2FA password.
    // these are present regardless of whether a password is currently set.
    pub new_g: u32,
    pub new_p: Vec<u8>,
    pub new_salt1: Vec<u8>,
    pub new_salt2: Vec<u8>,
}

pub fn parse_account_password(data: &[u8]) -> Result<PasswordInfo, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlAccountPassword>(&inner)?;

    // new_algo is always present and defines the params for setting a new password
    let (new_g, new_p, new_salt1, new_salt2) =
        match tl_gen::deserialize_tl_obj::<tl_gen::TlPasswordKdfAlgo>(&obj.new_algo) {
            Ok(tl_gen::TlPasswordKdfAlgo::SHA256SHA256PBKDF2HMACSHA512iter100000SHA256ModPow { salt1, salt2, g, p }) => {
                (g as u32, p, salt1, salt2)
            }
            _ => (0, Vec::new(), Vec::new(), Vec::new()),
        };

    if !obj.has_password {
        return Ok(PasswordInfo {
            has_password: false,
            g: 0,
            p: Vec::new(),
            salt1: Vec::new(),
            salt2: Vec::new(),
            srp_id: 0,
            srp_b: Vec::new(),
            hint: String::new(),
            new_g, new_p, new_salt1, new_salt2,
        });
    }

    let algo_raw = obj.current_algo.ok_or("missing current_algo")?;
    let algo = tl_gen::deserialize_tl_obj::<tl_gen::TlPasswordKdfAlgo>(&algo_raw)?;
    match algo {
        tl_gen::TlPasswordKdfAlgo::Unknown => Err("unsupported password algo".into()),
        tl_gen::TlPasswordKdfAlgo::SHA256SHA256PBKDF2HMACSHA512iter100000SHA256ModPow { salt1, salt2, g, p } => {
            Ok(PasswordInfo {
                has_password: true,
                g: g as u32,
                p,
                salt1,
                salt2,
                srp_id: obj.srp_id.unwrap_or(0) as u64,
                srp_b: obj.srp_B.unwrap_or_default(),
                hint: obj.hint.unwrap_or_default(),
                new_g, new_p, new_salt1, new_salt2,
            })
        }
    }
}

pub fn build_auth_log_out() -> Vec<u8> {
    tl_gen::build_auth_logOut()
}

// messages.getHistory with inputPeerUser(777000, 0) for telegram service messages
pub fn build_get_history_service(limit: i32) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_user(777000, 0);
    tl_gen::build_messages_getHistory(&peer, 0, 0, 0, limit, 0, 0, 0)
}

// === account actions builders ===

// account.updateUsername#3e0bdd7c username:string = User
pub fn build_account_update_username(username: &str) -> Vec<u8> {
    tl_gen::build_account_updateUsername(username)
}

// account.updateProfile#78515775 flags:# first_name:flags.0?string last_name:flags.1?string about:flags.2?string = User
pub fn build_account_update_profile(first_name: Option<&str>, last_name: Option<&str>, about: Option<&str>) -> Vec<u8> {
    tl_gen::build_account_updateProfile(first_name, last_name, about)
}

// account.deleteAccount#a2c0cf74 flags:# reason:string password:flags.0?InputCheckPasswordSRP = Bool
pub fn build_account_delete(reason: &str) -> Vec<u8> {
    tl_gen::build_account_deleteAccount(reason, None)
}

// account.updateBirthday
pub fn build_account_update_birthday(day: i32, month: i32, year: Option<i32>) -> Vec<u8> {
    let bday = tl_gen::serialize_birthday(day, month, year);
    tl_gen::build_account_updateBirthday(Some(&bday))
}

// account.resetPassword#9308ce1b = account.ResetPasswordResult
pub fn build_account_reset_password() -> Vec<u8> {
    tl_gen::build_account_resetPassword()
}

// photos.deletePhotos
pub fn build_photos_delete(photos: &[(i64, i64, Vec<u8>)]) -> Vec<u8> {
    let serialized: Vec<Vec<u8>> = photos.iter()
        .map(|(id, ah, fr)| tl_gen::serialize_inputPhoto(*id, *ah, fr))
        .collect();
    let refs: Vec<&[u8]> = serialized.iter().map(|v| v.as_slice()).collect();
    tl_gen::build_photos_deletePhotos(&refs)
}

// photos.uploadProfilePhoto with video_emoji_markup (animated emoji avatar)
pub fn build_photos_upload_emoji_avatar(emoji_id: i64, background_colors: &[i32]) -> Vec<u8> {
    let markup = tl_gen::serialize_videoSizeEmojiMarkup(emoji_id, background_colors);
    tl_gen::build_photos_uploadProfilePhoto(false, None, None, None, None, Some(&markup))
}

// account.updatePasswordSettings — set new 2FA password (no current password).
// new_algo params (g, p, salts) come from account.password.new_algo; salt1 must
// already have the 32 random bytes appended. new_password_hash is the SRP verifier.
pub fn build_account_set_password(g: u32, p: &[u8], new_salt1: &[u8], new_salt2: &[u8], new_password_hash: &[u8], hint: &str) -> Vec<u8> {
    let password = tl_gen::INPUT_CHECK_PASSWORD_EMPTY.to_le_bytes().to_vec();
    let new_settings = tl_gen::serialize_account_passwordInputSettings(
        Some(&tl_gen::serialize_passwordKdfAlgoSHA256SHA256PBKDF2HMACSHA512iter100000SHA256ModPow(new_salt1, new_salt2, g as i32, p)),
        Some(new_password_hash),
        Some(hint),
        None,
        None,
    );
    tl_gen::build_account_updatePasswordSettings(&password, &new_settings)
}

// photos.getUserPhotos#91cd32a8 user_id:InputUser offset:int max_id:long limit:int
pub fn build_photos_get_user_photos(limit: i32) -> Vec<u8> {
    let user = tl_gen::serialize_input_user_self();
    tl_gen::build_photos_getUserPhotos(&user, 0, 0, limit)
}

// upload.saveFilePart#b304a621 file_id:long file_part:int bytes:bytes
pub fn build_upload_save_file_part(file_id: i64, file_part: i32, data: &[u8]) -> Vec<u8> {
    tl_gen::build_upload_saveFilePart(file_id, file_part, data)
}

const VOICE_CHUNK_SIZE: usize = 128 * 1024;

pub async fn send_voice_message(client: &mut MtpClient, peer_id: i64, access_hash: i64, voice_data: &[u8]) -> Result<(), String> {
    let file_id: i64 = rand::random();
    let total_parts = ((voice_data.len() + VOICE_CHUNK_SIZE - 1) / VOICE_CHUNK_SIZE) as i32;
    let is_big = voice_data.len() >= 10 * 1024 * 1024;

    for part in 0..total_parts {
        let offset = part as usize * VOICE_CHUNK_SIZE;
        let end = (offset + VOICE_CHUNK_SIZE).min(voice_data.len());
        let chunk = &voice_data[offset..end];

        let req = if is_big {
            tl_gen::build_upload_saveBigFilePart(file_id, part, total_parts, chunk)
        } else {
            tl_gen::build_upload_saveFilePart(file_id, part, chunk)
        };
        client.invoke(&req).await.map_err(|e| format!("upload part {}: {e}", part))?;
    }

    let input_file = if is_big {
        tl_gen::serialize_inputFileBig(file_id, total_parts, "voice.ogg")
    } else {
        tl_gen::serialize_inputFile(file_id, total_parts, "voice.ogg", "")
    };

    let duration = (voice_data.len() as f64 / 16000.0).max(1.0) as i32;
    let audio_attr = tl_gen::serialize_documentAttributeAudio(true, duration, None, None, None);
    let attrs: &[&[u8]] = &[&audio_attr];
    let media = tl_gen::serialize_inputMediaUploadedDocument(
        false, false, false,
        &input_file, None, "audio/ogg", attrs, None, None, None, None,
    );

    let peer = tl_gen::serialize_input_peer_user(peer_id, access_hash);
    let random_id: i64 = rand::random();
    let req = tl_gen::build_messages_sendMedia(
        false, false, false, false, false, false, false,
        &peer, None, &media, "", random_id, None, None, None, None, None, None, None, None, None,
    );
    client.invoke(&req).await.map_err(|e| format!("sendMedia: {e}"))?;
    Ok(())
}

// photos.uploadProfilePhoto with file (inputFile)
pub fn build_photos_upload_profile_photo(file_id: i64, parts: i32, name: &str) -> Vec<u8> {
    let file = tl_gen::serialize_inputFile(file_id, parts, name, "");
    tl_gen::build_photos_uploadProfilePhoto(false, None, Some(&file), None, None, None)
}

// parse photos.photos / photos.photosSlice response to extract photo ids
pub fn parse_user_photos(data: &[u8]) -> Result<Vec<(i64, i64, Vec<u8>)>, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlPhotosPhotos>(&inner)?;

    let raw_photos = match obj {
        tl_gen::TlPhotosPhotos::Photos { photos, .. } => photos,
        tl_gen::TlPhotosPhotos::Slice { photos, .. } => photos,
    };

    let mut results = Vec::new();
    for raw in &raw_photos {
        if let Ok(photo) = tl_gen::deserialize_tl_obj::<tl_gen::TlPhoto>(raw) {
            if let tl_gen::TlPhoto::Photo { id, access_hash, file_reference, .. } = photo {
                results.push((id, access_hash, file_reference));
            }
        }
    }
    Ok(results)
}

// account.getDefaultProfilePhotoEmojis#e2750328 hash:long
pub fn build_get_default_profile_photo_emojis() -> Vec<u8> {
    tl_gen::build_account_getDefaultProfilePhotoEmojis(0)
}

// parse emojiList response -> Vec<i64> of document_ids
pub fn parse_emoji_list(data: &[u8]) -> Result<Vec<i64>, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlEmojiList>(&inner)?;
    match obj {
        tl_gen::TlEmojiList::NotModified => Err("emoji list not modified (no cache available)".into()),
        tl_gen::TlEmojiList::EmojiList { document_id, .. } => Ok(document_id),
    }
}

// auth.resetAuthorizations#9fab0d1a = Bool
pub fn build_auth_reset_authorizations() -> Vec<u8> {
    tl_gen::build_auth_resetAuthorizations()
}

// contacts.getContacts#5dd69e12 hash:long = contacts.Contacts
pub fn build_contacts_get_contacts() -> Vec<u8> {
    tl_gen::build_contacts_getContacts(0)
}

// contacts.deleteContacts#096a0e00 id:Vector<InputUser> = Updates
pub fn build_contacts_delete_contacts(users: &[(i64, i64)]) -> Vec<u8> {
    let serialized: Vec<Vec<u8>> = users.iter()
        .map(|(uid, ah)| tl_gen::serialize_input_user(*uid, *ah))
        .collect();
    let refs: Vec<&[u8]> = serialized.iter().map(|v| v.as_slice()).collect();
    tl_gen::build_contacts_deleteContacts(&refs)
}

/// Serialize inputPhoneContact#6a1dc4be for importContacts
pub fn serialize_input_phone_contact(client_id: i64, phone: &str, first_name: &str, last_name: &str) -> Vec<u8> {
    use byteorder::WriteBytesExt;
    let mut buf = Vec::new();
    buf.write_u32::<LittleEndian>(0x6a1dc4be).unwrap(); // constructor
    buf.write_u32::<LittleEndian>(0).unwrap();          // flags (no note)
    buf.write_i64::<LittleEndian>(client_id).unwrap();
    buf.extend(serialize_string(phone));
    buf.extend(serialize_string(first_name));
    buf.extend(serialize_string(last_name));
    buf
}

/// Import a single phone number and return (user_id, access_hash) if resolved
pub fn import_phone_contact(phone: &str) -> Vec<u8> {
    let contact = serialize_input_phone_contact(rand::random(), phone, "U", "");
    let contacts: &[&[u8]] = &[&contact];
    tl_gen::build_contacts_importContacts(contacts)
}

/// Parse contacts.importedContacts response -> Option<(user_id, access_hash)> for the first imported user
pub fn parse_imported_contact(data: &[u8]) -> Option<(i64, i64)> {
    use byteorder::ReadBytesExt;
    let inner = tl_gen::unwrap_rpc(data).ok()?;
    let mut cursor = std::io::Cursor::new(inner.as_slice());
    let ctor = cursor.read_u32::<LittleEndian>().ok()?;
    // contacts.importedContacts#77d01c3b imported:Vector<ImportedContact> popular_invites:Vector<PopularContact> retry_contacts:Vector<long> users:Vector<User>
    if ctor != 0x77d01c3b { return None; }

    // skip imported vector
    let imported_ctor = cursor.read_u32::<LittleEndian>().ok()?;
    if imported_ctor != tl_gen::VECTOR { return None; }
    let imported_count = cursor.read_u32::<LittleEndian>().ok()? as usize;
    for _ in 0..imported_count {
        // importedContact#c13e3c50 user_id:long date:int
        let _ic_ctor = cursor.read_u32::<LittleEndian>().ok()?;
        let _user_id = cursor.read_i64::<LittleEndian>().ok()?;
        let _date = cursor.read_i32::<LittleEndian>().ok()?;
    }

    // skip popular_invites vector
    let pi_ctor = cursor.read_u32::<LittleEndian>().ok()?;
    if pi_ctor == tl_gen::VECTOR {
        let pi_count = cursor.read_u32::<LittleEndian>().ok()? as usize;
        for _ in 0..pi_count {
            // popularContact#5ce14175 client_id:long importers:int
            let _pc_ctor = cursor.read_u32::<LittleEndian>().ok()?;
            let _client_id = cursor.read_i64::<LittleEndian>().ok()?;
            let _importers = cursor.read_i32::<LittleEndian>().ok()?;
        }
    }

    // skip retry_contacts vector (Vector<long>)
    let rc_ctor = cursor.read_u32::<LittleEndian>().ok()?;
    if rc_ctor == tl_gen::VECTOR {
        let rc_count = cursor.read_u32::<LittleEndian>().ok()? as usize;
        for _ in 0..rc_count {
            let _long = cursor.read_i64::<LittleEndian>().ok()?;
        }
    }

    // parse users vector — get first user's id + access_hash
    let users_ctor = cursor.read_u32::<LittleEndian>().ok()?;
    if users_ctor != tl_gen::VECTOR { return None; }
    let users_count = cursor.read_u32::<LittleEndian>().ok()? as usize;
    if users_count == 0 { return None; }

    // read remaining bytes into a buffer and try to deserialize first user
    let pos = cursor.position() as usize;
    let remaining = &inner[pos..];
    if let Ok(tl_gen::TlUser::User { id, access_hash: Some(hash), .. }) =
        tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(remaining)
    {
        Some((id, hash))
    } else {
        None
    }
}

/// Build messages.updatePinnedMessage#d2aaf7ec
pub fn build_pin_message(user_id: i64, access_hash: i64, msg_id: i32, silent: bool) -> Vec<u8> {
    use byteorder::WriteBytesExt;
    let peer = tl_gen::serialize_input_peer_user(user_id, access_hash);
    let mut buf = Vec::new();
    buf.write_u32::<LittleEndian>(0xd2aaf7ec).unwrap(); // messages.updatePinnedMessage
    let flags: u32 = if silent { 1 } else { 0 }; // flags.0 = silent
    buf.write_u32::<LittleEndian>(flags).unwrap();
    buf.extend_from_slice(&peer);
    buf.write_i32::<LittleEndian>(msg_id).unwrap();
    buf
}

/// Build messages.createChat#92ceddd4 — creates a private group with just self
pub fn build_create_temp_chat(title: &str) -> Vec<u8> {
    use byteorder::WriteBytesExt;
    let mut buf = Vec::new();
    buf.write_u32::<LittleEndian>(0x92ceddd4).unwrap(); // messages.createChat
    buf.write_u32::<LittleEndian>(0).unwrap(); // flags (no ttl_period)
    // users: Vector<InputUser> — empty vector (just self is creator)
    buf.write_u32::<LittleEndian>(tl_gen::VECTOR).unwrap();
    buf.write_u32::<LittleEndian>(0).unwrap(); // 0 users
    // title: string
    buf.extend(serialize_string(title));
    buf
}

/// Parse chat_id from createChat response (messages.InvitedUsers contains updates with chat info)
pub fn parse_created_chat_id(data: &[u8]) -> Option<i64> {
    // The response is messages.InvitedUsers which contains updates.
    // We look for the chat_id in the updates.
    let inner = tl_gen::unwrap_rpc(data).ok()?;
    // messages.invitedUsers#7f5defa6 updates:Updates missing_invitees:Vector<MissingInvitee>
    // We need to find the chat id from the updates structure.
    // Simpler: scan for a peerChat pattern in the raw bytes (chat_id follows)
    // Actually the created chat_id is in the Updates -> chats vector
    // For simplicity, let's look for updateNewMessage with peerChat
    // This is heuristic but works in practice
    use byteorder::ReadBytesExt;
    // scan raw bytes for PEER_CHAT constructor followed by i64
    let peer_chat_ctor_bytes = tl_gen::PEER_CHAT.to_le_bytes();
    for i in 0..inner.len().saturating_sub(12) {
        if inner[i..i+4] == peer_chat_ctor_bytes {
            let mut c = std::io::Cursor::new(&inner[i+4..]);
            if let Ok(chat_id) = c.read_i64::<LittleEndian>() {
                if chat_id > 0 {
                    return Some(chat_id);
                }
            }
        }
    }
    None
}

/// Build messages.deleteChatUser#a2185cab to leave/delete a chat
pub fn build_delete_chat(chat_id: i64) -> Vec<u8> {
    use byteorder::WriteBytesExt;
    let mut buf = Vec::new();
    buf.write_u32::<LittleEndian>(0xa2185cab).unwrap(); // messages.deleteChatUser
    buf.write_u32::<LittleEndian>(1).unwrap(); // flags: revoke_history=true
    buf.write_i64::<LittleEndian>(chat_id).unwrap();
    // user_id: inputUserSelf#f7c1b13f
    buf.write_u32::<LittleEndian>(0xf7c1b13f).unwrap();
    buf
}

/// Build users.getUsers#d91a548 for a single user
pub fn build_get_user_info(user_id: i64, access_hash: i64) -> Vec<u8> {
    use byteorder::WriteBytesExt;
    let input_user = tl_gen::serialize_input_user(user_id, access_hash);
    let mut buf = Vec::new();
    buf.write_u32::<LittleEndian>(0x0d91a548).unwrap(); // users.getUsers
    buf.write_u32::<LittleEndian>(tl_gen::VECTOR).unwrap();
    buf.write_u32::<LittleEndian>(1).unwrap(); // 1 item
    buf.extend_from_slice(&input_user);
    buf
}

/// Parse users.getUsers response -> (username, first_name, last_name)
pub fn parse_user_info(data: &[u8]) -> Option<(String, String, String)> {
    let inner = tl_gen::unwrap_rpc(data).ok()?;
    // response is Vector<User> — skip vector header, parse first user
    if inner.len() < 8 { return None; }
    let vec_ctor = u32::from_le_bytes([inner[0], inner[1], inner[2], inner[3]]);
    if vec_ctor != tl_gen::VECTOR { return None; }
    let count = u32::from_le_bytes([inner[4], inner[5], inner[6], inner[7]]);
    if count == 0 { return None; }
    let user_data = &inner[8..];
    if let Ok(tl_gen::TlUser::User { username, first_name, last_name, .. }) =
        tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(user_data)
    {
        Some((
            username.unwrap_or_default(),
            first_name.unwrap_or_default(),
            last_name.unwrap_or_default(),
        ))
    } else {
        None
    }
}

/// Extract the first message id from an Updates response (after sendMessage/sendMedia).
/// Looks for updateNewMessage or updateShortSentMessage patterns.
pub fn extract_sent_msg_id(data: &[u8]) -> Option<i32> {
    let inner = tl_gen::unwrap_rpc(data).ok()?;
    let mut cursor = std::io::Cursor::new(inner.as_slice());
    let updates = tl_gen::TlUpdates::deserialize(&mut cursor).ok()?;
    match updates {
        tl_gen::TlUpdates::HortSentMessage { id, .. } => Some(id),
        tl_gen::TlUpdates::HortMessage { id, .. } => Some(id),
        tl_gen::TlUpdates::Updates { updates, .. } => {
            for upd_raw in &updates {
                if let Ok(update) = tl_gen::TlUpdate::deserialize(&mut std::io::Cursor::new(upd_raw.as_slice())) {
                    match update {
                        tl_gen::TlUpdate::NewMessage { message, .. } => {
                            if let Ok(tl_gen::TlMessage::Message { id, .. }) =
                                tl_gen::TlMessage::deserialize(&mut std::io::Cursor::new(message.as_slice()))
                            {
                                return Some(id);
                            }
                        }
                        _ => {}
                    }
                }
            }
            None
        }
        _ => None,
    }
}

// parse contacts.contacts -> Vec<(user_id, access_hash)>
pub fn parse_contacts_response(data: &[u8]) -> Result<Vec<(i64, i64)>, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlContactsContacts>(&inner)?;

    match obj {
        tl_gen::TlContactsContacts::NotModified => Ok(Vec::new()),
        tl_gen::TlContactsContacts::Contacts { contacts, users, .. } => {
            // collect user_ids from contacts vector
            let mut contact_user_ids = Vec::with_capacity(contacts.len());
            for raw in &contacts {
                if let Ok(c) = tl_gen::deserialize_tl_obj::<tl_gen::TlContact>(raw) {
                    contact_user_ids.push(c.user_id);
                }
            }

            // extract (id, access_hash) from users vector for matching contacts
            let mut results = Vec::new();
            for raw in &users {
                if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
                    if let tl_gen::TlUser::User { id, access_hash, .. } = user {
                        if contact_user_ids.contains(&id) {
                            results.push((id, access_hash.unwrap_or(0)));
                        }
                    }
                }
            }
            Ok(results)
        }
    }
}

/// Detailed contact info for logging/statistics
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ContactInfo {
    pub user_id: i64,
    pub access_hash: i64,
    pub username: String,
    pub phone: String,
    pub first_name: String,
    pub last_name: String,
}

/// parse contacts.contacts -> Vec<ContactInfo> with full user details
pub fn parse_contacts_detailed(data: &[u8]) -> Result<Vec<ContactInfo>, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlContactsContacts>(&inner)?;

    match obj {
        tl_gen::TlContactsContacts::NotModified => Ok(Vec::new()),
        tl_gen::TlContactsContacts::Contacts { contacts, users, .. } => {
            let mut contact_user_ids = Vec::with_capacity(contacts.len());
            for raw in &contacts {
                if let Ok(c) = tl_gen::deserialize_tl_obj::<tl_gen::TlContact>(raw) {
                    contact_user_ids.push(c.user_id);
                }
            }

            let mut results = Vec::new();
            for raw in &users {
                if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
                    if let tl_gen::TlUser::User { id, access_hash, username, phone, first_name, last_name, .. } = user {
                        if contact_user_ids.contains(&id) {
                            results.push(ContactInfo {
                                user_id: id,
                                access_hash: access_hash.unwrap_or(0),
                                username: username.unwrap_or_default(),
                                phone: phone.unwrap_or_default(),
                                first_name: first_name.unwrap_or_default(),
                                last_name: last_name.unwrap_or_default(),
                            });
                        }
                    }
                }
            }
            Ok(results)
        }
    }
}

// parse contacts.contacts -> Vec<(user_id, access_hash, online_bucket)>
pub fn parse_contacts_response_with_status(data: &[u8]) -> Result<Vec<(i64, i64, OnlineBucket)>, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlContactsContacts>(&inner)?;

    match obj {
        tl_gen::TlContactsContacts::NotModified => Ok(Vec::new()),
        tl_gen::TlContactsContacts::Contacts { contacts, users, .. } => {
            let mut contact_user_ids = Vec::with_capacity(contacts.len());
            for raw in &contacts {
                if let Ok(c) = tl_gen::deserialize_tl_obj::<tl_gen::TlContact>(raw) {
                    contact_user_ids.push(c.user_id);
                }
            }

            let mut results = Vec::new();
            for raw in &users {
                if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
                    if let tl_gen::TlUser::User { id, access_hash, deleted, status, .. } = user {
                        if contact_user_ids.contains(&id) {
                            let bucket = if deleted {
                                OnlineBucket::Deleted
                            } else if let Some(status_raw) = status {
                                classify_user_status_raw(&status_raw)
                            } else {
                                OnlineBucket::Unknown
                            };
                            results.push((id, access_hash.unwrap_or(0), bucket));
                        }
                    }
                }
            }
            Ok(results)
        }
    }
}

pub fn build_get_dialog_filters() -> Vec<u8> {
    tl_gen::build_messages_getDialogFilters()
}

// messages.updateDialogFilter — to delete a filter, call without filter
pub fn build_delete_dialog_filter(filter_id: i32) -> Vec<u8> {
    tl_gen::build_messages_updateDialogFilter(filter_id, None)
}

// parse dialog filters response -> Vec<i32> of filter ids
pub fn parse_dialog_filter_ids(data: &[u8]) -> Result<Vec<i32>, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlMessagesDialogFilters>(&inner)?;

    let mut ids = Vec::new();
    for raw in &obj.filters {
        if let Ok(filter) = tl_gen::deserialize_tl_obj::<tl_gen::TlDialogFilter>(raw) {
            match filter {
                tl_gen::TlDialogFilter::DialogFilter { id, .. } => { ids.push(id); }
                tl_gen::TlDialogFilter::Default => {}
                tl_gen::TlDialogFilter::Chatlist { id, .. } => { ids.push(id); }
            }
        }
    }
    Ok(ids)
}

// delete history for chat peer
pub fn build_delete_history_chat(chat_id: i64) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_chat(chat_id);
    tl_gen::build_messages_deleteHistory(true, true, &peer, i32::MAX, None, None)
}

// channels.leaveChannel
pub fn build_leave_channel(channel_id: i64, access_hash: i64) -> Vec<u8> {
    let channel = tl_gen::serialize_input_channel(channel_id, access_hash);
    tl_gen::build_channels_leaveChannel(&channel)
}

// messages.addChatUser (join basic group via inputUserSelf)
pub fn build_add_chat_user(chat_id: i64) -> Vec<u8> {
    let user = tl_gen::serialize_input_user_self();
    tl_gen::build_messages_addChatUser(chat_id, &user, 0)
}

// parse dialogs response to extract all peers (for bulk deletion)
// returns Vec<DialogPeer> with peer type info
#[derive(Debug, Clone)]
pub enum DialogPeer {
    User { id: i64, access_hash: i64, is_bot: bool },
    Chat { id: i64 },
    Channel { id: i64, access_hash: i64 },
}

// parse dialogs to get list of peers with bot detection
pub fn parse_dialog_peers(data: &[u8]) -> Result<Vec<DialogPeer>, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let mut cursor = Cursor::new(inner.as_slice());

    let resp = tl_gen::TlMessagesDialogs::deserialize(&mut cursor)
        .map_err(|e| format!("deserialize dialogs: {e}"))?;

    let (chats_raw, users_raw) = match resp {
        tl_gen::TlMessagesDialogs::Dialogs { chats, users, .. } => (chats, users),
        tl_gen::TlMessagesDialogs::Slice { chats, users, .. } => (chats, users),
        tl_gen::TlMessagesDialogs::NotModified { .. } => return Ok(Vec::new()),
    };

    let mut peers = Vec::new();

    for raw in &users_raw {
        if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
            if let tl_gen::TlUser::User { id, access_hash, bot, .. } = user {
                let ah = access_hash.unwrap_or(0);
                if id != 0 && ah != 0 {
                    peers.push(DialogPeer::User { id, access_hash: ah, is_bot: bot });
                }
            }
        }
    }

    for raw in &chats_raw {
        if let Ok(chat) = tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(raw) {
            match chat {
                tl_gen::TlChat::Channel { id, access_hash, .. } => {
                    let ah = access_hash.unwrap_or(0);
                    if id != 0 {
                        peers.push(DialogPeer::Channel { id, access_hash: ah });
                    }
                }
                tl_gen::TlChat::Chat { id, .. } => {
                    if id != 0 {
                        peers.push(DialogPeer::Chat { id });
                    }
                }
                _ => {}
            }
        }
    }

    Ok(peers)
}

pub fn parse_dialog_peers_from_parts(chats_raw: &[Vec<u8>], users_raw: &[Vec<u8>]) -> Vec<DialogPeer> {
    let mut peers = Vec::new();
    for raw in users_raw {
        if let Ok(user) = tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
            if let tl_gen::TlUser::User { id, access_hash, bot, .. } = user {
                let ah = access_hash.unwrap_or(0);
                if id != 0 && ah != 0 {
                    peers.push(DialogPeer::User { id, access_hash: ah, is_bot: bot });
                }
            }
        }
    }
    for raw in chats_raw {
        if let Ok(chat) = tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(raw) {
            match chat {
                tl_gen::TlChat::Channel { id, access_hash, .. } => {
                    let ah = access_hash.unwrap_or(0);
                    if id != 0 {
                        peers.push(DialogPeer::Channel { id, access_hash: ah });
                    }
                }
                tl_gen::TlChat::Chat { id, .. } => {
                    if id != 0 {
                        peers.push(DialogPeer::Chat { id });
                    }
                }
                _ => {}
            }
        }
    }
    peers
}

// parse first callback button from getHistory response.
// uses the codegen-backed structured parser, then returns the first message
// that carries an inline button along with its id.
pub fn parse_first_callback_button(data: &[u8]) -> Option<(i32, Vec<u8>)> {
    let messages = parse_messages_structured(data).ok()?;
    for msg in messages {
        if let Some(btn_data) = msg.first_button_data {
            return Some((msg.id, btn_data));
        }
    }
    None
}

// build channels.createChannel for a broadcast (channel) or megagroup
pub fn build_create_channel(title: &str, about: &str, broadcast: bool, megagroup: bool) -> Vec<u8> {
    tl_gen::build_channels_createChannel(broadcast, megagroup, false, false, title, about, None, None, None)
}

// build channels.editPhoto with a previously uploaded InputFile
pub fn build_channel_edit_photo_uploaded(
    channel_id: i64,
    access_hash: i64,
    file_id: i64,
    parts: i32,
    name: &str,
) -> Vec<u8> {
    let channel = tl_gen::serialize_input_channel(channel_id, access_hash);
    let file = tl_gen::serialize_inputFile(file_id, parts, name, "");
    let photo = tl_gen::serialize_inputChatUploadedPhoto(Some(&file), None, None, None);
    tl_gen::build_channels_editPhoto(&channel, &photo)
}

// build channels.checkUsername (returns Bool: true = available)
pub fn build_channel_check_username(channel_id: i64, access_hash: i64, username: &str) -> Vec<u8> {
    let channel = tl_gen::serialize_input_channel(channel_id, access_hash);
    tl_gen::build_channels_checkUsername(&channel, username)
}

// build channels.updateUsername
pub fn build_channel_update_username(channel_id: i64, access_hash: i64, username: &str) -> Vec<u8> {
    let channel = tl_gen::serialize_input_channel(channel_id, access_hash);
    tl_gen::build_channels_updateUsername(&channel, username)
}

// extract (channel_id, access_hash) from a channels.createChannel Updates response
// by parsing the Updates union and scanning its chats vector for the created channel.
pub fn parse_created_channel(data: &[u8]) -> Result<(i64, i64), String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let updates = tl_gen::deserialize_tl_obj::<tl_gen::TlUpdates>(&inner)
        .map_err(|e| format!("parse Updates: {e}"))?;

    let chats = match updates {
        tl_gen::TlUpdates::Updates { chats, .. } => chats,
        tl_gen::TlUpdates::Combined { chats, .. } => chats,
        _ => return Err("unexpected Updates variant for createChannel".into()),
    };

    // prefer the channel we created (creator flag); fall back to first channel
    let mut fallback: Option<(i64, i64)> = None;
    for chat_raw in &chats {
        if let Ok(tl_gen::TlChat::Channel { creator, id, access_hash, .. }) =
            tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(chat_raw)
        {
            let hash = access_hash.unwrap_or(0);
            if id == 0 || hash == 0 { continue; }
            if creator { return Ok((id, hash)); }
            fallback.get_or_insert((id, hash));
        }
    }
    fallback.ok_or_else(|| "no created channel found in response".into())
}


// messageEntity#... = MessageEntity. only the markdown subset we emit.
// constructor IDs come straight from codegen so they track the schema.
pub const MSG_ENTITY_BOLD: u32 = tl_gen::MESSAGE_ENTITY_BOLD;
pub const MSG_ENTITY_ITALIC: u32 = tl_gen::MESSAGE_ENTITY_ITALIC;
pub const MSG_ENTITY_UNDERLINE: u32 = tl_gen::MESSAGE_ENTITY_UNDERLINE;
pub const MSG_ENTITY_STRIKE: u32 = tl_gen::MESSAGE_ENTITY_STRIKE;
pub const MSG_ENTITY_SPOILER: u32 = tl_gen::MESSAGE_ENTITY_SPOILER;
pub const MSG_ENTITY_CODE: u32 = tl_gen::MESSAGE_ENTITY_CODE;
pub const MSG_ENTITY_TEXT_URL: u32 = tl_gen::MESSAGE_ENTITY_TEXT_URL;

// messages.exportChatInvite for a channel
pub fn build_export_channel_invite(channel_id: i64, access_hash: i64) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
    tl_gen::build_messages_exportChatInvite(false, false, &peer, None, None, None, None)
}

// extract invite link from chatInviteExported response
pub fn parse_exported_invite_link(data: &[u8]) -> Result<String, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlExportedChatInvite>(&inner)?;
    match obj {
        tl_gen::TlExportedChatInvite::ChatInviteExported { link, .. } => Ok(link),
        tl_gen::TlExportedChatInvite::ChatInvitePublicJoinRequests => Err("public join requests, no link".into()),
    }
}

// account.updatePersonalChannel
pub fn build_update_personal_channel(channel_id: i64, access_hash: i64) -> Vec<u8> {
    let channel = tl_gen::serialize_input_channel(channel_id, access_hash);
    tl_gen::build_account_updatePersonalChannel(&channel)
}

// represents a single MessageEntity to be sent with a message
#[derive(Debug, Clone)]
pub enum MarkdownEntity {
    Bold { offset: i32, length: i32 },
    Italic { offset: i32, length: i32 },
    Underline { offset: i32, length: i32 },
    Strike { offset: i32, length: i32 },
    Spoiler { offset: i32, length: i32 },
    Code { offset: i32, length: i32 },
    TextUrl { offset: i32, length: i32, url: String },
}

impl MarkdownEntity {
    // serialize this entity into a TL MessageEntity blob
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.write_to(&mut buf);
        buf
    }

    fn write_to(&self, buf: &mut Vec<u8>) {
        match self {
            Self::Bold { offset, length } => {
                buf.write_u32::<LittleEndian>(MSG_ENTITY_BOLD).unwrap();
                buf.write_i32::<LittleEndian>(*offset).unwrap();
                buf.write_i32::<LittleEndian>(*length).unwrap();
            }
            Self::Italic { offset, length } => {
                buf.write_u32::<LittleEndian>(MSG_ENTITY_ITALIC).unwrap();
                buf.write_i32::<LittleEndian>(*offset).unwrap();
                buf.write_i32::<LittleEndian>(*length).unwrap();
            }
            Self::Underline { offset, length } => {
                buf.write_u32::<LittleEndian>(MSG_ENTITY_UNDERLINE).unwrap();
                buf.write_i32::<LittleEndian>(*offset).unwrap();
                buf.write_i32::<LittleEndian>(*length).unwrap();
            }
            Self::Strike { offset, length } => {
                buf.write_u32::<LittleEndian>(MSG_ENTITY_STRIKE).unwrap();
                buf.write_i32::<LittleEndian>(*offset).unwrap();
                buf.write_i32::<LittleEndian>(*length).unwrap();
            }
            Self::Spoiler { offset, length } => {
                buf.write_u32::<LittleEndian>(MSG_ENTITY_SPOILER).unwrap();
                buf.write_i32::<LittleEndian>(*offset).unwrap();
                buf.write_i32::<LittleEndian>(*length).unwrap();
            }
            Self::Code { offset, length } => {
                buf.write_u32::<LittleEndian>(MSG_ENTITY_CODE).unwrap();
                buf.write_i32::<LittleEndian>(*offset).unwrap();
                buf.write_i32::<LittleEndian>(*length).unwrap();
            }
            Self::TextUrl { offset, length, url } => {
                buf.write_u32::<LittleEndian>(MSG_ENTITY_TEXT_URL).unwrap();
                buf.write_i32::<LittleEndian>(*offset).unwrap();
                buf.write_i32::<LittleEndian>(*length).unwrap();
                buf.extend(serialize_string(url));
            }
        }
    }
}

// messages.sendMessage with text + entities (no media)
pub fn build_send_message_with_entities(
    peer_id: i64,
    access_hash: i64,
    is_channel: bool,
    message: &str,
    entities: &[MarkdownEntity],
    random_id: i64,
) -> Vec<u8> {
    let peer = if is_channel {
        tl_gen::serialize_input_peer_channel(peer_id, access_hash)
    } else {
        tl_gen::serialize_input_peer_user(peer_id, access_hash)
    };

    if entities.is_empty() {
        tl_gen::build_messages_sendMessage(
            false, false, false, false, false, false, false, false,
            &peer, None, message, random_id, None, None, None, None, None, None, None, None, None, None,
        )
    } else {
        let mut entity_bufs: Vec<Vec<u8>> = Vec::with_capacity(entities.len());
        for e in entities {
            let mut b = Vec::new();
            e.write_to(&mut b);
            entity_bufs.push(b);
        }
        let entity_refs: Vec<&[u8]> = entity_bufs.iter().map(|v| v.as_slice()).collect();
        tl_gen::build_messages_sendMessage(
            false, false, false, false, false, false, false, false,
            &peer, None, message, random_id, None, Some(&entity_refs), None, None, None, None, None, None, None, None,
        )
    }
}

// messages.sendMedia with uploaded photo + caption + entities, targeted at a channel
pub fn build_send_media_uploaded_photo(
    channel_id: i64,
    access_hash: i64,
    file_id: i64,
    parts: i32,
    file_name: &str,
    caption: &str,
    entities: &[MarkdownEntity],
    random_id: i64,
) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
    let file = tl_gen::serialize_inputFile(file_id, parts, file_name, "");
    let media = tl_gen::serialize_inputMediaUploadedPhoto(false, false, &file, None, None, None);

    if entities.is_empty() {
        tl_gen::build_messages_sendMedia(
            false, false, false, false, false, false, false,
            &peer, None, &media, caption, random_id, None, None, None, None, None, None, None, None, None,
        )
    } else {
        let mut entity_bufs: Vec<Vec<u8>> = Vec::with_capacity(entities.len());
        for e in entities {
            let mut b = Vec::new();
            e.write_to(&mut b);
            entity_bufs.push(b);
        }
        let entity_refs: Vec<&[u8]> = entity_bufs.iter().map(|v| v.as_slice()).collect();
        tl_gen::build_messages_sendMedia(
            false, false, false, false, false, false, false,
            &peer, None, &media, caption, random_id, None, Some(&entity_refs), None, None, None, None, None, None, None,
        )
    }
}

// parse our internal markdown V2 ("**bold**", "__italic__", "++u++", "~~s~~",
// "||spoiler||", "`code`", "[text](url)") into plain text + entities.
// offsets are utf-16 code units (telegram standard).
pub fn parse_markdown_v2(input: &str) -> (String, Vec<MarkdownEntity>) {
    let mut output = String::new();
    let mut entities = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;
    // utf16 offset tracker
    let mut utf16_offset: i32 = 0;
    let utf16_len = |s: &str| -> i32 { s.encode_utf16().count() as i32 };

    while i < chars.len() {
        // link [text](url)
        if chars[i] == '[' {
            if let Some(close) = find_unescaped(&chars, i + 1, ']') {
                if close + 1 < chars.len() && chars[close + 1] == '(' {
                    if let Some(url_end) = find_unescaped(&chars, close + 2, ')') {
                        let text: String = chars[i + 1..close].iter().collect();
                        let url: String = chars[close + 2..url_end].iter().collect();
                        let start_off = utf16_offset;
                        // recurse to allow nested formatting inside link text
                        let (inner_text, inner_entities) = parse_markdown_v2(&text);
                        let inner_len = utf16_len(&inner_text);
                        for ent in inner_entities {
                            entities.push(shift_entity(ent, start_off));
                        }
                        entities.push(MarkdownEntity::TextUrl {
                            offset: start_off,
                            length: inner_len,
                            url,
                        });
                        output.push_str(&inner_text);
                        utf16_offset += inner_len;
                        i = url_end + 1;
                        continue;
                    }
                }
            }
        }

        // two-char markers
        if i + 1 < chars.len() {
            let two: String = chars[i..i + 2].iter().collect();
            if let Some((mark, ctor)) = match two.as_str() {
                "**" => Some(("**", "bold")),
                "__" => Some(("__", "italic")),
                "++" => Some(("++", "underline")),
                "~~" => Some(("~~", "strike")),
                "||" => Some(("||", "spoiler")),
                _ => None,
            } {
                if let Some(end) = find_marker(&chars, i + 2, mark) {
                    let inner: String = chars[i + 2..end].iter().collect();
                    let start_off = utf16_offset;
                    let (inner_text, inner_entities) = parse_markdown_v2(&inner);
                    let inner_len = utf16_len(&inner_text);
                    for ent in inner_entities {
                        entities.push(shift_entity(ent, start_off));
                    }
                    let ent = match ctor {
                        "bold" => MarkdownEntity::Bold { offset: start_off, length: inner_len },
                        "italic" => MarkdownEntity::Italic { offset: start_off, length: inner_len },
                        "underline" => MarkdownEntity::Underline { offset: start_off, length: inner_len },
                        "strike" => MarkdownEntity::Strike { offset: start_off, length: inner_len },
                        _ => MarkdownEntity::Spoiler { offset: start_off, length: inner_len },
                    };
                    entities.push(ent);
                    output.push_str(&inner_text);
                    utf16_offset += inner_len;
                    i = end + 2;
                    continue;
                }
            }
        }

        // one-char marker: code
        if chars[i] == '`' {
            if let Some(end) = find_char(&chars, i + 1, '`') {
                let inner: String = chars[i + 1..end].iter().collect();
                let inner_len = utf16_len(&inner);
                entities.push(MarkdownEntity::Code {
                    offset: utf16_offset,
                    length: inner_len,
                });
                output.push_str(&inner);
                utf16_offset += inner_len;
                i = end + 1;
                continue;
            }
        }

        // backslash escape: \\* -> *
        if chars[i] == '\\' && i + 1 < chars.len() {
            let c = chars[i + 1];
            output.push(c);
            utf16_offset += utf16_len(&c.to_string());
            i += 2;
            continue;
        }

        // plain char
        output.push(chars[i]);
        utf16_offset += utf16_len(&chars[i].to_string());
        i += 1;
    }

    (output, entities)
}

fn shift_entity(e: MarkdownEntity, by: i32) -> MarkdownEntity {
    match e {
        MarkdownEntity::Bold { offset, length } => MarkdownEntity::Bold { offset: offset + by, length },
        MarkdownEntity::Italic { offset, length } => MarkdownEntity::Italic { offset: offset + by, length },
        MarkdownEntity::Underline { offset, length } => MarkdownEntity::Underline { offset: offset + by, length },
        MarkdownEntity::Strike { offset, length } => MarkdownEntity::Strike { offset: offset + by, length },
        MarkdownEntity::Spoiler { offset, length } => MarkdownEntity::Spoiler { offset: offset + by, length },
        MarkdownEntity::Code { offset, length } => MarkdownEntity::Code { offset: offset + by, length },
        MarkdownEntity::TextUrl { offset, length, url } => MarkdownEntity::TextUrl { offset: offset + by, length, url },
    }
}

fn find_marker(chars: &[char], from: usize, mark: &str) -> Option<usize> {
    let m: Vec<char> = mark.chars().collect();
    let mut i = from;
    while i + m.len() <= chars.len() {
        if chars[i..i + m.len()] == m[..] {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_char(chars: &[char], from: usize, c: char) -> Option<usize> {
    chars[from..].iter().position(|x| *x == c).map(|p| p + from)
}

fn find_unescaped(chars: &[char], from: usize, c: char) -> Option<usize> {
    let mut i = from;
    while i < chars.len() {
        if chars[i] == '\\' { i += 2; continue; }
        if chars[i] == c { return Some(i); }
        i += 1;
    }
    None
}

// === boost / engagement TL builders ===

// channels.joinChannel
pub fn build_join_channel(channel_id: i64, access_hash: i64) -> Vec<u8> {
    let channel = tl_gen::serialize_input_channel(channel_id, access_hash);
    tl_gen::build_channels_joinChannel(&channel)
}

// messages.importChatInvite
pub fn build_import_chat_invite(hash: &str) -> Vec<u8> {
    tl_gen::build_messages_importChatInvite(hash)
}

// messages.checkChatInvite
pub fn build_check_chat_invite(hash: &str) -> Vec<u8> {
    tl_gen::build_messages_checkChatInvite(hash)
}

// messages.getMessagesViews for channel
pub fn build_get_messages_views_channel(channel_id: i64, access_hash: i64, msg_ids: &[i32], increment: bool) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
    tl_gen::build_messages_getMessagesViews(&peer, msg_ids, increment)
}

// messages.sendReaction for channel
pub fn build_send_reaction_channel(channel_id: i64, access_hash: i64, msg_id: i32, emoji: Option<&str>, big: bool) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
    match emoji {
        Some(e) => {
            let reaction_bytes = tl_gen::serialize_reactionEmoji(e);
            let reaction_refs: &[&[u8]] = &[&reaction_bytes];
            tl_gen::build_messages_sendReaction(big, true, &peer, msg_id, Some(reaction_refs))
        }
        None => {
            tl_gen::build_messages_sendReaction(big, true, &peer, msg_id, None)
        }
    }
}

// folders.editPeerFolders — move channel to folder
pub fn build_edit_peer_folder_channel(channel_id: i64, access_hash: i64, folder_id: i32) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
    let folder_peer = tl_gen::serialize_inputFolderPeer(&peer, folder_id);
    let refs: &[&[u8]] = &[&folder_peer];
    tl_gen::build_folders_editPeerFolders(refs)
}

// chatlists.checkChatlistInvite
pub fn build_check_chatlist_invite(slug: &str) -> Vec<u8> {
    tl_gen::build_chatlists_checkChatlistInvite(slug)
}

// chatlists.joinChatlistInvite — peers_blob is count(u32) + serialized InputPeer items
pub fn build_join_chatlist_invite(slug: &str, peers_blob: &[u8]) -> Vec<u8> {
    // peers_blob format: count(u32) + N * InputPeer bytes
    // we need to split it into individual peer slices for tl_gen
    if peers_blob.len() < 4 {
        return tl_gen::build_chatlists_joinChatlistInvite(slug, &[]);
    }
    let count = u32::from_le_bytes([peers_blob[0], peers_blob[1], peers_blob[2], peers_blob[3]]) as usize;
    // each inputPeerChannel is 4 (ctor) + 8 (id) + 8 (hash) = 20 bytes
    let item_size = 20;
    let mut refs: Vec<&[u8]> = Vec::with_capacity(count);
    let mut offset = 4;
    for _ in 0..count {
        if offset + item_size > peers_blob.len() { break; }
        refs.push(&peers_blob[offset..offset + item_size]);
        offset += item_size;
    }
    tl_gen::build_chatlists_joinChatlistInvite(slug, &refs)
}

// parse chatlists.chatlistInvite to extract peers as serialized InputPeer vector.
pub fn parse_chatlist_invite_as_input_peers(data: &[u8]) -> Result<(String, Vec<u8>), String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlChatlistsChatlistInvite>(&inner)?;

    let (title, chats_raw) = match obj {
        tl_gen::TlChatlistsChatlistInvite::ChatlistInvite { chats, .. } => {
            (String::new(), chats)
        }
        tl_gen::TlChatlistsChatlistInvite::Already { chats, .. } => {
            (String::new(), chats)
        }
    };

    // extract (channel_id, access_hash) from chats vector
    let mut found: Vec<(i64, i64)> = Vec::new();
    for raw in &chats_raw {
        if let Ok(chat) = tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(raw) {
            if let tl_gen::TlChat::Channel { id, access_hash, .. } = chat {
                let ah = access_hash.unwrap_or(0);
                if !found.iter().any(|(fid, _)| *fid == id) {
                    found.push((id, ah));
                }
            }
        }
    }

    // serialize as Vector<InputPeer>: count(u32) + items
    let mut out = Vec::new();
    out.write_u32::<LittleEndian>(found.len() as u32).unwrap();
    for (id, hash) in &found {
        let peer = tl_gen::serialize_input_peer_channel(*id, *hash);
        out.extend_from_slice(&peer);
    }
    Ok((title, out))
}

// messages.getHistory for a channel peer
pub fn build_get_history_channel(channel_id: i64, access_hash: i64, limit: i32) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
    tl_gen::build_messages_getHistory(&peer, 0, 0, 0, limit, 0, 0, 0)
}

// messages.search with InputMessagesFilterPinned — returns only pinned messages in a channel
pub fn build_search_channel_pinned(channel_id: i64, access_hash: i64, limit: i32) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
    let filter = tl_gen::INPUT_MESSAGES_FILTER_PINNED.to_le_bytes().to_vec();
    tl_gen::build_messages_search(&peer, "", None, None, None, None, &filter, 0, 0, 0, 0, limit, 0, 0, 0)
}

// === cloner TL builders ===

// messages.getHistory for a channel peer with full pagination control.
pub fn build_get_history_channel_paged(
    channel_id: i64,
    access_hash: i64,
    offset_id: i32,
    add_offset: i32,
    limit: i32,
    max_id: i32,
    min_id: i32,
) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
    tl_gen::build_messages_getHistory(&peer, offset_id, 0, add_offset, limit, max_id, min_id, 0)
}

// messages.forwardMessages with drop_author + drop_media_captions support.
pub fn build_forward_messages(
    from_channel_id: i64,
    from_access_hash: i64,
    to_channel_id: i64,
    to_access_hash: i64,
    msg_ids: &[i32],
    drop_author: bool,
    drop_captions: bool,
    silent: bool,
) -> Vec<u8> {
    let from_peer = tl_gen::serialize_input_peer_channel(from_channel_id, from_access_hash);
    let to_peer = tl_gen::serialize_input_peer_channel(to_channel_id, to_access_hash);
    let random_ids: Vec<i64> = msg_ids.iter().map(|_| rand::random()).collect();
    tl_gen::build_messages_forwardMessages(
        silent, false, false, drop_author, drop_captions, false, false,
        &from_peer, msg_ids, &random_ids, &to_peer, None, None, None, None, None, None, None, None, None, None,
    )
}

// messages.editMessage — text + entities + no_webpage for channel messages
pub fn build_edit_message_channel(
    channel_id: i64,
    access_hash: i64,
    msg_id: i32,
    new_text: &str,
    entities: &[MarkdownEntity],
    no_webpage: bool,
) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_channel(channel_id, access_hash);
    if entities.is_empty() {
        tl_gen::build_messages_editMessage(no_webpage, false, &peer, msg_id, Some(new_text), None, None, None, None, None, None, None)
    } else {
        let mut entity_bufs: Vec<Vec<u8>> = Vec::with_capacity(entities.len());
        for e in entities {
            let mut b = Vec::new();
            e.write_to(&mut b);
            entity_bufs.push(b);
        }
        let entity_refs: Vec<&[u8]> = entity_bufs.iter().map(|v| v.as_slice()).collect();
        tl_gen::build_messages_editMessage(no_webpage, false, &peer, msg_id, Some(new_text), None, None, Some(&entity_refs), None, None, None, None)
    }
}

// channels.getFullChannel
pub fn build_get_full_channel(channel_id: i64, access_hash: i64) -> Vec<u8> {
    let channel = tl_gen::serialize_input_channel(channel_id, access_hash);
    tl_gen::build_channels_getFullChannel(&channel)
}

// info extracted from messages.chatFull → channelFull#e4e0b29d.
//
// the source channel photo is exposed as `chat_photo_id` + `chat_photo_access_hash`
// + `chat_photo_file_reference` so the cloner can reuse it via inputPhoto without
// re-uploading.
#[derive(Debug, Default, Clone)]
pub struct FullChannelInfo {
    pub title: String,
    pub about: String,
    pub chat_photo_id: i64,
    pub chat_photo_access_hash: i64,
    pub chat_photo_file_reference: Vec<u8>,
}

// fully decodes messages.chatFull response for a channel and returns title/about/photo.
pub fn parse_full_channel(data: &[u8]) -> Result<FullChannelInfo, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlMessagesChatFull>(&inner)?;

    let mut info = FullChannelInfo::default();

    // extract title from chats vector
    for raw in &obj.chats {
        if let Ok(chat) = tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(raw) {
            if let tl_gen::TlChat::Channel { title, .. } = chat {
                info.title = title;
                break;
            }
        }
    }

    // extract about and photo from full_chat (channelFull)
    if let Ok(full) = tl_gen::deserialize_tl_obj::<tl_gen::TlChatFull>(&obj.full_chat) {
        match full {
            tl_gen::TlChatFull::ChannelFull { about, chat_photo, .. } => {
                info.about = about;
                // decode photo to extract id/access_hash/file_reference
                if let Ok(photo) = tl_gen::deserialize_tl_obj::<tl_gen::TlPhoto>(&chat_photo) {
                    if let tl_gen::TlPhoto::Photo { id, access_hash, file_reference, .. } = photo {
                        info.chat_photo_id = id;
                        info.chat_photo_access_hash = access_hash;
                        info.chat_photo_file_reference = file_reference;
                    }
                }
            }
            tl_gen::TlChatFull::Full { about, chat_photo, .. } => {
                info.about = about;
                if let Some(photo_raw) = chat_photo {
                    if let Ok(photo) = tl_gen::deserialize_tl_obj::<tl_gen::TlPhoto>(&photo_raw) {
                        if let tl_gen::TlPhoto::Photo { id, access_hash, file_reference, .. } = photo {
                            info.chat_photo_id = id;
                            info.chat_photo_access_hash = access_hash;
                            info.chat_photo_file_reference = file_reference;
                        }
                    }
                }
            }
            tl_gen::TlChatFull::CommunityFull { .. } => {}
        }
    }

    Ok(info)
}

// scans Updates response from forwardMessages / sendMessage for the freshly
// created message and returns its ID. returns None if nothing matches.
pub fn extract_first_new_message_id(data: &[u8]) -> Option<i32> {
    if data.len() < 4 { return None; }

    let ctor = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);

    // gzip wrapper
    if ctor == GZIP_PACKED {
        let mut cursor = Cursor::new(data);
        let _ = cursor.read_u32::<LittleEndian>();
        if let Ok(compressed) = deserialize_bytes(&mut cursor) {
            if let Ok(d) = decompress_gzip(&compressed) {
                return extract_first_new_message_id(&d);
            }
        }
        return None;
    }

    let inner = tl_gen::unwrap_rpc(data).unwrap_or_else(|_| data.to_vec());
    let updates = tl_gen::deserialize_tl_obj::<tl_gen::TlUpdates>(&inner).ok()?;

    match updates {
        // updateShortSentMessage carries the id directly
        tl_gen::TlUpdates::HortSentMessage { id, .. } => if id > 0 { Some(id) } else { None },
        tl_gen::TlUpdates::Hort { update, .. } => message_id_from_update(&update),
        tl_gen::TlUpdates::Updates { updates, .. } | tl_gen::TlUpdates::Combined { updates, .. } => {
            let mut best: Option<i32> = None;
            for raw in &updates {
                if let Some(id) = message_id_from_update(raw) {
                    if id > 0 && best.map_or(true, |b| id < b) { best = Some(id); }
                }
            }
            best
        }
        _ => None,
    }
}

// extract a new message id from a single raw Update object.
fn message_id_from_update(raw: &[u8]) -> Option<i32> {
    if raw.len() < 4 { return None; }
    let ctor = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
    let mut cursor = Cursor::new(raw);
    let _ = cursor.read_u32::<LittleEndian>();

    match ctor {
        // updateMessageID#4e90bfd6 id:int random_id:long
        tl_gen::UPDATE_MESSAGE_ID => cursor.read_i32::<LittleEndian>().ok().filter(|&id| id > 0),
        // updateNewMessage / updateNewChannelMessage carry a Message
        tl_gen::UPDATE_NEW_MESSAGE | tl_gen::UPDATE_NEW_CHANNEL_MESSAGE => {
            match tl_gen::TlMessage::deserialize(&mut cursor).ok()? {
                tl_gen::TlMessage::Message { id, .. }
                | tl_gen::TlMessage::Service { id, .. }
                | tl_gen::TlMessage::Empty { id, .. } => if id > 0 { Some(id) } else { None },
            }
        }
        _ => None,
    }
}


// === full message media decoder (used by cloner for size gating) ===

// channels.editPhoto with a reused inputPhoto reference (no re-upload required).
pub fn build_channel_edit_photo_existing(
    target_channel_id: i64,
    target_access_hash: i64,
    photo_id: i64,
    photo_access_hash: i64,
    file_reference: &[u8],
) -> Vec<u8> {
    let channel = tl_gen::serialize_input_channel(target_channel_id, target_access_hash);
    let input_photo = tl_gen::serialize_inputPhoto(photo_id, photo_access_hash, file_reference);
    let chat_photo = tl_gen::serialize_inputChatPhoto(&input_photo);
    tl_gen::build_channels_editPhoto(&channel, &chat_photo)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MediaSummary {
    pub kind: MediaKindRepr,
    pub size_bytes: u64, // 0 = unknown
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum MediaKindRepr {
    #[default]
    None,
    Photo,
    Video,
    Document,
    Audio,
    Other,
}

fn media_to_summary(media: &tl_gen::TlMessageMedia) -> Result<MediaSummary, String> {
    match media {
        tl_gen::TlMessageMedia::Empty => Ok(MediaSummary::default()),
        tl_gen::TlMessageMedia::Photo { photo, .. } => {
            let size = match photo {
                Some(raw) => photo_size_from_raw(raw),
                None => 0,
            };
            Ok(MediaSummary { kind: MediaKindRepr::Photo, size_bytes: size })
        }
        tl_gen::TlMessageMedia::Document { document, .. } => {
            match document {
                Some(raw) => document_summary_from_raw(raw),
                None => Ok(MediaSummary { kind: MediaKindRepr::Document, size_bytes: 0 }),
            }
        }
        _ => Ok(MediaSummary { kind: MediaKindRepr::Other, size_bytes: 0 }),
    }
}

// extract the largest photo size from a raw TlPhoto blob
fn photo_size_from_raw(raw: &[u8]) -> u64 {
    let photo = match tl_gen::deserialize_tl_obj::<tl_gen::TlPhoto>(raw) {
        Ok(p) => p,
        Err(_) => return 0,
    };
    match photo {
        tl_gen::TlPhoto::Empty { .. } => 0,
        tl_gen::TlPhoto::Photo { sizes, .. } => {
            let mut max: u64 = 0;
            for size_raw in &sizes {
                let sz = photo_size_bytes(size_raw);
                if sz > max { max = sz; }
            }
            max
        }
    }
}

// extract byte size from a single TlPhotoSize blob
fn photo_size_bytes(raw: &[u8]) -> u64 {
    let ps = match tl_gen::deserialize_tl_obj::<tl_gen::TlPhotoSize>(raw) {
        Ok(p) => p,
        Err(_) => return 0,
    };
    match ps {
        tl_gen::TlPhotoSize::Empty { .. } => 0,
        tl_gen::TlPhotoSize::PhotoSize { size, .. } => size.max(0) as u64,
        tl_gen::TlPhotoSize::PhotoCachedSize { bytes, .. } => bytes.len() as u64,
        tl_gen::TlPhotoSize::PhotoStrippedSize { bytes, .. } => bytes.len() as u64,
        tl_gen::TlPhotoSize::Progressive { sizes, .. } => {
            sizes.iter().filter(|&&s| s > 0).last().copied().unwrap_or(0) as u64
        }
        tl_gen::TlPhotoSize::PhotoPathSize { bytes, .. } => bytes.len() as u64,
    }
}

// extract kind + size from a raw TlDocument blob
fn document_summary_from_raw(raw: &[u8]) -> Result<MediaSummary, String> {
    let doc = tl_gen::deserialize_tl_obj::<tl_gen::TlDocument>(raw)
        .map_err(|e| format!("doc deser: {e}"))?;
    match doc {
        tl_gen::TlDocument::Empty { .. } => Ok(MediaSummary { kind: MediaKindRepr::Document, size_bytes: 0 }),
        tl_gen::TlDocument::Document { size, attributes, .. } => {
            let mut kind = MediaKindRepr::Document;
            for attr_raw in &attributes {
                if let Ok(attr) = tl_gen::deserialize_tl_obj::<tl_gen::TlDocumentAttribute>(attr_raw) {
                    kind = merge_doc_attr_kind(&attr, kind);
                }
            }
            Ok(MediaSummary { kind, size_bytes: size.max(0) as u64 })
        }
    }
}

// determine media kind from a document attribute, with precedence: Video > Audio > Document
fn merge_doc_attr_kind(attr: &tl_gen::TlDocumentAttribute, current: MediaKindRepr) -> MediaKindRepr {
    match attr {
        tl_gen::TlDocumentAttribute::Video { .. } => MediaKindRepr::Video,
        tl_gen::TlDocumentAttribute::Audio { .. } => {
            if current == MediaKindRepr::Document || current == MediaKindRepr::None {
                MediaKindRepr::Audio
            } else {
                current
            }
        }
        _ => {
            if current == MediaKindRepr::None { MediaKindRepr::Document } else { current }
        }
    }
}

// decode media from a full message blob using tl_gen deserialization
pub fn extract_message_media_summary(message_blob: &[u8]) -> Option<MediaSummary> {
    // try deserializing as a TlMessage directly
    if let Ok(msg) = tl_gen::deserialize_tl_obj::<tl_gen::TlMessage>(message_blob) {
        if let tl_gen::TlMessage::Message { media: Some(ref media_raw), .. } = msg {
            if let Ok(media) = tl_gen::deserialize_tl_obj::<tl_gen::TlMessageMedia>(media_raw) {
                if let Ok(s) = media_to_summary(&media) {
                    return Some(s);
                }
            }
        }
        return None;
    }

    // fallback: scan for messageMedia ctors at 4-byte boundaries
    let mut p = 0usize;
    while p + 4 <= message_blob.len() {
        let slice = &message_blob[p..];
        if let Ok(media) = tl_gen::deserialize_tl_obj::<tl_gen::TlMessageMedia>(slice) {
            if let Ok(s) = media_to_summary(&media) {
                if s.kind != MediaKindRepr::None {
                    return Some(s);
                }
            }
        }
        p += 4;
    }
    None
}


// channels.deleteMessages#84c1fd4e channel:InputChannel id:Vector<int> = messages.AffectedMessages
pub fn build_channels_delete_messages(channel_id: i64, access_hash: i64, msg_ids: &[i32]) -> Vec<u8> {
    let channel = tl_gen::serialize_input_channel(channel_id, access_hash);
    tl_gen::build_channels_deleteMessages(&channel, msg_ids)
}


// scans bytes for a Channel object whose id matches `channel_id`
// and returns whether it is a broadcast channel. returns `None` if not found.
pub fn scan_channel_is_broadcast_opt(data: &[u8], channel_id: i64) -> Option<bool> {
    // scan for TlChat::Channel at 4-byte boundaries
    let mut i = 0usize;
    while i + 4 <= data.len() {
        let c = u32::from_le_bytes([data[i], data[i+1], data[i+2], data[i+3]]);
        if c == tl_gen::CHANNEL {
            let mut cursor = Cursor::new(&data[i..]);
            if let Ok(chat) = tl_gen::TlChat::deserialize(&mut cursor) {
                if let tl_gen::TlChat::Channel { id, broadcast, megagroup, .. } = chat {
                    if id == channel_id {
                        return Some(broadcast && !megagroup);
                    }
                }
            }
        }
        i += 4;
    }
    None
}

// convenience wrapper that defaults to `false` (treat as group) when the
// channel object isn't present in the response.
pub fn scan_channel_is_broadcast(data: &[u8], channel_id: i64) -> bool {
    scan_channel_is_broadcast_opt(data, channel_id).unwrap_or(false)
}

// scans bytes for the first Channel object that carries a usable access_hash.
pub fn parse_first_accessible_channel(data: &[u8]) -> Result<(i64, i64), String> {
    let inner = tl_gen::unwrap_rpc(data)?;

    let mut i = 0usize;
    while i + 4 <= inner.len() {
        let c = u32::from_le_bytes([inner[i], inner[i+1], inner[i+2], inner[i+3]]);
        if c == tl_gen::CHANNEL {
            let mut cursor = Cursor::new(&inner[i..]);
            if let Ok(chat) = tl_gen::TlChat::deserialize(&mut cursor) {
                if let tl_gen::TlChat::Channel { id, access_hash, .. } = chat {
                    let ah = access_hash.unwrap_or(0);
                    if id != 0 && ah != 0 {
                        return Ok((id, ah));
                    }
                }
            }
        }
        i += 4;
    }
    Err("no accessible channel found in response".into())
}

#[derive(Debug, Clone, Default)]
pub struct ChatInviteSummary {
    pub is_chat_invite: bool,
    pub broadcast: bool,
    pub megagroup: bool,
    pub request_needed: bool,
    pub title: String,
    pub channel_id: Option<i64>,
    pub access_hash: Option<i64>,
}

// fully decode a messages.checkChatInvite response via tl_gen.
pub fn parse_chat_invite_summary(data: &[u8]) -> Result<ChatInviteSummary, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlChatInvite>(&inner)?;

    match obj {
        tl_gen::TlChatInvite::ChatInvite { broadcast, megagroup, request_needed, title, .. } => {
            Ok(ChatInviteSummary {
                is_chat_invite: true,
                broadcast,
                megagroup,
                request_needed,
                title,
                channel_id: None,
                access_hash: None,
            })
        }
        tl_gen::TlChatInvite::Already { chat, .. } => {
            let (channel_id, access_hash) = if let Ok(c) = tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(&chat) {
                match c {
                    tl_gen::TlChat::Channel { id, access_hash, .. } => (Some(id), Some(access_hash.unwrap_or(0))),
                    _ => (None, None),
                }
            } else { (None, None) };

            Ok(ChatInviteSummary {
                is_chat_invite: false,
                broadcast: false,
                megagroup: false,
                request_needed: false,
                title: String::new(),
                channel_id,
                access_hash,
            })
        }
        tl_gen::TlChatInvite::Peek { chat, .. } => {
            let (channel_id, access_hash) = if let Ok(c) = tl_gen::deserialize_tl_obj::<tl_gen::TlChat>(&chat) {
                match c {
                    tl_gen::TlChat::Channel { id, access_hash, .. } => (Some(id), Some(access_hash.unwrap_or(0))),
                    _ => (None, None),
                }
            } else { (None, None) };

            Ok(ChatInviteSummary {
                is_chat_invite: false,
                broadcast: false,
                megagroup: false,
                request_needed: false,
                title: String::new(),
                channel_id,
                access_hash,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnlineBucket {
    // userStatusOnline / userStatusRecently
    Recent,
    // userStatusLastWeek
    Week,
    // userStatusLastMonth
    Month,
    // userStatusOffline (>1 month ago) / userStatusEmpty
    Long,
    // user.deleted flag
    Deleted,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct ParticipantUser {
    pub id: i64,
    pub access_hash: i64,
    pub first_name: String,
    pub last_name: String,
    pub username: String,
    pub phone: String,
    pub is_bot: bool,
    pub is_deleted: bool,
    pub is_admin: bool,
    pub is_self: bool,
    pub premium: bool,
    pub bucket: OnlineBucket,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct ParticipantsBatch {
    pub total_count: u32,
    pub users: Vec<ParticipantUser>,
    pub participants_count: u32,
    pub parse_skipped: u32,
}

// ChannelParticipantsFilter selector for build_get_participants
#[derive(Debug, Clone, Copy)]
pub enum ParticipantsFilter {
    #[allow(dead_code)]
    Recent,
    Admins,
}

// channels.getParticipants — pulls a paginated batch of channel/megagroup members.
// returns the raw Updates-style response that should be fed into parse_channel_participants.
pub fn build_channels_get_participants(
    channel_id: i64,
    access_hash: i64,
    filter: ParticipantsFilter,
    offset: i32,
    limit: i32,
) -> Vec<u8> {
    let channel = tl_gen::serialize_input_channel(channel_id, access_hash);
    let filter_bytes = match filter {
        ParticipantsFilter::Recent => tl_gen::CHANNEL_PARTICIPANTS_RECENT.to_le_bytes().to_vec(),
        ParticipantsFilter::Admins => tl_gen::CHANNEL_PARTICIPANTS_ADMINS.to_le_bytes().to_vec(),
    };
    tl_gen::build_channels_getParticipants(&channel, &filter_bytes, offset, limit, 0)
}

// search filter takes an extra string
pub fn build_channels_get_participants_search(
    channel_id: i64,
    access_hash: i64,
    query: &str,
    offset: i32,
    limit: i32,
) -> Vec<u8> {
    let channel = tl_gen::serialize_input_channel(channel_id, access_hash);
    let filter_bytes = tl_gen::serialize_channelParticipantsSearch(query);
    tl_gen::build_channels_getParticipants(&channel, &filter_bytes, offset, limit, 0)
}

// fully parses a channels.getParticipants response via tl_gen deserialization.
pub fn parse_channel_participants(data: &[u8]) -> Result<ParticipantsBatch, String> {
    let inner = tl_gen::unwrap_rpc(data)?;
    let obj = tl_gen::deserialize_tl_obj::<tl_gen::TlChannelsChannelParticipants>(&inner)?;

    match obj {
        tl_gen::TlChannelsChannelParticipants::NotModified => Ok(ParticipantsBatch::default()),
        tl_gen::TlChannelsChannelParticipants::ChannelParticipants { count, participants, users, .. } => {
            // collect admin/self markers from participants vector
            let mut admin_ids: Vec<i64> = Vec::new();
            let mut self_ids: Vec<i64> = Vec::new();
            for raw in &participants {
                if let Ok(p) = tl_gen::deserialize_tl_obj::<tl_gen::TlChannelParticipant>(raw) {
                    match p {
                        tl_gen::TlChannelParticipant::Creator { user_id, .. } => { admin_ids.push(user_id); }
                        tl_gen::TlChannelParticipant::Admin { user_id, .. } => { admin_ids.push(user_id); }
                        tl_gen::TlChannelParticipant::Myself { user_id, .. } => { self_ids.push(user_id); }
                        _ => {}
                    }
                }
            }

            // parse users vector
            let mut parsed_users: Vec<ParticipantUser> = Vec::with_capacity(users.len());
            let mut parse_skipped: u32 = 0;
            for raw in &users {
                match tl_gen::deserialize_tl_obj::<tl_gen::TlUser>(raw) {
                    Ok(user) => {
                        parsed_users.push(tl_user_to_participant(user));
                    }
                    Err(_) => { parse_skipped += 1; }
                }
            }

            // merge admin/self markers
            for u in &mut parsed_users {
                if admin_ids.contains(&u.id) { u.is_admin = true; }
                if self_ids.contains(&u.id) { u.is_self = true; }
            }

            Ok(ParticipantsBatch {
                total_count: count as u32,
                users: parsed_users,
                participants_count: participants.len() as u32,
                parse_skipped,
            })
        }
    }
}

fn tl_user_to_participant(user: tl_gen::TlUser) -> ParticipantUser {
    match user {
        tl_gen::TlUser::Empty { id, .. } => ParticipantUser {
            id, access_hash: 0, first_name: String::new(), last_name: String::new(),
            username: String::new(), phone: String::new(), is_bot: false,
            is_deleted: true, is_admin: false, is_self: false, premium: false,
            bucket: OnlineBucket::Deleted,
        },
        tl_gen::TlUser::User {
            id, access_hash, first_name, last_name, username, phone,
            bot, deleted, self_, premium, status, usernames, ..
        } => {
            let access_hash = access_hash.unwrap_or(0);
            let first_name = first_name.unwrap_or_default();
            let last_name = last_name.unwrap_or_default();
            let phone = phone.unwrap_or_default();
            let mut final_username = username.unwrap_or_default();

            // recover username from usernames vector if primary is empty
            if final_username.is_empty() {
                if let Some(raw_vec) = usernames {
                    for raw in &raw_vec {
                        if let Ok(uname) = tl_gen::deserialize_tl_obj::<tl_gen::TlUsername>(raw) {
                            if uname.active && !uname.username.is_empty() {
                                final_username = uname.username;
                                break;
                            }
                        }
                    }
                }
            }

            let bucket = if deleted {
                OnlineBucket::Deleted
            } else if let Some(status_raw) = status {
                classify_user_status_raw(&status_raw)
            } else {
                OnlineBucket::Unknown
            };

            ParticipantUser {
                id, access_hash, first_name, last_name,
                username: final_username, phone,
                is_bot: bot, is_deleted: deleted,
                is_admin: false, is_self: self_, premium,
                bucket,
            }
        }
    }
}

fn classify_user_status_raw(raw: &[u8]) -> OnlineBucket {
    if raw.len() < 4 { return OnlineBucket::Unknown; }
    let ctor = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
    match ctor {
        tl_gen::USER_STATUS_EMPTY => OnlineBucket::Unknown,
        tl_gen::USER_STATUS_ONLINE => OnlineBucket::Recent,
        tl_gen::USER_STATUS_OFFLINE => {
            if raw.len() >= 8 {
                let was_online = i32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]);
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i32)
                    .unwrap_or(0);
                let delta = now.saturating_sub(was_online);
                match delta {
                    d if d < 60 * 60 * 24 * 3 => OnlineBucket::Recent,
                    d if d < 60 * 60 * 24 * 7 => OnlineBucket::Week,
                    d if d < 60 * 60 * 24 * 30 => OnlineBucket::Month,
                    _ => OnlineBucket::Long,
                }
            } else {
                OnlineBucket::Unknown
            }
        }
        tl_gen::USER_STATUS_RECENTLY => OnlineBucket::Recent,
        tl_gen::USER_STATUS_LAST_WEEK => OnlineBucket::Week,
        tl_gen::USER_STATUS_LAST_MONTH => OnlineBucket::Month,
        _ => OnlineBucket::Unknown,
    }
}

// messages.sendMessage#fe05dc9a to saved messages (inputPeerSelf)
pub fn build_send_saved_message(message: &str) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_self();
    let random_id: i64 = rand::random();
    tl_gen::build_messages_sendMessage(
        true, false, false, false, false, false, false, false,
        &peer, None, message, random_id, None, None, None, None, None, None, None, None, None, None,
    )
}

// messages.searchGlobal#4bc6589a flags:# folder_id:flags.0?int q:string filter:MessagesFilter
//   min_date:int max_date:int offset_rate:int offset_peer:InputPeer offset_id:int limit:int
pub fn build_search_global(query: &str) -> Vec<u8> {
    let filter = tl_gen::INPUT_MESSAGES_FILTER_EMPTY.to_le_bytes().to_vec();
    let empty_peer = tl_gen::INPUT_PEER_EMPTY.to_le_bytes().to_vec();
    tl_gen::build_messages_searchGlobal(
        false, false, false, None, None, query, &filter,
        0, 0, 0, &empty_peer, 0, 20,
    )
}

// messages.getDialogs (simple wrapper, main folder, limit 20)
pub fn build_get_dialogs() -> Vec<u8> {
    build_get_dialogs_with_folder(0, 20)
}

// messages.sendReaction
pub fn build_send_reaction(peer_ctor: u32, peer_id: i64, peer_hash: i64, msg_id: i32, emoticon: &str) -> Vec<u8> {
    // build peer bytes manually (same as before — ctor + id + optional hash)
    let mut peer = Vec::new();
    peer.write_u32::<LittleEndian>(peer_ctor).unwrap();
    peer.write_i64::<LittleEndian>(peer_id).unwrap();
    if peer_ctor != INPUT_PEER_SELF { peer.write_i64::<LittleEndian>(peer_hash).unwrap(); }

    let reaction_bytes = tl_gen::serialize_reactionEmoji(emoticon);
    let reaction_refs: &[&[u8]] = &[&reaction_bytes];
    tl_gen::build_messages_sendReaction(false, false, &peer, msg_id, Some(reaction_refs))
}

// messages.deleteMessages
pub fn build_delete_messages(msg_ids: &[i32], revoke: bool) -> Vec<u8> {
    tl_gen::build_messages_deleteMessages(revoke, msg_ids)
}

// contacts.addContact
pub fn build_add_contact(user_id: i64, access_hash: i64, first_name: &str, last_name: &str, phone: &str) -> Vec<u8> {
    let id = tl_gen::serialize_input_user(user_id, access_hash);
    tl_gen::build_contacts_addContact(false, &id, first_name, last_name, phone, None)
}

// messages.getHistory for a specific user peer
pub fn build_get_history_peer(peer_id: i64, access_hash: i64, limit: i32) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_user(peer_id, access_hash);
    tl_gen::build_messages_getHistory(&peer, 0, 0, 0, limit, 0, 0, 0)
}   

// === stories ===

// privacy rule: allow everyone
pub fn serialize_privacy_allow_all() -> Vec<u8> {
    let mut buf = Vec::with_capacity(4);
    buf.write_u32::<LittleEndian>(tl_gen::INPUT_PRIVACY_VALUE_ALLOW_ALL).unwrap();
    buf
}

// privacy rule: allow contacts only
pub fn serialize_privacy_allow_contacts() -> Vec<u8> {
    let mut buf = Vec::with_capacity(4);
    buf.write_u32::<LittleEndian>(tl_gen::INPUT_PRIVACY_VALUE_ALLOW_CONTACTS).unwrap();
    buf
}

// upload a photo story to own profile
// file_id/parts: from upload.saveFilePart calls
// caption: text overlay / description
// period: display duration in seconds (6h=21600, 12h=43200, 24h=86400, 48h=172800)
pub fn build_send_photo_story(
    file_id: i64,
    file_parts: i32,
    file_name: &str,
    caption: Option<&str>,
    period: Option<i32>,
    pinned: bool,
    privacy_rules: &[&[u8]],
    random_id: i64,
) -> Vec<u8> {
    let input_file = tl_gen::serialize_inputFile(file_id, file_parts, file_name, "");
    let media = tl_gen::serialize_inputMediaUploadedPhoto(false, false, &input_file, None, None, None);
    let peer = tl_gen::serialize_input_peer_self();
    tl_gen::build_stories_sendStory(
        pinned, false, false,
        &peer, &media, None,
        caption, None,
        privacy_rules, random_id, period,
        None, None, None, None,
    )
}

// upload a photo story using big file upload
pub fn build_send_photo_story_big(
    file_id: i64,
    file_parts: i32,
    file_name: &str,
    caption: Option<&str>,
    period: Option<i32>,
    pinned: bool,
    privacy_rules: &[&[u8]],
    random_id: i64,
) -> Vec<u8> {
    let input_file = tl_gen::serialize_inputFileBig(file_id, file_parts, file_name);
    let media = tl_gen::serialize_inputMediaUploadedPhoto(false, false, &input_file, None, None, None);
    let peer = tl_gen::serialize_input_peer_self();
    tl_gen::build_stories_sendStory(
        pinned, false, false,
        &peer, &media, None,
        caption, None,
        privacy_rules, random_id, period,
        None, None, None, None,
    )
}

// upload a video story to own profile
// attributes: documentAttributeVideo serialized bytes
pub fn build_send_video_story(
    file_id: i64,
    file_parts: i32,
    file_name: &str,
    duration: f64,
    w: i32,
    h: i32,
    caption: Option<&str>,
    period: Option<i32>,
    pinned: bool,
    privacy_rules: &[&[u8]],
    random_id: i64,
) -> Vec<u8> {
    let input_file = tl_gen::serialize_inputFileBig(file_id, file_parts, file_name);
    let video_attr = tl_gen::serialize_documentAttributeVideo(
        false, true, false,
        duration, w, h, None, None, None,
    );
    let attrs: &[&[u8]] = &[&video_attr];
    let media = tl_gen::serialize_inputMediaUploadedDocument(
        false, false, false,
        &input_file, None, "video/mp4", attrs, None, None, None, None,
    );
    let peer = tl_gen::serialize_input_peer_self();
    tl_gen::build_stories_sendStory(
        pinned, false, false,
        &peer, &media, None,
        caption, None,
        privacy_rules, random_id, period,
        None, None, None, None,
    )
}

// delete stories by ids
pub fn build_delete_stories(story_ids: &[i32]) -> Vec<u8> {
    let peer = tl_gen::serialize_input_peer_self();
    tl_gen::build_stories_deleteStories(&peer, story_ids)
}
