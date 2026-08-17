use crate::accounts::commands::get_storage_pub;
use crate::accounts::devices;
use crate::accounts::session::{AccountJson, TelethonSession};
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl;
use crate::proxy;

pub async fn connect_account_with_info(account_id: &str) -> Result<(MtpClient, tl::UserInfo, u64), String> {
    let storage = get_storage_pub();
    let session_path = storage.session_path(account_id);
    let json_path = storage.json_path(account_id);
    let session = TelethonSession::from_file(&session_path)
        .map_err(|e| crate::i18n::t_with("connect_session_error", &[("error", &e)]))?;
    let json = if json_path.exists() {
        AccountJson::from_file(&json_path).unwrap_or_default()
    } else {
        AccountJson::default()
    };
    if session.auth_key.len() != 256 {
        return Err(crate::i18n::t("connect_invalid_authkey"));
    }
    let mut key = [0u8; 256];
    key.copy_from_slice(&session.auth_key);
    let addr = format!("{}:{}", session.server_address, session.port);
    let proxy = proxy::select_proxy_for_account(json.proxy.as_deref()).ok().flatten();

    let mut client = {
        let mut last_err = String::new();
        let mut connected = None;
        for attempt in 0..5 {
            match MtpClient::connect(&addr, &key, proxy.as_ref()).await {
                Ok(c) => { connected = Some(c); break; }
                Err(e) => {
                    last_err = e;
                    if attempt < 4 {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        }
        connected.ok_or_else(|| format!("connect (5 attempts): {last_err}"))?
    };

    let dev = if !json.device.is_empty() && !json.sdk.is_empty() {
        devices::DeviceInfo { device: json.device.clone(), sdk: json.sdk.clone(), app_version: json.app_version.clone() }
    } else {
        devices::generate_random_device()
    };
    let app_id = if json.app_id == 0 { crate::get_app_config().app_id } else { json.app_id };
    let get_me = tl::build_get_me_request(app_id, &dev.device, &dev.sdk, &dev.app_version, "en", "en");
    let resp = client.invoke(&get_me).await.map_err(|e| format!("init: {e}"))?;
    let user_info = crate::mtproto::tl::parse_users_response(&resp)
        .map_err(|e| format!("init parse: {e}"))?;
    let server_salt = client.server_salt();
    Ok((client, user_info, server_salt))
}

pub async fn connect_account(account_id: &str) -> Result<MtpClient, String> {
    let (client, _, _) = connect_account_with_info(account_id).await?;
    Ok(client)
}
