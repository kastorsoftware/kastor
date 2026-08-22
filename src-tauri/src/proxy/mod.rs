use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

// proxy list cache
static PROXY_CACHE: std::sync::LazyLock<StdMutex<Option<ProxyList>>> =
    std::sync::LazyLock::new(|| StdMutex::new(None));

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProxyType {
    Socks5,
    Socks4,
    Https,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ProxyStatus {
    Valid,
    Invalid,
    Unchecked,
    Checking,
}

impl Default for ProxyStatus {
    fn default() -> Self {
        Self::Unchecked
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    pub id: String,
    pub proxy_type: ProxyType,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    #[serde(default)]
    pub status: ProxyStatus,
    #[serde(default)]
    pub last_check: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxyList {
    pub proxies: Vec<ProxyConfig>,
}

impl ProxyConfig {
    // supported formats:
    // socks5://user:pass@host:port
    // socks4://host:port
    // https://user:pass@host:port
    // host:port:user:pass
    // host:port@user:pass
    // host:port (default socks5)
    pub fn from_string(s: &str) -> Result<Self, String> {
        let s = s.trim();
        dbg_log!("proxy::from_string input: '{}'", s);

        let id = uuid::Uuid::new_v4().to_string();

        // check for scheme prefix
        let (ptype, rest) = if let Some(r) = s.strip_prefix("socks5://") {
            (ProxyType::Socks5, r.to_string())
        } else if let Some(r) = s.strip_prefix("socks4://") {
            (ProxyType::Socks4, r.to_string())
        } else if let Some(r) = s.strip_prefix("https://") {
            (ProxyType::Https, r.to_string())
        } else if let Some(r) = s.strip_prefix("http://") {
            (ProxyType::Https, r.to_string())
        } else {
            // no scheme - check for @ separator: ip:port@login:pass
            if let Some(at_pos) = s.find('@') {
                let hostport_part = &s[..at_pos];
                let auth_part = &s[at_pos + 1..];
                let hp_parts: Vec<&str> = hostport_part.split(':').collect();
                if hp_parts.len() == 2 {
                    let host = hp_parts[0].to_string();
                    let port = hp_parts[1].parse::<u16>().map_err(|_| "invalid port".to_string())?;
                    let (user, pass) = if let Some(colon) = auth_part.find(':') {
                        (Some(auth_part[..colon].to_string()), Some(auth_part[colon + 1..].to_string()))
                    } else {
                        (Some(auth_part.to_string()), None)
                    };
                    return Ok(ProxyConfig {
                        id,
                        proxy_type: ProxyType::Socks5,
                        host,
                        port,
                        username: user,
                        password: pass,
                        status: ProxyStatus::Unchecked,
                        last_check: None,
                    });
                }
            }

            // try colon-separated: host:port:user:pass or host:port
            let parts: Vec<&str> = s.split(':').collect();

            if parts.len() == 4 {
                let host = parts[0].to_string();
                let port = parts[1].parse::<u16>().map_err(|_| "invalid port".to_string())?;
                let user = parts[2].to_string();
                let pass = parts[3].to_string();
                return Ok(ProxyConfig {
                    id,
                    proxy_type: ProxyType::Socks5,
                    host,
                    port,
                    username: Some(user),
                    password: Some(pass),
                    status: ProxyStatus::Unchecked,
                    last_check: None,
                });
            } else if parts.len() == 2 {
                let host = parts[0].to_string();
                let port = parts[1].parse::<u16>().map_err(|_| "invalid port".to_string())?;
                return Ok(ProxyConfig {
                    id,
                    proxy_type: ProxyType::Socks5,
                    host,
                    port,
                    username: None,
                    password: None,
                    status: ProxyStatus::Unchecked,
                    last_check: None,
                });
            } else if parts.len() >= 3 {
                let host = parts[0].to_string();
                if let Ok(port) = parts[1].parse::<u16>() {
                    let user = parts[2].to_string();
                    let pass = if parts.len() > 3 { parts[3..].join(":") } else { String::new() };
                    return Ok(ProxyConfig {
                        id,
                        proxy_type: ProxyType::Socks5,
                        host,
                        port,
                        username: Some(user),
                        password: if pass.is_empty() { None } else { Some(pass) },
                        status: ProxyStatus::Unchecked,
                        last_check: None,
                    });
                }
            }

            // fallback: treat as scheme-less with @ syntax
            (ProxyType::Socks5, s.to_string())
        };

        // parse scheme:// format with optional user:pass@
        let (auth, hostport) = if let Some(at_pos) = rest.rfind('@') {
            let auth_part = &rest[..at_pos];
            let hp = &rest[at_pos + 1..];
            let (user, pass) = if let Some(colon) = auth_part.find(':') {
                (Some(auth_part[..colon].to_string()), Some(auth_part[colon + 1..].to_string()))
            } else {
                (Some(auth_part.to_string()), None)
            };
            ((user, pass), hp.to_string())
        } else {
            ((None, None), rest)
        };

        let (host, port) = if let Some(colon) = hostport.rfind(':') {
            let h = hostport[..colon].to_string();
            let p = hostport[colon + 1..]
                .parse::<u16>()
                .map_err(|_| "invalid port".to_string())?;
            (h, p)
        } else {
            return Err("missing port".to_string());
        };

        Ok(ProxyConfig {
            id,
            proxy_type: ptype,
            host,
            port,
            username: auth.0,
            password: auth.1,
            status: ProxyStatus::Unchecked,
            last_check: None,
        })
    }

    pub fn to_string_repr(&self) -> String {
        let scheme = match self.proxy_type {
            ProxyType::Socks5 => "socks5",
            ProxyType::Socks4 => "socks4",
            ProxyType::Https => "https",
        };
        let auth = match (&self.username, &self.password) {
            (Some(u), Some(p)) => format!("{u}:{p}@"),
            (Some(u), None) => format!("{u}@"),
            _ => String::new(),
        };
        format!("{scheme}://{auth}{}:{}", self.host, self.port)
    }
}

impl ProxyList {
    pub fn config_path() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("kastor")
            .join("proxies.json")
    }

    pub fn load() -> Self {
        if let Ok(cache) = PROXY_CACHE.lock() {
            if let Some(ref cached) = *cache {
                return cached.clone();
            }
        }
        let path = Self::config_path();
        let list = if path.exists() {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        };
        if let Ok(mut cache) = PROXY_CACHE.lock() {
            *cache = Some(list.clone());
        }
        list
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("serialize failed: {e}"))?;
        std::fs::write(&path, content)
            .map_err(|e| format!("write failed: {e}"))?;
        // update cache
        if let Ok(mut cache) = PROXY_CACHE.lock() {
            *cache = Some(self.clone());
        }
        Ok(())
    }

    pub fn get_random(&self) -> Option<&ProxyConfig> {
        if self.proxies.is_empty() {
            return None;
        }
        use rand::Rng;
        let idx = rand::thread_rng().gen_range(0..self.proxies.len());
        Some(&self.proxies[idx])
    }
}

// resolve a proxy for an account in a long-running task.
// priority: account-assigned > random from pool > none (only if allow_no_proxy = true).
// returns Err with a user-facing ru message when no proxy is available
// and the global allow_no_proxy flag is off.
pub fn select_proxy_for_account(account_proxy: Option<&str>) -> Result<Option<ProxyConfig>, String> {
    if let Some(s) = account_proxy {
        if let Ok(cfg) = ProxyConfig::from_string(s) {
            return Ok(Some(cfg));
        }
    }
    let list = ProxyList::load();
    if let Some(rand) = list.get_random() {
        return Ok(Some(rand.clone()));
    }
    let settings = crate::settings::AppSettings::load();
    if settings.allow_no_proxy {
        Ok(None)
    } else {
        Err(crate::i18n::t("proxy_no_available"))
    }
}

// validate proxy by connecting to telegram dc2
// 5 attempts with progressive delays: 200, 300, 800, 800, 800
pub async fn validate_proxy(proxy: &ProxyConfig) -> bool {
    let delays = [0, 200, 300, 800, 800];
    for attempt in 0..5 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delays[attempt])).await;
        }
        match tokio::time::timeout(
            std::time::Duration::from_secs(7),
            connect_via_proxy(proxy, "149.154.167.51", 443),
        ).await {
            Ok(Ok(_)) => return true,
            _ => {}
        }
    }
    false
}

pub async fn validate_proxies_batch(list: &mut ProxyList, ids: &[String], concurrency: usize) -> Vec<(String, bool)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string();

    let proxies_to_check: Vec<(String, ProxyConfig)> = ids.iter()
        .filter_map(|id| {
            list.proxies.iter().find(|p| p.id == *id).map(|p| (id.clone(), p.clone()))
        })
        .collect();

    dbg_log!("proxy::validate_batch {} proxies, concurrency={}", proxies_to_check.len(), concurrency);

    // mark all as Checking before starting
    for id in ids {
        if let Some(p) = list.proxies.iter_mut().find(|p| p.id == *id) {
            p.status = ProxyStatus::Checking;
        }
    }
    list.save().ok();

    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let shared_list = Arc::new(tokio::sync::Mutex::new(list.clone()));
    let now_clone = now.clone();
    let mut handles = Vec::new();

    for (id, proxy) in proxies_to_check {
        let permit = sem.clone();
        let shared = shared_list.clone();
        let ts = now_clone.clone();
        handles.push(tokio::spawn(async move {
            let _permit = permit.acquire().await.unwrap();
            let valid = validate_proxy(&proxy).await;

            // update status in memory
            {
                let mut plist = shared.lock().await;
                if let Some(p) = plist.proxies.iter_mut().find(|p| p.id == id) {
                    p.status = if valid { ProxyStatus::Valid } else { ProxyStatus::Invalid };
                    p.last_check = Some(ts.clone());
                }
                // save to disk every 5 completions for UI responsiveness
                let done_count = plist.proxies.iter()
                    .filter(|p| p.status != ProxyStatus::Checking)
                    .count();
                if done_count % 5 == 0 {
                    plist.save().ok();
                }
            }

            (id, valid)
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        if let Ok(result) = handle.await {
            results.push(result);
        }
    }

    // final save + sync back
    let final_list = shared_list.lock().await.clone();
    *list = final_list;
    list.save().ok();

    results
}

pub async fn connect_via_proxy(
    proxy: &ProxyConfig,
    target_host: &str,
    target_port: u16,
) -> Result<TcpStream, String> {
    let proxy_addr = format!("{}:{}", proxy.host, proxy.port);
    dbg_log!("proxy::connect_via_proxy to {} via {:?} {}", target_host, proxy.proxy_type, proxy_addr);

    let mut stream = TcpStream::connect(&proxy_addr)
        .await
        .map_err(|e| crate::i18n::t_with("proxy_connect_error", &[("error", &e.to_string())]))?;

    match proxy.proxy_type {
        ProxyType::Socks5 => socks5_handshake(&mut stream, proxy, target_host, target_port).await,
        ProxyType::Socks4 => socks4_handshake(&mut stream, proxy, target_host, target_port).await,
        ProxyType::Https => https_connect(&mut stream, proxy, target_host, target_port).await,
    }?;

    Ok(stream)
}

async fn socks5_handshake(
    stream: &mut TcpStream,
    proxy: &ProxyConfig,
    target_host: &str,
    target_port: u16,
) -> Result<(), String> {
    let has_auth = proxy.username.is_some();

    if has_auth {
        stream.write_all(&[0x05, 0x02, 0x00, 0x02]).await
            .map_err(|e| format!("socks5 greeting write: {e}"))?;
    } else {
        stream.write_all(&[0x05, 0x01, 0x00]).await
            .map_err(|e| format!("socks5 greeting write: {e}"))?;
    }

    let mut resp = [0u8; 2];
    stream.read_exact(&mut resp).await
        .map_err(|e| format!("socks5 greeting read: {e}"))?;

    if resp[0] != 0x05 {
        return Err(format!("socks5: invalid version {:#04x}", resp[0]));
    }

    if resp[1] == 0x02 {
        let user = proxy.username.as_deref().unwrap_or("");
        let pass = proxy.password.as_deref().unwrap_or("");
        let mut auth_req = vec![0x01, user.len() as u8];
        auth_req.extend_from_slice(user.as_bytes());
        auth_req.push(pass.len() as u8);
        auth_req.extend_from_slice(pass.as_bytes());
        stream.write_all(&auth_req).await
            .map_err(|e| format!("socks5 auth write: {e}"))?;

        let mut auth_resp = [0u8; 2];
        stream.read_exact(&mut auth_resp).await
            .map_err(|e| format!("socks5 auth read: {e}"))?;
        if auth_resp[1] != 0x00 {
            return Err("socks5: authentication failed".into());
        }
    } else if resp[1] != 0x00 {
        return Err(format!("socks5: no acceptable auth method (got {:#04x})", resp[1]));
    }

    // connect request using domain name
    let mut connect_req = vec![0x05, 0x01, 0x00, 0x03];
    connect_req.push(target_host.len() as u8);
    connect_req.extend_from_slice(target_host.as_bytes());
    connect_req.push((target_port >> 8) as u8);
    connect_req.push((target_port & 0xff) as u8);
    stream.write_all(&connect_req).await
        .map_err(|e| format!("socks5 connect write: {e}"))?;

    let mut connect_resp = [0u8; 4];
    stream.read_exact(&mut connect_resp).await
        .map_err(|e| format!("socks5 connect read: {e}"))?;

    if connect_resp[1] != 0x00 {
        return Err(format!("socks5: connect failed with code {:#04x}", connect_resp[1]));
    }

    // skip bound address
    match connect_resp[3] {
        0x01 => {
            let mut skip = [0u8; 6];
            stream.read_exact(&mut skip).await.ok();
        }
        0x03 => {
            let mut len_buf = [0u8; 1];
            stream.read_exact(&mut len_buf).await.ok();
            let mut skip = vec![0u8; len_buf[0] as usize + 2];
            stream.read_exact(&mut skip).await.ok();
        }
        0x04 => {
            let mut skip = [0u8; 18];
            stream.read_exact(&mut skip).await.ok();
        }
        _ => {}
    }

    Ok(())
}

async fn socks4_handshake(
    stream: &mut TcpStream,
    proxy: &ProxyConfig,
    target_host: &str,
    target_port: u16,
) -> Result<(), String> {
    let target_addr = format!("{target_host}:{target_port}");
    let addrs: Vec<_> = tokio::net::lookup_host(&target_addr)
        .await
        .map_err(|e| format!("dns resolve failed: {e}"))?
        .collect();

    let ip = addrs.iter()
        .find_map(|a| match a {
            std::net::SocketAddr::V4(v4) => Some(v4.ip().octets()),
            _ => None,
        })
        .ok_or_else(|| "no ipv4 address found".to_string())?;

    let user = proxy.username.as_deref().unwrap_or("");
    let mut req = vec![0x04, 0x01];
    req.push((target_port >> 8) as u8);
    req.push((target_port & 0xff) as u8);
    req.extend_from_slice(&ip);
    req.extend_from_slice(user.as_bytes());
    req.push(0x00);

    stream.write_all(&req).await
        .map_err(|e| format!("socks4 write: {e}"))?;

    let mut resp = [0u8; 8];
    stream.read_exact(&mut resp).await
        .map_err(|e| format!("socks4 read: {e}"))?;

    if resp[1] != 0x5a {
        return Err(format!("socks4: connect rejected with code {:#04x}", resp[1]));
    }

    Ok(())
}

async fn https_connect(
    stream: &mut TcpStream,
    proxy: &ProxyConfig,
    target_host: &str,
    target_port: u16,
) -> Result<(), String> {
    let mut connect_line = format!(
        "CONNECT {target_host}:{target_port} HTTP/1.1\r\nHost: {target_host}:{target_port}\r\n"
    );
    if let Some(username) = proxy.username.as_deref() {
        let password = proxy.password.as_deref().unwrap_or("");
        if username.contains(['\r', '\n']) || password.contains(['\r', '\n']) {
            return Err("https proxy credentials contain a line break".into());
        }
        let credentials = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            format!("{username}:{password}"),
        );
        connect_line.push_str(&format!("Proxy-Authorization: Basic {credentials}\r\n"));
    }
    connect_line.push_str("\r\n");
    stream.write_all(connect_line.as_bytes()).await
        .map_err(|e| format!("https connect write: {e}"))?;

    let mut buf = Vec::with_capacity(512);
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).await
            .map_err(|e| format!("https connect read: {e}"))?;
        buf.push(byte[0]);
        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
            break;
        }
        if buf.len() > 4096 {
            return Err("https connect: response too large".into());
        }
    }

    let response = String::from_utf8_lossy(&buf);
    if !response.contains("200") {
        return Err(format!("https connect failed: {}", response.lines().next().unwrap_or("")));
    }

    Ok(())
}

#[tauri::command]
pub async fn enqueue_validate_proxies(
    ids: Vec<String>,
    threads: Option<usize>,
    queue: tauri::State<'_, crate::queue::TaskQueue>,
) -> Result<String, String> {
    let task_id = uuid::Uuid::new_v4().to_string();
    let tid = task_id.clone();
    let concurrency = threads.unwrap_or(10).max(1).min(1000);
    let count = if ids.is_empty() {
        ProxyList::load().proxies.len()
    } else {
        ids.len()
    };

    queue.enqueue(
        task_id.clone(),
        "proxy_validate".to_string(),
        crate::i18n::t_with("proxy_validate_task", &[("count", &count.to_string())]),
        move || {
            Box::pin(async move {
                let mut list = ProxyList::load();
                let target_ids: Vec<String> = if ids.is_empty() {
                    list.proxies.iter().map(|p| p.id.clone()).collect()
                } else {
                    ids
                };
                validate_proxies_batch(&mut list, &target_ids, concurrency).await;
                Ok(())
            })
        },
    );

    Ok(tid)
}
