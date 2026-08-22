// Browser manager: opens a dedicated Chrome/Chromium instance for an account,
// routes traffic through a local HTTP->upstream proxy, and injects Telegram Web
// localStorage via Chrome DevTools Protocol.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{LazyLock, Mutex as StdMutex};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use futures_util::{future::join_all, SinkExt, StreamExt};
use httparse;
use rand::Rng;
use serde_json::{json, Map, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

use crate::accounts::connect::connect_account_with_info;
use crate::accounts::devices;
use crate::accounts::session::{AccountJson, TelethonSession};
use crate::accounts::storage::AccountStorage;
use crate::i18n::{t, t_with};
use crate::mtproto::auth::perform_dh;
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl::UserInfo;
use crate::mtproto::tl_gen;
use crate::mtproto::transport::MtpTransport;
use crate::proxy::ProxyConfig;

pub struct BrowserInstance {
    chrome: Child,
    _proxy_handle: JoinHandle<()>,
    _proxy_abort: Option<oneshot::Sender<()>>,
}

static RUNNING_BROWSERS: LazyLock<StdMutex<HashMap<String, BrowserInstance>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

const CHROME_BASE_DIR: &str = "chrome";

impl BrowserInstance {
    pub async fn open_telegram_web(
        account_id: String,
        proxy_url: Option<String>,
    ) -> Result<String, String> {
        dbg_log!(
            "BrowserInstance::open_telegram_web id={} proxy={:?}",
            account_id,
            proxy_url
        );

        let accounts_base = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("kastor")
            .join("accounts");
        let storage = AccountStorage::new(&accounts_base);
        let session_path = storage.session_path(&account_id);
        let json_path = storage.json_path(&account_id);

        if !session_path.exists() {
            return Err(t("acc_session_not_found"));
        }

        let session = TelethonSession::from_file(&session_path)?;
        let json = if json_path.exists() {
            AccountJson::from_file(&json_path).unwrap_or_default()
        } else {
            AccountJson::default()
        };

        let user_id = {
            let from_session = TelethonSession::get_user_id(&session_path);
            if from_session != 0 {
                from_session
            } else {
                json.user_id
            }
        };
        if user_id == 0 {
            return Err(t("acc_userid_empty"));
        }

        let (mut mtp_client, user_info, server_salt) = connect_account_with_info(&account_id)
            .await
            .map_err(|e| format!("mtproto fetch: {e}"))?;
        let init_device = if !json.device.is_empty() && !json.sdk.is_empty() {
            devices::DeviceInfo {
                device: json.device.clone(),
                sdk: json.sdk.clone(),
                app_version: json.app_version.clone(),
            }
        } else {
            devices::generate_random_device()
        };
        let app_id = if json.app_id == 0 {
            crate::get_app_config().app_id
        } else {
            json.app_id
        };

        let proxy_config = parse_proxy_string(proxy_url.as_deref())?;
        let dc_auths = collect_web_dc_auths(
            &mut mtp_client,
            &session,
            server_salt,
            proxy_config.as_ref(),
            app_id,
            &init_device,
        )
        .await?;
        let data = build_telegram_web_data(&session, &json, user_id, &dc_auths, &user_info)?;
        let script = build_inject_script(&data);

        let base_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("kastor");
        let chrome_dir = base_dir.join(CHROME_BASE_DIR);
        let chrome_exe = tokio::task::spawn_blocking(move || find_or_download_chrome(&chrome_dir))
            .await
            .map_err(|e| format!("chrome lookup: {e}"))??;

        let profile_dir = base_dir.join("browser_profiles").join(&account_id);
        std::fs::create_dir_all(&profile_dir).ok();

        // Start local HTTP proxy that forwards to the upstream proxy.
        let (proxy_port, proxy_handle, proxy_abort) =
            start_local_proxy(proxy_config.as_ref()).await?;

        let cdp_port = pick_free_port().await?;
        let chrome = spawn_chrome(&chrome_exe, &profile_dir, proxy_port, cdp_port).await?;

        let cdp_client = connect_cdp(cdp_port).await?;
        inject_and_navigate(cdp_client, &script, "https://web.telegram.org/a/").await?;

        let instance = Self {
            chrome,
            _proxy_handle: proxy_handle,
            _proxy_abort: Some(proxy_abort),
        };

        {
            let mut map = RUNNING_BROWSERS.lock().unwrap();
            if let Some(mut old) = map.remove(&account_id) {
                old.stop();
            }
            map.insert(account_id.clone(), instance);
        }

        Ok(account_id)
    }

    pub fn stop(&mut self) {
        if let Some(abort) = self._proxy_abort.take() {
            let _ = abort.send(());
        }
        let _ = self.chrome.start_kill();
    }
}

impl Drop for BrowserInstance {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Debug, Clone)]
struct WebDcAuth {
    dc_id: i32,
    auth_key_hex: String,
    server_salt_hex: String,
}

#[derive(Debug, Clone)]
struct ExportedDcAuth {
    dc_id: i32,
    id: i64,
    bytes: Vec<u8>,
}

fn find_or_download_chrome(base_dir: &Path) -> Result<PathBuf, String> {
    if let Some(path) = find_system_chrome() {
        return Ok(path);
    }
    find_or_download_portable_chrome(base_dir)
}

fn find_system_chrome() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec![
        r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe".into(),
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe".into(),
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(),
    ];
    if let Some(local) = dirs::data_local_dir() {
        candidates.push(local.join(r"Google\Chrome\Application\chrome.exe"));
    }
    for path in candidates {
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn find_or_download_portable_chrome(base_dir: &Path) -> Result<PathBuf, String> {
    let chrome_exe = base_dir.join("chrome-win64").join("chrome.exe");
    if chrome_exe.exists() {
        return Ok(chrome_exe);
    }

    std::fs::create_dir_all(base_dir).ok();
    let zip_path = base_dir.join("chrome-win64.zip");

    let url = chrome_for_testing_win64_url()
        .map_err(|e| t_with("browser_chrome_download_error", &[("error", &e)]))?;

    dbg_log!("browser: downloading Chrome from {}", url);
    let body = ureq::get(&url)
        .call()
        .map_err(|e| {
            t_with(
                "browser_chrome_download_error",
                &[("error", &e.to_string())],
            )
        })?
        .into_body()
        .read_to_vec()
        .map_err(|e| {
            t_with(
                "browser_chrome_download_error",
                &[("error", &e.to_string())],
            )
        })?;

    std::fs::write(&zip_path, &body).map_err(|e| {
        t_with(
            "browser_chrome_download_error",
            &[("error", &e.to_string())],
        )
    })?;

    extract_zip(&zip_path, base_dir)
        .map_err(|e| t_with("browser_chrome_extract_error", &[("error", &e)]))?;

    if chrome_exe.exists() {
        Ok(chrome_exe)
    } else {
        Err(t("browser_chrome_not_found_after_extract"))
    }
}

fn chrome_for_testing_win64_url() -> Result<String, String> {
    let url = "https://googlechromelabs.github.io/chrome-for-testing/known-good-versions-with-downloads.json";
    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("versions request: {e}"))?;
    let json: Value = resp
        .into_body()
        .read_json()
        .map_err(|e| format!("versions json: {e}"))?;

    let versions = json["versions"]
        .as_array()
        .ok_or("versions array missing")?;
    for version in versions.iter().rev() {
        if let Some(downloads) = version["downloads"].as_object() {
            if let Some(chrome) = downloads.get("chrome").and_then(|v| v.as_array()) {
                for entry in chrome {
                    if entry["platform"].as_str() == Some("win64") {
                        if let Some(u) = entry["url"].as_str() {
                            return Ok(u.to_string());
                        }
                    }
                }
            }
        }
    }
    Err("no win64 chrome download found".to_string())
}

fn extract_zip(zip_path: &Path, out_dir: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("open zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("read zip: {e}"))?;
    archive
        .extract(out_dir)
        .map_err(|e| format!("extract zip: {e}"))?;
    Ok(())
}

async fn spawn_chrome(
    chrome_exe: &Path,
    profile_dir: &Path,
    proxy_port: u16,
    cdp_port: u16,
) -> Result<Child, String> {
    let chrome_args = [
        format!("--user-data-dir={}", profile_dir.display()),
        format!("--proxy-server=http://127.0.0.1:{}", proxy_port),
        format!("--remote-debugging-port={}", cdp_port),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-dev-shm-usage".to_string(),
        "--disable-background-networking".to_string(),
        "--disable-component-update".to_string(),
        "--disable-domain-reliability".to_string(),
        "--disable-features=OptimizationHints,AutofillServerCommunication,MediaRouter,Translate,SafeBrowsingEnhancedProtection,CertificateTransparencyComponentUpdater".to_string(),
        "--disable-hang-monitor".to_string(),
        "--disable-popup-blocking".to_string(),
        "--disable-prompt-on-repost".to_string(),
        "--disable-renderer-backgrounding".to_string(),
        "--disable-sync".to_string(),
        "--metrics-recording-only".to_string(),
        "--no-pings".to_string(),
        "--safebrowsing-disable-auto-update".to_string(),
        "--disable-search-engine-choice-screen".to_string(),
        "--password-store=basic".to_string(),
        "about:blank".to_string(),
    ];

    let mut cmd = Command::new(chrome_exe);
    cmd.args(&chrome_args)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let child = cmd
        .spawn()
        .map_err(|e| t_with("browser_chrome_spawn_error", &[("error", &e.to_string())]))?;

    dbg_log!("browser: chrome spawned on cdp_port={}", cdp_port);
    Ok(child)
}

async fn pick_free_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("bind: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("local_addr: {e}"))?
        .port();
    Ok(port)
}

fn build_telegram_web_data(
    session: &TelethonSession,
    json: &AccountJson,
    user_id: i64,
    dc_auths: &[WebDcAuth],
    user_info: &UserInfo,
) -> Result<Map<String, Value>, String> {
    let mut map = Map::new();

    let dc = session.dc_id;
    let auth_key_hex = hex::encode(&session.auth_key);
    let fingerprint = auth_key_fingerprint(&auth_key_hex)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| format!("time: {e}"))?
        .as_secs();
    let now_ms = now * 1000;
    let mut rng = rand::thread_rng();
    let instance_id: u32 = rng.gen();
    let push_key: Vec<u8> = (0..256).map(|_| rng.gen::<u8>()).collect();
    let push_key_hex = hex::encode(&push_key);

    let first_name = if user_info.first_name.is_empty() {
        &json.first_name
    } else {
        &user_info.first_name
    };
    let last_name = if user_info.last_name.is_empty() {
        &json.last_name
    } else {
        &user_info.last_name
    };
    let phone = if user_info.phone.is_empty() {
        &json.phone
    } else {
        &user_info.phone
    };
    let username = user_info.username.clone();
    let is_premium = user_info.premium || json.is_premium;

    map.insert("dc".to_string(), json!(dc.to_string()));
    map.insert("auth_key_fingerprint".to_string(), json!(fingerprint));
    for auth in dc_auths {
        map.insert(
            format!("dc{}_auth_key", auth.dc_id),
            json!(auth.auth_key_hex.clone()),
        );
        map.insert(
            format!("dc{}_server_salt", auth.dc_id),
            json!(auth.server_salt_hex.clone()),
        );
    }
    map.insert("number_of_accounts".to_string(), json!("1"));
    map.insert("k_build".to_string(), json!("657"));
    map.insert("kz_version".to_string(), json!("Z"));
    map.insert("tt-multitab_1".to_string(), json!("1"));
    map.insert(
        "tgme_sync".to_string(),
        json!(json!({"canRedirect": true, "ts": now }).to_string()),
    );
    map.insert(
        "user_auth".to_string(),
        json!(json!({"dcID": dc, "id": user_id }).to_string()),
    );
    map.insert(
        "xt_instance".to_string(),
        json!(
            json!({"id": instance_id, "idle": true, "time": now_ms, "accountNumber": 1 })
                .to_string()
        ),
    );
    map.insert("push_key".to_string(), json!(push_key_hex));

    let mut account = Map::new();
    for auth in dc_auths {
        account.insert(
            format!("dc{}_auth_key", auth.dc_id),
            json!(auth.auth_key_hex.clone()),
        );
        account.insert(
            format!("dc{}_server_salt", auth.dc_id),
            json!(auth.server_salt_hex.clone()),
        );
    }
    account.insert("auth_key_fingerprint".to_string(), json!(fingerprint));
    account.insert("userId".to_string(), json!(user_id.to_string()));
    account.insert("dcId".to_string(), json!(dc));
    account.insert("date".to_string(), json!(now));
    account.insert("push_key".to_string(), json!(push_key_hex));
    account.insert("firstName".to_string(), json!(first_name));
    account.insert("lastName".to_string(), json!(last_name));
    account.insert("phone".to_string(), json!(phone));
    account.insert("username".to_string(), json!(username));
    account.insert("isPremium".to_string(), json!(is_premium));
    account.insert("emojiStatusId".to_string(), json!(json.premium_expiry));
    account.insert("avatarUri".to_string(), json!(""));
    map.insert(
        "account1".to_string(),
        json!(Value::Object(account).to_string()),
    );

    Ok(map)
}

async fn collect_web_dc_auths(
    main_client: &mut MtpClient,
    session: &TelethonSession,
    main_server_salt: u64,
    proxy: Option<&ProxyConfig>,
    app_id: i32,
    init_device: &devices::DeviceInfo,
) -> Result<Vec<WebDcAuth>, String> {
    let mut dc_ids: Vec<i32> = crate::get_app_config()
        .dc_addresses
        .keys()
        .copied()
        .collect();
    dc_ids.push(session.dc_id);
    dc_ids.sort_unstable();
    dc_ids.dedup();

    let mut auths = vec![WebDcAuth {
        dc_id: session.dc_id,
        auth_key_hex: hex::encode(&session.auth_key),
        server_salt_hex: hex::encode(main_server_salt.to_le_bytes()),
    }];

    let mut exported = Vec::new();
    for dc_id in dc_ids {
        if dc_id == session.dc_id {
            continue;
        }

        match export_dc_auth(main_client, dc_id).await {
            Ok(auth) => exported.push(auth),
            Err(e) => {
                dbg_log!("browser: dc{} web auth export failed: {}", dc_id, e);
            }
        }
    }

    let import_tasks = exported.into_iter().map(|exported| {
        import_exported_dc_auth(exported, proxy.cloned(), app_id, init_device.clone())
    });
    for result in join_all(import_tasks).await {
        match result {
            Ok(auth) => auths.push(auth),
            Err(e) => dbg_log!("browser: dc web auth import failed: {}", e),
        }
    }

    if auths.is_empty() {
        return Err("browser: no dc auth keys collected".to_string());
    }
    Ok(auths)
}

async fn export_dc_auth(main_client: &mut MtpClient, dc_id: i32) -> Result<ExportedDcAuth, String> {
    let export_req = tl_gen::build_auth_exportAuthorization(dc_id);
    let export_resp = main_client.invoke(&export_req).await?;
    let exported = tl_gen::parse_auth_exportAuthorization(&export_resp)?;

    Ok(ExportedDcAuth {
        dc_id,
        id: exported.id,
        bytes: exported.bytes,
    })
}

async fn import_exported_dc_auth(
    exported: ExportedDcAuth,
    proxy: Option<ProxyConfig>,
    app_id: i32,
    init_device: devices::DeviceInfo,
) -> Result<WebDcAuth, String> {
    let dc_id = exported.dc_id;

    let addr = crate::get_app_config()
        .dc_addresses
        .get(&dc_id)
        .cloned()
        .ok_or_else(|| format!("dc{} address not found", dc_id))?;

    let mut transport = MtpTransport::connect(&addr, proxy.as_ref()).await?;
    let dh = perform_dh(&mut transport).await?;
    let mut target_client =
        MtpClient::from_transport(transport, dh.auth_key, dh.server_salt, &addr);
    target_client.set_proxy(proxy);

    let import_inner = tl_gen::build_auth_importAuthorization(exported.id, &exported.bytes);
    let import_req = tl_gen::wrap_invoke_with_layer(
        &import_inner,
        app_id,
        &init_device.device,
        &init_device.sdk,
        &init_device.app_version,
        "en",
        "en",
    );
    let import_resp = target_client.invoke(&import_req).await?;
    let _ = tl_gen::parse_auth_importAuthorization(&import_resp)?;

    Ok(WebDcAuth {
        dc_id,
        auth_key_hex: hex::encode(target_client.auth_key()),
        server_salt_hex: hex::encode(target_client.server_salt().to_le_bytes()),
    })
}

fn auth_key_fingerprint(auth_key_hex: &str) -> Result<String, String> {
    if auth_key_hex.len() < 8 {
        return Err(format!(
            "invalid auth_key hex length: {}",
            auth_key_hex.len()
        ));
    }
    Ok(auth_key_hex[..8].to_string())
}

fn build_inject_script(data: &Map<String, Value>) -> String {
    let json = Value::Object(data.clone());
    let json_str = json.to_string();
    format!(
        "(()=>{{ const data = {}; try {{ for (const [k, v] of Object.entries(data)) {{ localStorage.setItem(k, v); }} }} catch (e) {{ console.error('browser inject', e); }} }})();",
        json_str
    )
}

async fn connect_cdp(port: u16) -> Result<WebSocketStream<MaybeTlsStream<TcpStream>>, String> {
    let list_url = format!("http://127.0.0.1:{}/json/list", port);
    let mut attempts = 0;
    let ws_url = loop {
        if attempts >= 30 {
            return Err(t("browser_cdp_timeout"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if let Ok(resp) = ureq::get(&list_url).call() {
            if let Ok(json) = resp.into_body().read_json::<Value>() {
                if let Some(first) = json.as_array().and_then(|a| a.first()) {
                    if let Some(url) = first["webSocketDebuggerUrl"].as_str() {
                        break url.to_string();
                    }
                }
            }
        }
        attempts += 1;
    };

    let (ws, _) = connect_async(&ws_url)
        .await
        .map_err(|e| t_with("browser_cdp_connect_error", &[("error", &e.to_string())]))?;
    Ok(ws)
}

async fn inject_and_navigate(
    mut ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    script: &str,
    target_url: &str,
) -> Result<(), String> {
    send_cdp_command(&mut ws, 1, "Runtime.enable", None).await?;
    send_cdp_command(&mut ws, 2, "Page.enable", None).await?;
    send_cdp_command(
        &mut ws,
        3,
        "Page.addScriptToEvaluateOnNewDocument",
        Some(json!({ "source": script })),
    )
    .await?;
    send_cdp_command(
        &mut ws,
        4,
        "Page.navigate",
        Some(json!({ "url": target_url })),
    )
    .await?;

    Ok(())
}

async fn send_cdp_command(
    ws: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    id: u64,
    method: &str,
    params: Option<Value>,
) -> Result<(), String> {
    let mut cmd = json!({ "id": id, "method": method });
    if let Some(params) = params {
        cmd["params"] = params;
    }
    ws.send(Message::Text(cmd.to_string()))
        .await
        .map_err(|e| t_with("browser_cdp_send_error", &[("error", &e.to_string())]))?;

    let deadline = std::time::Duration::from_secs(8);
    loop {
        let next = tokio::time::timeout(deadline, ws.next())
            .await
            .map_err(|_| t_with("browser_cdp_send_error", &[("error", "response timeout")]))?;
        let Some(msg) = next else {
            return Err(t_with(
                "browser_cdp_send_error",
                &[("error", "websocket closed")],
            ));
        };
        let msg =
            msg.map_err(|e| t_with("browser_cdp_send_error", &[("error", &e.to_string())]))?;
        let Message::Text(text) = msg else {
            continue;
        };
        let value: Value = serde_json::from_str(&text)
            .map_err(|e| t_with("browser_cdp_send_error", &[("error", &e.to_string())]))?;
        if value.get("id").and_then(|v| v.as_u64()) != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(t_with(
                "browser_cdp_send_error",
                &[("error", &error.to_string())],
            ));
        }
        return Ok(());
    }
}

async fn start_local_proxy(
    config: Option<&ProxyConfig>,
) -> Result<(u16, JoinHandle<()>, oneshot::Sender<()>), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| t_with("browser_proxy_bind_error", &[("error", &e.to_string())]))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("local_addr: {e}"))?
        .port();
    let (tx, rx) = oneshot::channel();
    let cfg = config.cloned();
    let handle = tokio::spawn(run_proxy(listener, cfg, rx));
    Ok((port, handle, tx))
}

fn parse_proxy_string(proxy: Option<&str>) -> Result<Option<ProxyConfig>, String> {
    let Some(s) = proxy else {
        return Ok(None);
    };
    let cfg = ProxyConfig::from_string(s)?;
    match cfg.proxy_type {
        crate::proxy::ProxyType::Socks4 => {
            Err(t_with("browser_proxy_unsupported", &[("scheme", "socks4")]))
        }
        _ => Ok(Some(cfg)),
    }
}

async fn run_proxy(
    listener: TcpListener,
    config: Option<ProxyConfig>,
    mut stop: oneshot::Receiver<()>,
) {
    loop {
        let (client, _) = tokio::select! {
            res = listener.accept() => match res {
                Ok(v) => v,
                Err(_) => break,
            },
            _ = &mut stop => break,
        };
        let cfg = config.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_proxy_client(client, cfg).await {
                dbg_log!("browser proxy client error: {}", e);
            }
        });
    }
}

async fn handle_proxy_client(
    mut client: TcpStream,
    config: Option<ProxyConfig>,
) -> Result<(), String> {
    let mut buf = [0u8; 4096];
    let mut header_len = 0usize;
    loop {
        let n = client
            .read(&mut buf[header_len..])
            .await
            .map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            return Err("client closed".to_string());
        }
        header_len += n;
        if buf[..header_len].windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if header_len >= buf.len() {
            return Err("request too large".to_string());
        }
    }

    let mut headers = [httparse::EMPTY_HEADER; 16];
    let mut req = httparse::Request::new(&mut headers);
    let status = req
        .parse(&buf[..header_len])
        .map_err(|e| format!("parse request: {e}"))?;
    let _body_offset = status.unwrap();

    let method = req.method.unwrap_or("");
    let path = req.path.unwrap_or("");

    if !method.eq_ignore_ascii_case("CONNECT") {
        client
            .write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
            .await
            .ok();
        return Err("only CONNECT method is supported".to_string());
    }

    let target = path;
    dbg_log!("browser proxy CONNECT {}", target);
    let upstream = connect_through_upstream(config.as_ref(), target).await?;
    client
        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
        .await
        .map_err(|e| format!("write 200: {e}"))?;
    let (mut client_read, mut client_write) = client.into_split();
    let (mut upstream_read, mut upstream_write) = upstream.into_split();
    let c2u = tokio::io::copy(&mut client_read, &mut upstream_write);
    let u2c = tokio::io::copy(&mut upstream_read, &mut client_write);
    tokio::select! {
        _ = c2u => {},
        _ = u2c => {},
    }
    Ok(())
}

fn parse_host_port(addr: &str) -> Result<(&str, u16), String> {
    let colon = addr.rfind(':').ok_or_else(|| "missing port".to_string())?;
    let host = &addr[..colon];
    let port = addr[colon + 1..]
        .parse::<u16>()
        .map_err(|_| "invalid port".to_string())?;
    Ok((host, port))
}

async fn connect_through_upstream(
    config: Option<&ProxyConfig>,
    target: &str,
) -> Result<TcpStream, String> {
    let Some(cfg) = config else {
        return TcpStream::connect(target)
            .await
            .map_err(|e| format!("direct connect to {}: {}", target, e));
    };

    let (host, port) =
        parse_host_port(target).map_err(|e| format!("invalid target {}: {}", target, e))?;

    match cfg.proxy_type {
        crate::proxy::ProxyType::Socks5 | crate::proxy::ProxyType::Socks4 => {
            crate::proxy::connect_via_proxy(cfg, host, port)
                .await
                .map_err(|e| format!("proxy connect to {}: {}", target, e))
        }
        crate::proxy::ProxyType::Https => {
            let mut upstream = TcpStream::connect(format!("{}:{}", cfg.host, cfg.port))
                .await
                .map_err(|e| format!("proxy connect to {}: {}", cfg.host, e))?;

            let mut connect_req = format!("CONNECT {} HTTP/1.1\r\nHost: {}\r\n", target, target);
            if let (Some(u), Some(p)) = (&cfg.username, &cfg.password) {
                let creds =
                    base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", u, p));
                connect_req.push_str(&format!("Proxy-Authorization: Basic {}\r\n", creds));
            }
            connect_req.push_str("\r\n");
            upstream
                .write_all(connect_req.as_bytes())
                .await
                .map_err(|e| format!("proxy connect write: {e}"))?;

            let mut resp_buf = [0u8; 1024];
            let n = upstream
                .read(&mut resp_buf)
                .await
                .map_err(|e| format!("proxy connect read: {e}"))?;
            if n == 0 {
                return Err("proxy closed".to_string());
            }
            let resp = std::str::from_utf8(&resp_buf[..n])
                .map_err(|_| "proxy non-utf8 response".to_string())?;
            if !resp.starts_with("HTTP/1.1 200") && !resp.starts_with("HTTP/1.0 200") {
                return Err(format!(
                    "proxy connect failed: {}",
                    resp.lines().next().unwrap_or("")
                ));
            }
            Ok(upstream)
        }
    }
}
