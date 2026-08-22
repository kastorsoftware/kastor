use serde::Serialize;

fn default_account_threads() -> u32 { 5 }
fn default_proxy_threads() -> u32 { 10 }
fn default_checker_threads() -> u32 { 5 }
fn default_converter_threads() -> u32 { 10 }
fn default_checker_channels_min() -> u32 { 100 }
fn default_checker_groups_min() -> u32 { 100 }
fn default_reauth_threads() -> u32 { 5 }
fn default_account_actions_threads() -> u32 { 5 }
fn default_llm_api_type() -> String { "openai".to_string() }

#[derive(Serialize, serde::Deserialize, Clone)]
pub struct AppSettings {
    #[serde(default)]
    pub allow_no_proxy: bool,
    #[serde(default = "default_account_threads")]
    pub account_threads: u32,
    #[serde(default = "default_proxy_threads")]
    pub proxy_threads: u32,
    #[serde(default = "default_checker_threads")]
    pub checker_threads: u32,
    #[serde(default = "default_converter_threads")]
    pub converter_threads: u32,
    #[serde(default = "default_checker_channels_min")]
    pub checker_channels_min: u32,
    #[serde(default = "default_checker_groups_min")]
    pub checker_groups_min: u32,
    #[serde(default = "default_reauth_threads")]
    pub reauth_threads: u32,
    #[serde(default = "default_account_actions_threads")]
    pub account_actions_threads: u32,
    #[serde(default)]
    pub llm_api_url: String,
    #[serde(default)]
    pub llm_token: String,
    #[serde(default)]
    pub llm_model: String,
    #[serde(default = "default_llm_api_type")]
    pub llm_api_type: String,
    #[serde(default)]
    pub validate_check_2fa: bool,
    #[serde(default)]
    pub validate_check_aging: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            allow_no_proxy: false,
            account_threads: 5,
            proxy_threads: 10,
            checker_threads: 5,
            converter_threads: 10,
            checker_channels_min: 100,
            checker_groups_min: 100,
            reauth_threads: 5,
            account_actions_threads: 5,
            llm_api_url: String::new(),
            llm_token: String::new(),
            llm_model: String::new(),
            llm_api_type: "openai".to_string(),
            validate_check_2fa: false,
            validate_check_aging: false,
        }
    }
}

impl AppSettings {
    fn path() -> std::path::PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("kastor")
            .join("settings.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        if path.exists() {
            std::fs::read_to_string(&path).ok()
                .and_then(|c| serde_json::from_str(&c).ok())
                .unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let content = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, content).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn get_settings() -> AppSettings {
    AppSettings::load()
}

#[tauri::command]
pub fn save_settings(settings: AppSettings) -> Result<(), String> {
    settings.save()
}

fn patch_u32(patch: &serde_json::Value, key: &str, field: &mut u32) -> Result<(), String> {
    let Some(value) = patch.get(key) else { return Ok(()); };
    let value = value.as_u64().ok_or_else(|| format!("{key} must be an unsigned integer"))?;
    *field = u32::try_from(value).map_err(|_| format!("{key} exceeds u32 range"))?;
    Ok(())
}

// partial update: load existing, merge provided fields, save
#[tauri::command]
pub fn patch_settings(patch: serde_json::Value) -> Result<(), String> {
    let mut current = AppSettings::load();
    if let Some(v) = patch.get("allow_no_proxy").and_then(|v| v.as_bool()) { current.allow_no_proxy = v; }
    patch_u32(&patch, "account_threads", &mut current.account_threads)?;
    patch_u32(&patch, "proxy_threads", &mut current.proxy_threads)?;
    patch_u32(&patch, "checker_threads", &mut current.checker_threads)?;
    patch_u32(&patch, "converter_threads", &mut current.converter_threads)?;
    patch_u32(&patch, "checker_channels_min", &mut current.checker_channels_min)?;
    patch_u32(&patch, "checker_groups_min", &mut current.checker_groups_min)?;
    patch_u32(&patch, "reauth_threads", &mut current.reauth_threads)?;
    patch_u32(&patch, "account_actions_threads", &mut current.account_actions_threads)?;
    if let Some(v) = patch.get("llm_api_url").and_then(|v| v.as_str()) { current.llm_api_url = v.to_string(); }
    if let Some(v) = patch.get("llm_token").and_then(|v| v.as_str()) { current.llm_token = v.to_string(); }
    if let Some(v) = patch.get("llm_model").and_then(|v| v.as_str()) { current.llm_model = v.to_string(); }
    if let Some(v) = patch.get("llm_api_type").and_then(|v| v.as_str()) { current.llm_api_type = v.to_string(); }
    if let Some(v) = patch.get("validate_check_2fa").and_then(|v| v.as_bool()) { current.validate_check_2fa = v; }
    if let Some(v) = patch.get("validate_check_aging").and_then(|v| v.as_bool()) { current.validate_check_aging = v; }
    current.save()
}
