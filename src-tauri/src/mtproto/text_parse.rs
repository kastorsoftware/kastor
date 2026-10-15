// text parsing utilities (not TL-related)

// extract invite hash/slug from a t.me link.
// returns (kind, hash_or_slug) where kind is "private" for "/+xxx" or "/joinchat/xxx",
// "addlist" for "/addlist/xxx", or "public" for "/username".
pub fn parse_invite_link(link: &str) -> Option<(&'static str, String)> {
    let link = link.trim();
    let path = link
        .strip_prefix("https://t.me/")
        .or_else(|| link.strip_prefix("http://t.me/"))
        .or_else(|| link.strip_prefix("t.me/"))
        .or_else(|| link.strip_prefix("https://telegram.me/"))
        .or_else(|| link.strip_prefix("telegram.me/"))?;

    let path = path.split(['?', '#']).next().unwrap_or(path);

    if let Some(rest) = path.strip_prefix("addlist/") {
        let slug = rest.trim_end_matches('/');
        if slug.is_empty() {
            return None;
        }
        return Some(("addlist", slug.to_string()));
    }
    if let Some(rest) = path.strip_prefix("joinchat/") {
        let h = rest.trim_end_matches('/');
        if h.is_empty() {
            return None;
        }
        return Some(("private", h.to_string()));
    }
    if let Some(rest) = path.strip_prefix('+') {
        let h = rest.trim_end_matches('/');
        if h.is_empty() {
            return None;
        }
        return Some(("private", h.to_string()));
    }
    let username = path.trim_end_matches('/').trim_start_matches('@');
    if username.is_empty() {
        return None;
    }
    Some(("public", username.to_string()))
}

// parse a t.me/<channel>/<msg_id> post link -> (channel_username, msg_id)
pub fn parse_post_link(link: &str) -> Option<(String, i32)> {
    let link = link.trim();
    let path = link
        .strip_prefix("https://t.me/")
        .or_else(|| link.strip_prefix("http://t.me/"))
        .or_else(|| link.strip_prefix("t.me/"))?;
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 2 {
        if parts[0] == "c" && parts.len() >= 3 {
            let id = parts[2].parse::<i32>().ok()?;
            return Some((format!("c/{}", parts[1]), id));
        }
        let id = parts[1].parse::<i32>().ok()?;
        return Some((parts[0].to_string(), id));
    }
    None
}
