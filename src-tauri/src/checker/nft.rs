#[tauri::command]
pub async fn fetch_nft_preview(slug: String) -> Result<String, String> {
    let url = format!("https://t.me/nft/{}", slug);
    let mut resp = ureq::get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .call()
        .map_err(|e| format!("fetch page: {}", e))?;

    let mut html = String::new();
    resp.body_mut().read_to_string()
        .map(|s| { html = s; })
        .map_err(|e| format!("read body: {}", e))?;

    if html.is_empty() {
        return Err("empty response".into());
    }

    let og_image = extract_og_image(&html).ok_or_else(|| format!("no og:image in {} bytes", html.len()))?;

    let mut img_resp = ureq::get(&og_image)
        .header("User-Agent", "Mozilla/5.0")
        .call()
        .map_err(|e| format!("fetch image: {}", e))?;

    let img_bytes = img_resp.body_mut()
        .read_to_vec()
        .map_err(|e| format!("read image: {}", e))?;

    if img_bytes.is_empty() {
        return Err("empty image".into());
    }

    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &img_bytes);
    let mime = if og_image.contains(".png") { "image/png" } else { "image/jpeg" };
    Ok(format!("data:{};base64,{}", mime, b64))
}

pub fn extract_og_image(html: &str) -> Option<String> {
    for meta_start in html.match_indices("<meta").map(|(i, _)| i) {
        let meta_end = html[meta_start..].find('>').map(|i| meta_start + i)?;
        let tag = &html[meta_start..meta_end + 1];
        if !tag.contains("og:image") { continue; }
        if let Some(c_pos) = tag.find("content=\"") {
            let after = &tag[c_pos + 9..];
            if let Some(end) = after.find('"') {
                let url = &after[..end];
                if url.starts_with("http") {
                    return Some(url.to_string());
                }
            }
        }
    }
    None
}
