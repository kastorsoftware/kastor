use rand::Rng;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::time::Duration;

const TELEGRAM_ANDROID_TAGS_URL: &str =
    "https://api.github.com/repos/DrKLO/Telegram/tags?per_page=100";

// Model names and SDKs are paired so that the fingerprint remains internally
// consistent. These models launched with Android 13 or later.
const ANDROID_REGISTRATION_DEVICES: &[(&str, &str)] = &[
    ("Google Pixel 7", "SDK 33"),
    ("Google Pixel 7a", "SDK 33"),
    ("Google Pixel 8", "SDK 34"),
    ("Google Pixel 8a", "SDK 34"),
    ("Google Pixel 9", "SDK 35"),
    ("Google Pixel 9 Pro", "SDK 35"),
    ("Samsung Galaxy A15 5G", "SDK 34"),
    ("Samsung Galaxy A25 5G", "SDK 34"),
    ("Samsung Galaxy A35 5G", "SDK 34"),
    ("Samsung Galaxy A55 5G", "SDK 34"),
    ("Samsung Galaxy S23", "SDK 33"),
    ("Samsung Galaxy S23 Ultra", "SDK 33"),
    ("Samsung Galaxy S24", "SDK 34"),
    ("Samsung Galaxy S24 Ultra", "SDK 34"),
    ("Xiaomi 13T", "SDK 33"),
    ("Xiaomi 14", "SDK 34"),
    ("Xiaomi Redmi Note 13", "SDK 33"),
    ("Xiaomi Redmi Note 13 Pro 5G", "SDK 33"),
    ("Xiaomi Poco X6 Pro", "SDK 34"),
    ("Xiaomi Poco F6", "SDK 34"),
    ("OnePlus 11", "SDK 33"),
    ("OnePlus 12", "SDK 34"),
    ("OnePlus Nord CE 4", "SDK 34"),
    ("Nothing Phone (2)", "SDK 33"),
    ("Nothing Phone (2a)", "SDK 34"),
    ("Motorola Edge 40", "SDK 33"),
    ("Motorola Edge 50 Pro", "SDK 34"),
    ("Motorola Moto G84 5G", "SDK 33"),
];

#[derive(Deserialize)]
struct GithubTag {
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationProfile {
    pub api_id: i32,
    pub api_hash: String,
    #[serde(default)]
    pub device_model: String,
    #[serde(default)]
    pub system_version: String,
    #[serde(default)]
    pub app_version: String,
    pub system_lang_code: String,
    pub lang_code: String,
    pub timezone_offset_seconds: i32,
}

impl RegistrationProfile {
    pub fn validate(&self) -> Result<(), String> {
        if self.api_id <= 0 || self.api_hash.trim().is_empty() {
            return Err("API ID and API hash are required".to_string());
        }
        if self.api_hash.len() > 128 || !self.api_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(
                "API hash must be a hexadecimal value no longer than 128 characters".to_string(),
            );
        }
        for (name, value) in [
            ("device model", &self.device_model),
            ("system version", &self.system_version),
            ("app version", &self.app_version),
            ("system language", &self.system_lang_code),
            ("app language", &self.lang_code),
        ] {
            if value.trim().is_empty() || value.len() > 128 || value.contains(['\r', '\n', '\0']) {
                return Err(format!("invalid {name}"));
            }
        }
        if !(-43_200..=50_400).contains(&self.timezone_offset_seconds) {
            return Err("timezone offset must be between -43200 and 50400 seconds".to_string());
        }
        Ok(())
    }

    pub fn validate_request(&self) -> Result<(), String> {
        if self.api_id <= 0 || self.api_hash.trim().is_empty() {
            return Err("API ID and API hash are required".to_string());
        }
        if self.api_hash.len() > 128 || !self.api_hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(
                "API hash must be a hexadecimal value no longer than 128 characters".to_string(),
            );
        }
        for (name, value) in [
            ("system language", &self.system_lang_code),
            ("app language", &self.lang_code),
        ] {
            if value.trim().is_empty() || value.len() > 128 || value.contains(['\r', '\n', '\0']) {
                return Err(format!("invalid {name}"));
            }
        }
        if !(-43_200..=50_400).contains(&self.timezone_offset_seconds) {
            return Err("timezone offset must be between -43200 and 50400 seconds".to_string());
        }
        Ok(())
    }

    pub fn assign_random_android_client(&mut self, app_versions: &[String]) -> Result<(), String> {
        if app_versions.len() != 3 || app_versions.iter().any(|version| version.is_empty()) {
            return Err("exactly three Telegram Android versions are required".to_string());
        }
        let mut rng = rand::thread_rng();
        let (device_model, system_version) =
            ANDROID_REGISTRATION_DEVICES[rng.gen_range(0..ANDROID_REGISTRATION_DEVICES.len())];
        self.device_model = device_model.to_string();
        self.system_version = system_version.to_string();
        self.app_version = app_versions[rng.gen_range(0..app_versions.len())].clone();
        self.validate()
    }
}

pub fn fetch_latest_android_versions() -> Result<Vec<String>, String> {
    let agent = ureq::Agent::config_builder()
        .https_only(true)
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .new_agent();
    let response = agent
        .get(TELEGRAM_ANDROID_TAGS_URL)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "Kastor registrar")
        .call()
        .map_err(|_| "could not fetch Telegram Android releases from GitHub")?;
    let tags: Vec<GithubTag> = response
        .into_body()
        .read_json()
        .map_err(|_| "could not parse Telegram Android releases from GitHub")?;
    latest_android_versions_from_tags(tags.iter().map(|tag| tag.name.as_str()))
}

fn latest_android_versions_from_tags<'a>(
    tags: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, String> {
    let mut versions: Vec<([u32; 3], String)> = tags
        .into_iter()
        .filter_map(parse_android_release_tag)
        .collect();
    versions.sort_unstable_by_key(|(numbers, _)| Reverse(*numbers));
    versions.dedup_by(|left, right| left.0 == right.0);
    let versions: Vec<_> = versions
        .into_iter()
        .take(3)
        .map(|(_, version)| version)
        .collect();
    if versions.len() != 3 {
        return Err("GitHub did not return three Telegram Android release tags".to_string());
    }
    Ok(versions)
}

fn parse_android_release_tag(tag: &str) -> Option<([u32; 3], String)> {
    let (version, build) = tag.strip_prefix("release-")?.split_once('_')?;
    if build.is_empty() || !build.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut parts = version.split('.');
    let numbers = [
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ];
    if parts.next().is_some() {
        return None;
    }
    Some((numbers, version.to_string()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileGeneratorConfig {
    pub api_id: i32,
    pub api_hash: String,
    pub device_models: Vec<String>,
    pub system_versions: Vec<String>,
    pub app_versions: Vec<String>,
    pub system_languages: Vec<String>,
    pub app_languages: Vec<String>,
    pub timezone_offsets_hours: Vec<i32>,
}

impl ProfileGeneratorConfig {
    pub fn generate(&self, count: usize) -> Result<Vec<RegistrationProfile>, String> {
        if count == 0 || count > 100_000 {
            return Err("profile count must be between 1 and 100000".to_string());
        }
        let device_models = non_empty_values(&self.device_models, "device models")?;
        let system_versions = non_empty_values(&self.system_versions, "system versions")?;
        let app_versions = non_empty_values(&self.app_versions, "app versions")?;
        let system_languages = non_empty_values(&self.system_languages, "system languages")?;
        let app_languages = non_empty_values(&self.app_languages, "app languages")?;
        if self.timezone_offsets_hours.is_empty()
            || self
                .timezone_offsets_hours
                .iter()
                .any(|offset| !(-12..=14).contains(offset))
        {
            return Err("timezone offsets must be between -12 and 14 hours".to_string());
        }

        let mut rng = rand::thread_rng();
        let mut profiles = Vec::with_capacity(count);
        for _ in 0..count {
            let profile = RegistrationProfile {
                api_id: self.api_id,
                api_hash: self.api_hash.clone(),
                device_model: choose(&device_models, &mut rng).to_string(),
                system_version: choose(&system_versions, &mut rng).to_string(),
                app_version: choose(&app_versions, &mut rng).to_string(),
                system_lang_code: choose(&system_languages, &mut rng).to_string(),
                lang_code: choose(&app_languages, &mut rng).to_string(),
                timezone_offset_seconds: choose(&self.timezone_offsets_hours, &mut rng) * 3600,
            };
            profile.validate()?;
            profiles.push(profile);
        }
        Ok(profiles)
    }
}

fn non_empty_values<'a>(values: &'a [String], name: &str) -> Result<Vec<&'a str>, String> {
    let values: Vec<_> = values
        .iter()
        .map(|value| value.trim())
        .filter(|value| {
            !value.is_empty() && value.len() <= 128 && !value.contains(['\r', '\n', '\0'])
        })
        .collect();
    if values.is_empty() {
        Err(format!("{name} must contain at least one valid value"))
    } else {
        Ok(values)
    }
}

fn choose<'a, T>(values: &'a [T], rng: &mut impl Rng) -> &'a T {
    &values[rng.gen_range(0..values.len())]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generator() -> ProfileGeneratorConfig {
        ProfileGeneratorConfig {
            api_id: 1,
            api_hash: "0123456789abcdef0123456789abcdef".to_string(),
            device_models: vec!["Pixel 8".to_string()],
            system_versions: vec!["SDK 34".to_string()],
            app_versions: vec!["11.0.0".to_string()],
            system_languages: vec!["en-US".to_string()],
            app_languages: vec!["en".to_string()],
            timezone_offsets_hours: vec![3],
        }
    }

    #[test]
    fn generates_valid_profiles() {
        let profiles = generator().generate(2).unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].timezone_offset_seconds, 10_800);
    }

    #[test]
    fn rejects_invalid_hash() {
        let mut config = generator();
        config.api_hash = "not-a-hash".to_string();
        assert!(config.generate(1).is_err());
    }

    #[test]
    fn extracts_three_latest_android_versions_from_release_tags() {
        let versions = latest_android_versions_from_tags([
            "release-12.9.1_6976",
            "release-12.10.0_7031",
            "release-12.9.2_6991",
            "release-12.6.4_6666",
            "release-12.10.0_beta",
            "desktop-6.0.0_1",
        ])
        .unwrap();
        assert_eq!(versions, ["12.10.0", "12.9.2", "12.9.1"]);
    }

    #[test]
    fn android_assignment_replaces_client_values_from_request() {
        let mut profile = RegistrationProfile {
            api_id: 1,
            api_hash: "0123456789abcdef0123456789abcdef".to_string(),
            device_model: "request value".to_string(),
            system_version: "request value".to_string(),
            app_version: "request value".to_string(),
            system_lang_code: "en-US".to_string(),
            lang_code: "en".to_string(),
            timezone_offset_seconds: 0,
        };
        let versions = ["12.10.0", "12.9.2", "12.9.1"].map(str::to_string);
        profile.assign_random_android_client(&versions).unwrap();
        assert!(versions.contains(&profile.app_version));
        assert_ne!(profile.device_model, "request value");
        assert_ne!(profile.system_version, "request value");
    }
}
