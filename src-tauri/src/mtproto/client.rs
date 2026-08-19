use byteorder::{LittleEndian, WriteBytesExt};
use rand::Rng;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::proxy::ProxyConfig;
use super::crypto;
use super::tl;
use super::transport::MtpTransport;

// errors that indicate the session is permanently dead — no point retrying
const FATAL_SESSION_ERRORS: &[&str] = &[
    "AUTH_KEY_UNREGISTERED",
    "AUTH_KEY_DUPLICATED",
    "SESSION_REVOKED",
    "SESSION_EXPIRED",
    "USER_DEACTIVATED",
    "USER_DEACTIVATED_BAN",
    "PHONE_NUMBER_BANNED",
    "INPUT_USER_DEACTIVATED",
    // account is frozen by telegram — methods rejected with RPC 420 FROZEN_METHOD_INVALID
    "FROZEN_METHOD_INVALID",
];

// errors that indicate a transient network/connection issue — reconnect and retry
const NETWORK_ERRORS: &[&str] = &[
    "read length failed",
    "read data failed",
    "write failed",
    "flush failed",
    "early eof",
    "connection reset",
    "broken pipe",
    "os error 10053",
    "os error 10054",
    "os error 10060",
    "os error 104",
    "response too short",
    "timeout waiting for rpc_result",
    "no rpc_result after",
    "transport error",
];

pub fn is_fatal_session_error(err: &str) -> bool {
    FATAL_SESSION_ERRORS.iter().any(|m| err.contains(m))
}

pub fn is_network_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    NETWORK_ERRORS.iter().any(|m| lower.contains(&m.to_lowercase()))
}

// global salt cache per server address - avoids BAD_SERVER_SALT on first request
static SALT_CACHE: std::sync::LazyLock<Mutex<HashMap<String, u64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub struct MtpClient {
    transport: MtpTransport,
    auth_key: [u8; 256],
    session_id: u64,
    server_salt: u64,
    seq_no: u32,
    time_offset: i32,
    addr: String,
    last_msg_id: u64,
    log_event: Option<String>,
    log_handle: Option<tauri::AppHandle>,
    log_prefix: String,
    // upper bound on auto-retried FLOOD_WAIT/SLOWMODE_WAIT in seconds.
    // 0 = unlimited (legacy behavior). otherwise floods longer than this
    // are surfaced as errors instead of blocking the worker.
    max_flood_wait: u64,
    // stored proxy for reconnection
    proxy: Option<ProxyConfig>,
    // first fatal session error seen by invoke() during this client's lifetime.
    // workers that swallow per-call errors can consult this once at the end to
    // mark the account dead/frozen even if the error was logged-and-continued.
    fatal_error: Option<String>,
}

impl MtpClient {
    pub async fn connect(
        addr: &str,
        auth_key: &[u8; 256],
        proxy: Option<&ProxyConfig>,
    ) -> Result<Self, String> {
        dbg_log!("MtpClient::connect addr={} auth_key_id={:#018x}", addr, crypto::auth_key_id(auth_key));

        let transport = MtpTransport::connect(addr, proxy).await?;
        let session_id: u64 = rand::thread_rng().gen();

        // use cached salt for this DC if available (avoids BAD_SERVER_SALT round-trip)
        let cached_salt = SALT_CACHE.lock().ok()
            .and_then(|cache| cache.get(addr).copied())
            .unwrap_or(0);

        Ok(Self {
            transport,
            auth_key: *auth_key,
            session_id,
            server_salt: cached_salt,
            seq_no: 0,
            time_offset: 0,
            addr: addr.to_string(),
            last_msg_id: 0,
            log_event: None,
            log_handle: None,
            log_prefix: String::new(),
            max_flood_wait: 0,
            proxy: proxy.cloned(),
            fatal_error: None,
        })
    }

    // build a client from an already-connected transport with a freshly negotiated auth_key + salt
    pub fn from_transport(
        transport: MtpTransport,
        auth_key: [u8; 256],
        server_salt: u64,
        addr: &str,
    ) -> Self {
        let session_id: u64 = rand::thread_rng().gen();
        if let Ok(mut cache) = SALT_CACHE.lock() {
            cache.insert(addr.to_string(), server_salt);
        }
        Self {
            transport,
            auth_key,
            session_id,
            server_salt,
            seq_no: 0,
            time_offset: 0,
            addr: addr.to_string(),
            last_msg_id: 0,
            log_event: None,
            log_handle: None,
            log_prefix: String::new(),
            max_flood_wait: 0,
            proxy: None,
            fatal_error: None,
        }
    }

    pub fn auth_key(&self) -> &[u8; 256] {
        &self.auth_key
    }

    pub fn server_salt(&self) -> u64 {
        self.server_salt
    }

    pub fn set_proxy(&mut self, proxy: Option<ProxyConfig>) {
        self.proxy = proxy;
    }

    pub fn set_log_target(&mut self, event_name: &str, handle: tauri::AppHandle) {
        self.log_event = Some(event_name.to_string());
        self.log_handle = Some(handle);
    }

    pub fn set_log_prefix(&mut self, prefix: &str) {
        self.log_prefix = prefix.to_string();
    }

    // 0 = unlimited (default). any non-zero value caps how long invoke() will
    // sleep for FLOOD_WAIT before returning an error to the caller.
    pub fn set_max_flood_wait(&mut self, seconds: u64) {
        self.max_flood_wait = seconds;
    }

    // the first fatal session error invoke() encountered, if any. lets workers
    // that swallow per-call errors still detect a dead/frozen session at the end.
    pub fn fatal_error(&self) -> Option<&str> {
        self.fatal_error.as_deref()
    }

    fn emit_log(&self, msg: &str) {
        if let (Some(event), Some(handle)) = (&self.log_event, &self.log_handle) {
            use tauri::Emitter;
            let full_msg = if self.log_prefix.is_empty() {
                msg.to_string()
            } else {
                format!("{} {}", self.log_prefix, msg)
            };
            let _ = handle.emit(event, full_msg);
        }
    }

    // re-establish the TCP connection (new session_id, reset seq_no)
    // preserves auth_key, salt cache, log target, max_flood_wait
    pub async fn reconnect(&mut self) -> Result<(), String> {
        dbg_log!("MtpClient::reconnect addr={}", self.addr);
        self.emit_log(&crate::i18n::t("mtproto_reconnecting"));

        let transport = MtpTransport::connect(&self.addr, self.proxy.as_ref()).await?;
        self.transport = transport;
        self.session_id = rand::thread_rng().gen();
        self.seq_no = 0;
        self.last_msg_id = 0;

        // restore cached salt
        if let Ok(cache) = SALT_CACHE.lock() {
            if let Some(&salt) = cache.get(&self.addr) {
                self.server_salt = salt;
            }
        }

        dbg_log!("MtpClient::reconnect OK");
        Ok(())
    }

    // invoke with automatic reconnect on network errors (up to 5 attempts)
    // fatal session errors (AUTH_KEY_UNREGISTERED etc.) are never retried
    pub async fn invoke(&mut self, request: &[u8]) -> Result<Vec<u8>, String> {
        let mut last_err = String::new();
        for attempt in 0..5u32 {
            let result = self.invoke_once(request).await;
            match result {
                Ok(data) => return Ok(data),
                Err(e) => {
                    if is_fatal_session_error(&e) {
                        // remember the first fatal error so workers that
                        // log-and-continue still mark the account afterwards
                        if self.fatal_error.is_none() {
                            self.fatal_error = Some(e.clone());
                        }
                        return Err(e);
                    }
                    if is_network_error(&e) {
                        last_err = e;
                        if attempt < 4 {
                            let delay = match attempt {
                                0 => 500,
                                1 => 1000,
                                2 => 2000,
                                _ => 3000,
                            };
                            self.emit_log(&crate::i18n::t_with(
                                "mtproto_network_error",
                                &[("attempt", &(attempt + 1).to_string()), ("error", &last_err), ("delay", &delay.to_string())]
                            ));
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            if let Err(re) = self.reconnect().await {
                                last_err = format!("reconnect: {re}");
                                continue;
                            }
                        }
                        continue;
                    }
                    // non-network, non-fatal error — return immediately
                    return Err(e);
                }
            }
        }
        Err(format!("connect (5 attempts): {last_err}"))
    }

    // single invoke attempt (the original invoke logic)
    async fn invoke_once(&mut self, request: &[u8]) -> Result<Vec<u8>, String> {
        let msg_id = self.gen_msg_id();
        let seq_no = self.next_seq_no(true);

        dbg_log!("MtpClient::invoke msg_id={:#018x} seq_no={} len={}", msg_id, seq_no, request.len());

        let mut plaintext = Vec::new();
        plaintext.write_u64::<LittleEndian>(self.server_salt).unwrap();
        plaintext.write_u64::<LittleEndian>(self.session_id).unwrap();
        plaintext.write_u64::<LittleEndian>(msg_id).unwrap();
        plaintext.write_u32::<LittleEndian>(seq_no).unwrap();
        plaintext.write_u32::<LittleEndian>(request.len() as u32).unwrap();
        plaintext.extend_from_slice(request);

        let encrypted = crypto::encrypt_message(&self.auth_key, &plaintext);
        self.transport.send(&encrypted).await?;

        let response = self.transport.recv().await?;
        dbg_log!("MtpClient::invoke got response {} bytes", response.len());

        if response.len() < 24 {
            return Err("response too short".into());
        }

        let resp_key_id = u64::from_le_bytes(response[0..8].try_into().unwrap());
        let expected_key_id = crypto::auth_key_id(&self.auth_key);
        if resp_key_id != expected_key_id {
            return Err("auth_key_id mismatch in response".into());
        }

        let decrypted = crypto::decrypt_message(&self.auth_key, &response)?;
        if decrypted.len() < 32 {
            return Err("decrypted message too short".into());
        }

        let body_len = u32::from_le_bytes(decrypted[28..32].try_into().unwrap()) as usize;
        if decrypted.len() < 32 + body_len {
            return Err("body length mismatch".into());
        }

        let body = &decrypted[32..32 + body_len];

        if body.len() >= 4 {
            let ctor = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
            dbg_log!("MtpClient::invoke response ctor={:#010x}", ctor);

            // bad_server_salt - retry with new salt
            if ctor == super::service_ctors::BAD_SERVER_SALT && body.len() >= 28 {
                let new_salt = u64::from_le_bytes(body[20..28].try_into().unwrap());
                dbg_log!("MtpClient::invoke BAD_SERVER_SALT new_salt={:#018x}", new_salt);
                self.server_salt = new_salt;
                // cache salt for future connections to this DC
                if let Ok(mut cache) = SALT_CACHE.lock() {
                    cache.insert(self.addr.clone(), new_salt);
                }
                return self.invoke_inner(request).await;
            }

            // bad_msg_notification - adjust time if needed
            if ctor == super::service_ctors::BAD_MSG_NOTIFICATION && body.len() >= 16 {
                let error_code = u32::from_le_bytes(body[12..16].try_into().unwrap());
                dbg_log!("MtpClient::invoke BAD_MSG error_code={}", error_code);
                if error_code == 32 || error_code == 33 {
                    let server_time = (u64::from_le_bytes(decrypted[16..24].try_into().unwrap()) >> 32) as i64;
                    let local_time = SystemTime::now()
                        .duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
                    self.time_offset = (server_time - local_time) as i32;
                    dbg_log!("MtpClient::invoke adjusted time_offset={}", self.time_offset);
                    return self.invoke_inner(request).await;
                }
                return Err(format!("bad_msg_notification error_code={}", error_code));
            }

            // service messages that are not responses to our request - read next packet
            if is_service_or_update_ctor(ctor) {
                dbg_log!("MtpClient::invoke skipping service/update ctor={:#010x}, reading next...", ctor);
                return self.read_next_rpc_response().await;
            }

            // bare top-level gzip_packed: telegram compresses large rpc_result bodies
            // wholesale, so this may be our answer rather than a push update. decompress
            // and inspect the inner ctor before deciding.
            if ctor == super::service_ctors::GZIP_PACKED {
                if let Some(decompressed) = decompress_top_gzip(body) {
                    if decompressed.len() >= 4 {
                        let inner = u32::from_le_bytes([decompressed[0], decompressed[1], decompressed[2], decompressed[3]]);
                        if inner == super::service_ctors::RPC_RESULT || inner == super::service_ctors::MSG_CONTAINER {
                            dbg_log!("MtpClient::invoke gzip wraps rpc_result/container, unwrapping");
                            let parsed = tl::parse_rpc_response(&decompressed);
                            return self.finalize_rpc(parsed, request).await;
                        }
                    }
                }
                dbg_log!("MtpClient::invoke skipping bare gzip_packed push update, reading next...");
                return self.read_next_rpc_response().await;
            }
        }

        // try parsing rpc_result; if container has no rpc_result, read next packet
        let parsed = tl::parse_rpc_response(body);
        self.finalize_rpc(parsed, request).await
    }

    // post-process a parsed rpc_result: surface rpc_error, auto-retry FLOOD_WAIT
    // within the configured cap, and chase missing rpc_results across packets
    async fn finalize_rpc(&mut self, parsed: Result<Vec<u8>, String>, request: &[u8]) -> Result<Vec<u8>, String> {
        match parsed {
            Ok(result) => {
                // check for rpc_error inside result
                if result.len() >= 4 {
                    let inner_ctor = u32::from_le_bytes([result[0], result[1], result[2], result[3]]);
                    if inner_ctor == super::service_ctors::RPC_ERROR { // RPC_ERROR
                        if let Some(wait) = parse_flood_wait(&result) {
                            if self.max_flood_wait > 0 && wait > self.max_flood_wait {
                                dbg_log!("MtpClient::invoke FLOOD_WAIT {} sec exceeds cap {} sec, prefix='{}', request_ctor={:?}, returning error", wait, self.max_flood_wait, self.log_prefix, request_constructor(request));
                                self.emit_log(&crate::i18n::t_with("mtproto_flood_over_limit", &[("wait", &wait.to_string()), ("limit", &self.max_flood_wait.to_string())]));
                                return Err(format!("FLOOD_WAIT_{}", wait));
                            }
                            dbg_log!("MtpClient::invoke FLOOD_WAIT {} sec, prefix='{}', request_ctor={:?}, sleeping...", wait, self.log_prefix, request_constructor(request));
                            self.emit_log(&crate::i18n::t_with("mtproto_flood_waiting", &[("wait", &wait.to_string())]));
                            tokio::time::sleep(std::time::Duration::from_secs(wait + 1)).await;
                            return self.invoke_inner(request).await;
                        }
                    }
                }
                Ok(result)
            }
            Err(e) if e.contains("no rpc_result in container") => {
                dbg_log!("MtpClient::invoke container without rpc_result, reading next packet...");
                self.read_next_rpc_response().await
            }
            Err(e) if e.contains("FLOOD_WAIT") || e.contains("SLOWMODE_WAIT") => {
                if let Some(wait) = extract_wait_from_error(&e) {
                    if self.max_flood_wait > 0 && wait > self.max_flood_wait {
                        dbg_log!("MtpClient::invoke FLOOD_WAIT {} sec from error string exceeds cap {} sec, prefix='{}', request_ctor={:?}", wait, self.max_flood_wait, self.log_prefix, request_constructor(request));
                        self.emit_log(&crate::i18n::t_with("mtproto_flood_over_limit", &[("wait", &wait.to_string()), ("limit", &self.max_flood_wait.to_string())]));
                        return Err(e);
                    }
                    dbg_log!("MtpClient::invoke FLOOD_WAIT {} sec from error string, prefix='{}', request_ctor={:?}", wait, self.log_prefix, request_constructor(request));
                    self.emit_log(&crate::i18n::t_with("mtproto_flood_waiting", &[("wait", &wait.to_string())]));
                    tokio::time::sleep(std::time::Duration::from_secs(wait + 1)).await;
                    return self.invoke_inner(request).await;
                }
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    // read additional packets looking for rpc_result (up to 5 attempts)
    async fn read_next_rpc_response(&mut self) -> Result<Vec<u8>, String> {
        for _ in 0..5 {
            let response = match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                self.transport.recv()
            ).await {
                Ok(Ok(r)) => r,
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err("timeout waiting for rpc_result".into()),
            };

            if response.len() < 24 { continue; }

            let decrypted = crypto::decrypt_message(&self.auth_key, &response)?;
            if decrypted.len() < 32 { continue; }

            let body_len = u32::from_le_bytes(decrypted[28..32].try_into().unwrap()) as usize;
            if decrypted.len() < 32 + body_len { continue; }

            let body = &decrypted[32..32 + body_len];

            // skip service/update messages
            if body.len() >= 4 {
                let ctor = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
                if is_service_or_update_ctor(ctor) {
                    dbg_log!("MtpClient::read_next skipping {:#010x}", ctor);
                    continue;
                }
                // bare top-level gzip: may wrap our rpc_result or a push update
                if ctor == super::service_ctors::GZIP_PACKED {
                    if let Some(decompressed) = decompress_top_gzip(body) {
                        if decompressed.len() >= 4 {
                            let inner = u32::from_le_bytes([decompressed[0], decompressed[1], decompressed[2], decompressed[3]]);
                            if inner == super::service_ctors::RPC_RESULT || inner == super::service_ctors::MSG_CONTAINER {
                                match tl::parse_rpc_response(&decompressed) {
                                    Ok(result) => return Ok(result),
                                    Err(e) if e.contains("no rpc_result") => continue,
                                    Err(e) => return Err(e),
                                }
                            }
                        }
                    }
                    dbg_log!("MtpClient::read_next skipping bare gzip push update");
                    continue;
                }
            }

            match tl::parse_rpc_response(body) {
                Ok(result) => return Ok(result),
                Err(e) if e.contains("no rpc_result") => continue,
                Err(e) => return Err(e),
            }
        }
        Err("no rpc_result after multiple reads".into())
    }

    // inner invoke without recursive retry (prevents infinite loops)
    async fn invoke_inner(&mut self, request: &[u8]) -> Result<Vec<u8>, String> {
        let msg_id = self.gen_msg_id();
        let seq_no = self.next_seq_no(true);

        let mut plaintext = Vec::new();
        plaintext.write_u64::<LittleEndian>(self.server_salt).unwrap();
        plaintext.write_u64::<LittleEndian>(self.session_id).unwrap();
        plaintext.write_u64::<LittleEndian>(msg_id).unwrap();
        plaintext.write_u32::<LittleEndian>(seq_no).unwrap();
        plaintext.write_u32::<LittleEndian>(request.len() as u32).unwrap();
        plaintext.extend_from_slice(request);

        let encrypted = crypto::encrypt_message(&self.auth_key, &plaintext);
        self.transport.send(&encrypted).await?;

        // read responses until we get rpc_result
        for attempt in 0..10 {
            let response = self.transport.recv().await?;
            dbg_log!("MtpClient::invoke_inner recv[{}] {} bytes", attempt, response.len());

            if response.len() < 24 {
                return Err("response too short".into());
            }

            let resp_key_id = u64::from_le_bytes(response[0..8].try_into().unwrap());
            if resp_key_id != crypto::auth_key_id(&self.auth_key) {
                return Err("auth_key_id mismatch in response".into());
            }

            let decrypted = crypto::decrypt_message(&self.auth_key, &response)?;
            if decrypted.len() < 32 {
                return Err("decrypted message too short".into());
            }

            let body_len = u32::from_le_bytes(decrypted[28..32].try_into().unwrap()) as usize;
            if decrypted.len() < 32 + body_len {
                return Err("body length mismatch".into());
            }

            let body = &decrypted[32..32 + body_len];

            if body.len() >= 4 {
                let ctor = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);

                if ctor == super::service_ctors::BAD_MSG_NOTIFICATION && body.len() >= 16 {
                    let error_code = u32::from_le_bytes(body[12..16].try_into().unwrap());
                    return Err(format!("bad_msg_notification error_code={}", error_code));
                }

                // rpc_result
                if ctor == super::service_ctors::RPC_RESULT {
                    return tl::parse_rpc_response(body);
                }

                // msg_container - look inside
                if ctor == super::service_ctors::MSG_CONTAINER {
                    if let Ok(result) = tl::parse_rpc_response(body) {
                        return Ok(result);
                    }
                    continue;
                }

                // bare top-level gzip: may wrap our rpc_result/container or a push update
                if ctor == super::service_ctors::GZIP_PACKED {
                    if let Some(decompressed) = decompress_top_gzip(body) {
                        if decompressed.len() >= 4 {
                            let inner = u32::from_le_bytes([decompressed[0], decompressed[1], decompressed[2], decompressed[3]]);
                            if inner == super::service_ctors::RPC_RESULT || inner == super::service_ctors::MSG_CONTAINER {
                                if let Ok(result) = tl::parse_rpc_response(&decompressed) {
                                    return Ok(result);
                                }
                            }
                        }
                    }
                    dbg_log!("MtpClient::invoke_inner skipping bare gzip push update");
                    continue;
                }

                // service messages (new_session_created, msgs_ack, updates) - skip
                dbg_log!("MtpClient::invoke_inner skipping {:#010x}", ctor);
                continue;
            }
        }

        Err("no rpc_result after 10 reads".into())
    }

    pub async fn get_me(
        &mut self,
        api_id: i32,
        device: &str,
        system: &str,
        app_version: &str,
        system_lang: &str,
        lang: &str,
    ) -> Result<tl::UserInfo, String> {
        dbg_log!("MtpClient::get_me api_id={}", api_id);
        let request = tl::build_get_me_request(api_id, device, system, app_version, system_lang, lang);
        let response = self.invoke(&request).await?;
        tl::parse_users_response(&response)
    }

    fn gen_msg_id(&mut self) -> u64 {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
        let secs = now.as_secs() as i64 + self.time_offset as i64;
        let nanos = now.subsec_nanos();
        let mut id = ((secs as u64) << 32) | ((nanos as u64 / 1000) << 2) | 4;
        // ensure monotonically increasing
        if id <= self.last_msg_id {
            id = self.last_msg_id + 4;
        }
        self.last_msg_id = id;
        id
    }

    fn next_seq_no(&mut self, content_related: bool) -> u32 {
        let result = if content_related {
            self.seq_no * 2 + 1
        } else {
            self.seq_no * 2
        };
        if content_related {
            self.seq_no += 1;
        }
        result
    }

}


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
    pub is_creator: bool,
}


// true for mtproto service/push-update containers that are never a direct
// response to our rpc call and should be skipped while waiting for rpc_result.
fn is_service_or_update_ctor(ctor: u32) -> bool {
    use super::service_ctors as sc;
    matches!(ctor,
        x if x == sc::MSGS_ACK
          || x == sc::NEW_SESSION_CREATED
          || x == sc::UPDATES
          || x == sc::UPDATE_SHORT
          || x == sc::UPDATE_SHORT_MESSAGE
          || x == sc::UPDATES_COMBINED)
}

// decompress a body whose top-level ctor is gzip_packed#3072cfa1.
// returns the inner TL payload, or None on malformed input.
fn decompress_top_gzip(body: &[u8]) -> Option<Vec<u8>> {
    use std::io::Cursor;
    use byteorder::ReadBytesExt;
    if body.len() < 4 { return None; }
    let mut cursor = Cursor::new(body);
    let ctor = cursor.read_u32::<LittleEndian>().ok()?;
    if ctor != super::service_ctors::GZIP_PACKED { return None; }
    let compressed = tl::deserialize_bytes(&mut cursor).ok()?;
    tl::decompress_gzip(&compressed).ok()
}

// parse FLOOD_WAIT from raw rpc_error bytes
fn parse_flood_wait(data: &[u8]) -> Option<u64> {
    // rpc_error#2144ca19 error_code:int error_message:string
    if data.len() < 8 { return None; }
    let ctor = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if ctor != super::service_ctors::RPC_ERROR { return None; }
    // skip error_code (4 bytes)
    // parse error_message string
    let msg_start = 8; // ctor(4) + error_code(4)
    if msg_start >= data.len() { return None; }
    let first = data[msg_start] as usize;
    let (msg_bytes, _) = if first < 254 {
        let len = first;
        if msg_start + 1 + len > data.len() { return None; }
        (&data[msg_start + 1..msg_start + 1 + len], msg_start + 1 + len)
    } else {
        if msg_start + 4 > data.len() { return None; }
        let len = data[msg_start + 1] as usize | (data[msg_start + 2] as usize) << 8 | (data[msg_start + 3] as usize) << 16;
        if msg_start + 4 + len > data.len() { return None; }
        (&data[msg_start + 4..msg_start + 4 + len], msg_start + 4 + len)
    };
    let msg = std::str::from_utf8(msg_bytes).ok()?;
    extract_wait_from_error(msg)
}

fn request_constructor(request: &[u8]) -> Option<u32> {
    request
        .get(..4)
        .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

// extract wait seconds from error message like "FLOOD_WAIT_35" or "SLOWMODE_WAIT_60"
fn extract_wait_from_error(msg: &str) -> Option<u64> {
    for pattern in &["FLOOD_WAIT_", "SLOWMODE_WAIT_", "FLOOD_PREMIUM_WAIT_"] {
        if let Some(rest) = msg.strip_prefix(pattern) {
            if let Ok(n) = rest.trim().parse::<u64>() {
                return Some(n);
            }
        }
        // also handle when it's inside a longer string like "rpc error 420: FLOOD_WAIT_35"
        if let Some(pos) = msg.find(pattern) {
            let after = &msg[pos + pattern.len()..];
            let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = num_str.parse::<u64>() {
                return Some(n);
            }
        }
    }
    None
}
