use crate::accounts::auth_login::save_authorized_account;
use crate::accounts::devices::DeviceInfo;
use crate::mtproto::auth::perform_dh;
use crate::mtproto::client::MtpClient;
use crate::mtproto::tl::{self, AuthCodeSettings, AuthorizedUser, SentCode};
use crate::mtproto::transport::MtpTransport;
use crate::proxy::ProxyConfig;
use crate::registrar::profile::RegistrationProfile;
use serde::{Deserialize, Serialize};

pub struct RegistrationSession {
    client: MtpClient,
    dc_id: i32,
    phone: String,
    phone_code_hash: String,
    profile: RegistrationProfile,
    proxy: Option<ProxyConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RegistrationCodeRequest {
    pub phone: String,
    pub profile: RegistrationProfile,
    pub proxy: Option<ProxyConfig>,
    pub code_settings: AuthCodeSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationIdentity {
    pub first_name: String,
    pub last_name: String,
}

impl RegistrationIdentity {
    pub fn validate(&self) -> Result<(), String> {
        if self.first_name.trim().is_empty()
            || self.first_name.len() > 64
            || self.first_name.contains(['\r', '\n', '\0'])
            || self.last_name.len() > 64
            || self.last_name.contains(['\r', '\n', '\0'])
        {
            return Err("invalid registration name".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RegistrationOutcome {
    Registered { account_id: String },
    AlreadyRegistered,
}

impl RegistrationSession {
    pub async fn start(request: RegistrationCodeRequest) -> Result<(Self, SentCode), String> {
        let phone = normalize_phone(&request.phone)?;
        request.profile.validate()?;
        let mut dc_id = 2;
        let mut last_error = String::new();

        for _ in 0..=4 {
            match start_on_dc(dc_id, &phone, &request.profile, request.proxy.as_ref(), &request.code_settings).await {
                Ok((client, sent_code)) => {
                    return Ok((
                        Self {
                            client,
                            dc_id,
                            phone,
                            phone_code_hash: sent_code.phone_code_hash.clone(),
                            profile: request.profile,
                            proxy: request.proxy,
                        },
                        sent_code,
                    ));
                }
                Err(error) => {
                    if let Some(migrated_dc) = phone_migrate_dc(&error) {
                        dc_id = migrated_dc;
                        continue;
                    }
                    last_error = error;
                    break;
                }
            }
        }
        Err(last_error)
    }

    pub async fn complete(
        &mut self,
        code: &str,
        identity: &RegistrationIdentity,
    ) -> Result<RegistrationOutcome, String> {
        validate_code(code)?;
        identity.validate()?;
        let sign_in = tl::build_auth_sign_in(&self.phone, &self.phone_code_hash, code);
        match self.client.invoke(&sign_in).await {
            Ok(response) => match tl::parse_auth_authorization(&response) {
                Ok(_) => Ok(RegistrationOutcome::AlreadyRegistered),
                Err(error) if is_unoccupied(&error) => self.sign_up(identity).await,
                Err(error) => Err(error),
            },
            Err(error) if is_unoccupied(&error) => self.sign_up(identity).await,
            Err(error) => Err(error),
        }
    }

    pub async fn cancel(&mut self) -> Result<(), String> {
        let request = tl::build_auth_cancel_code(&self.phone, &self.phone_code_hash);
        self.client.invoke(&request).await.map(|_| ())
    }

    pub async fn resend(&mut self) -> Result<SentCode, String> {
        let request = tl::build_auth_resend_code(&self.phone, &self.phone_code_hash);
        let response = self.client.invoke(&request).await?;
        let sent_code = tl::parse_auth_sent_code(&response)?;
        self.phone_code_hash = sent_code.phone_code_hash.clone();
        Ok(sent_code)
    }

    async fn sign_up(
        &mut self,
        identity: &RegistrationIdentity,
    ) -> Result<RegistrationOutcome, String> {
        let request = tl::build_auth_sign_up(
            &self.phone,
            &self.phone_code_hash,
            &identity.first_name,
            &identity.last_name,
        );
        let response = self.client.invoke(&request).await?;
        let user = tl::parse_auth_authorization(&response)?;
        self.save(&user)
            .map(|account_id| RegistrationOutcome::Registered { account_id })
    }

    fn save(&self, user: &AuthorizedUser) -> Result<String, String> {
        let device = DeviceInfo {
            device: self.profile.device_model.clone(),
            sdk: self.profile.system_version.clone(),
            app_version: self.profile.app_version.clone(),
        };
        save_authorized_account(
            user,
            self.dc_id,
            &self.phone,
            self.proxy.as_ref(),
            &device,
            self.profile.api_id,
            &self.profile.api_hash,
            self.client.auth_key(),
            "",
        )
    }
}

async fn start_on_dc(
    dc_id: i32,
    phone: &str,
    profile: &RegistrationProfile,
    proxy: Option<&ProxyConfig>,
    settings: &AuthCodeSettings,
) -> Result<(MtpClient, SentCode), String> {
    let address = dc_address(dc_id)?;
    let mut transport = MtpTransport::connect(address, proxy).await?;
    let dh = perform_dh(&mut transport).await.map_err(|error| error.to_string())?;
    let mut client = MtpClient::from_transport(transport, dh.auth_key, dh.server_salt, address);
    let inner = tl::build_auth_send_code_with_settings(phone, profile.api_id, &profile.api_hash, settings);
    let request = tl::wrap_init_connection(
        &inner,
        profile.api_id,
        &profile.device_model,
        &profile.system_version,
        &profile.app_version,
        &profile.system_lang_code,
        &profile.lang_code,
    );
    let response = client.invoke(&request).await?;
    let sent_code = tl::parse_auth_sent_code(&response)?;
    Ok((client, sent_code))
}

fn dc_address(dc_id: i32) -> Result<&'static str, String> {
    match dc_id {
        1 => Ok("149.154.175.53:443"),
        2 => Ok("149.154.167.51:443"),
        3 => Ok("149.154.175.100:443"),
        4 => Ok("149.154.167.91:443"),
        5 => Ok("91.108.56.130:443"),
        _ => Err(format!("unsupported Telegram data center: {dc_id}")),
    }
}

fn phone_migrate_dc(error: &str) -> Option<i32> {
    let marker = "PHONE_MIGRATE_";
    let start = error.find(marker)? + marker.len();
    let dc_id: i32 = error[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()?;
    (1..=5).contains(&dc_id).then_some(dc_id)
}

fn normalize_phone(phone: &str) -> Result<String, String> {
    let phone: String = phone.chars().filter(char::is_ascii_digit).collect();
    if !(5..=15).contains(&phone.len()) {
        return Err("phone number must contain 5 to 15 digits".to_string());
    }
    Ok(phone)
}

fn validate_code(code: &str) -> Result<(), String> {
    if code.is_empty() || code.len() > 32 || !code.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err("verification code must contain up to 32 letters or digits".to_string());
    }
    Ok(())
}

fn is_unoccupied(error: &str) -> bool {
    error.contains("PHONE_NUMBER_UNOCCUPIED")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_phone_migration_target() {
        assert_eq!(phone_migrate_dc("rpc error 303: PHONE_MIGRATE_4"), Some(4));
        assert_eq!(phone_migrate_dc("PHONE_MIGRATE_9"), None);
    }

    #[test]
    fn validates_phone_and_code() {
        assert_eq!(normalize_phone("+1 (202) 555-0100").unwrap(), "12025550100");
        assert!(validate_code("12 34").is_err());
    }
}
