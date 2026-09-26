use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationProfile {
    pub api_id: i32,
    pub api_hash: String,
    pub device_model: String,
    pub system_version: String,
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
        if self.api_hash.len() > 128 || !self.api_hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("API hash must be a hexadecimal value no longer than 128 characters".to_string());
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
            || self.timezone_offsets_hours.iter().any(|offset| !(-12..=14).contains(offset))
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
        .map(String::trim)
        .filter(|value| !value.is_empty() && value.len() <= 128 && !value.contains(['\r', '\n', '\0']))
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
}
