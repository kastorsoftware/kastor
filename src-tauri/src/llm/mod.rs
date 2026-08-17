// llm: OpenAI-compatible API client for text generation.
// requests are proxied through a random proxy from the pool.

use std::time::Duration;
use serde::{Deserialize, Serialize};

use crate::proxy::{ProxyConfig, ProxyList, ProxyType};
use crate::settings::AppSettings;

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize, Serialize, Clone)]
pub struct ModelEntry {
    pub id: String,
}

// send a prompt to the configured LLM and return the assistant's reply
pub fn complete(system_prompt: &str, user_message: &str) -> Result<String, String> {
    let settings = AppSettings::load();
    if settings.llm_api_url.is_empty() || settings.llm_token.is_empty() || settings.llm_model.is_empty() {
        return Err(crate::i18n::t("llm_not_configured"));
    }

    let agent = build_agent()?;

    match settings.llm_api_type.as_str() {
        "claude" => complete_claude(&agent, &settings, system_prompt, user_message),
        _ => complete_openai(&agent, &settings, system_prompt, user_message),
    }
}

fn complete_openai(agent: &ureq::Agent, settings: &AppSettings, system_prompt: &str, user_message: &str) -> Result<String, String> {
    let messages = vec![
        ChatMessage { role: "system".to_string(), content: system_prompt.to_string() },
        ChatMessage { role: "user".to_string(), content: user_message.to_string() },
    ];
    let body = ChatRequest { model: settings.llm_model.clone(), messages };
    let url = format!("{}/chat/completions", settings.llm_api_url.trim_end_matches('/'));

    let resp = agent.post(&url)
        .header("Authorization", &format!("Bearer {}", settings.llm_token))
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| format!("LLM request failed: {e}"))?;

    let parsed: ChatResponse = resp.into_body()
        .read_json()
        .map_err(|e| format!("LLM response parse error: {e}"))?;

    parsed.choices.first()
        .map(|c| c.message.content.clone())
        .ok_or_else(|| "LLM returned empty choices".into())
}

fn complete_claude(agent: &ureq::Agent, settings: &AppSettings, system_prompt: &str, user_message: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": settings.llm_model,
        "max_tokens": 4096,
        "system": system_prompt,
        "messages": [{ "role": "user", "content": user_message }]
    });
    let url = format!("{}/messages", settings.llm_api_url.trim_end_matches('/'));

    let resp = agent.post(&url)
        .header("x-api-key", &settings.llm_token)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .send_json(&body)
        .map_err(|e| format!("Claude request failed: {e}"))?;

    let parsed: serde_json::Value = resp.into_body()
        .read_json()
        .map_err(|e| format!("Claude response parse error: {e}"))?;

    parsed["content"].as_array()
        .and_then(|arr| arr.first())
        .and_then(|block| block["text"].as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "Claude returned empty content".into())
}

// fetch available models from the API
#[tauri::command]
pub async fn llm_get_models() -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(llm_get_models_sync)
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
}

fn llm_get_models_sync() -> Result<Vec<String>, String> {
    let settings = AppSettings::load();
    if settings.llm_api_url.is_empty() || settings.llm_token.is_empty() {
        return Err(crate::i18n::t("llm_specify_url_token"));
    }

    let agent = build_agent()?;
    let url = format!("{}/models", settings.llm_api_url.trim_end_matches('/'));

    let resp = agent
        .get(&url)
        .header("Authorization", &format!("Bearer {}", settings.llm_token))
        .header("x-api-key", &settings.llm_token)
        .header("anthropic-version", "2023-06-01")
        .call()
        .map_err(|e| format!("request failed: {e}"))?;

    let parsed: ModelsResponse = resp.into_body()
        .read_json()
        .map_err(|e| format!("parse error: {e}"))?;

    let mut ids: Vec<String> = parsed.data.into_iter().map(|m| m.id).collect();
    ids.sort();
    Ok(ids)
}

// detect API type by probing endpoints (no model-specific requests)
#[tauri::command]
pub async fn llm_detect_api_type() -> Result<String, String> {
    tokio::task::spawn_blocking(llm_detect_api_type_sync)
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
}

fn llm_detect_api_type_sync() -> Result<String, String> {
    let settings = AppSettings::load();
    if settings.llm_api_url.is_empty() || settings.llm_token.is_empty() {
        return Err(crate::i18n::t("llm_specify_url_token"));
    }

    let agent = build_agent()?;
    let base = settings.llm_api_url.trim_end_matches('/');

    // openai exposes GET /models; claude exposes GET /models with anthropic-version header
    // but the key difference: openai uses "Authorization: Bearer" while claude uses "x-api-key"

    // try openai-style: GET /models with Bearer token
    let openai_url = format!("{base}/models");
    if let Ok(mut resp) = agent.get(&openai_url)
        .header("Authorization", &format!("Bearer {}", settings.llm_token))
        .call()
    {
        if let Ok(body) = resp.body_mut().read_to_string() {
            if body.contains("\"data\"") && body.contains("\"id\"") {
                // could still be claude proxy that supports /models
                // check if any model id contains "claude"
                if body.contains("claude") {
                    return Ok("claude".to_string());
                }
                return Ok("openai".to_string());
            }
        }
    }

    // try claude-style: GET /models with x-api-key + anthropic-version
    if let Ok(mut resp) = agent.get(&openai_url)
        .header("x-api-key", &settings.llm_token)
        .header("anthropic-version", "2023-06-01")
        .call()
    {
        if let Ok(body) = resp.body_mut().read_to_string() {
            if body.contains("claude") || body.contains("\"id\"") {
                return Ok("claude".to_string());
            }
        }
    }

    // fallback: check if /chat/completions endpoint exists (openai)
    // vs /messages endpoint (claude) by sending OPTIONS or checking 404/405
    let chat_url = format!("{base}/chat/completions");
    if let Ok(_) = agent.post(&chat_url)
        .header("Authorization", &format!("Bearer {}", settings.llm_token))
        .header("Content-Type", "application/json")
        .send_json(&serde_json::json!({"model":"x","messages":[]}))
    {
        return Ok("openai".to_string());
    }

    // if chat/completions returned an error but not 404, it's likely openai-compatible
    // (model not found != endpoint not found)

    Ok("openai".to_string())
}

fn build_agent() -> Result<ureq::Agent, String> {
    let proxy_list = ProxyList::load();
    if let Some(px) = proxy_list.get_random() {
        let proxy_url = proxy_to_url(px);
        let proxy = ureq::Proxy::new(&proxy_url).map_err(|e| format!("proxy: {e}"))?;
        let config = ureq::Agent::config_builder()
            .proxy(Some(proxy))
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        Ok(config.new_agent())
    } else {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        Ok(config.new_agent())
    }
}

fn proxy_to_url(px: &ProxyConfig) -> String {
    let auth = match (&px.username, &px.password) {
        (Some(u), Some(p)) => format!("{u}:{p}@"),
        (Some(u), None) => format!("{u}@"),
        _ => String::new(),
    };
    match px.proxy_type {
        ProxyType::Socks5 => format!("socks5://{auth}{}:{}", px.host, px.port),
        ProxyType::Socks4 => format!("socks4://{}:{}", px.host, px.port),
        ProxyType::Https => format!("http://{auth}{}:{}", px.host, px.port),
    }
}
