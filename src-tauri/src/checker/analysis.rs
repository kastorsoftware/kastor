// spambot response analysis, date parsing, seed detection

use crate::mtproto::tl;

// the clickable "Telegram Terms of Service" link in a frozen-account reply
// carries this URL inside a messageEntityTextUrl entity (not in the visible text).
fn has_tos_link(msg: &tl::ParsedMessage) -> bool {
    msg.entity_urls.iter().any(|u| u.contains("telegram.org/tos"))
        // legacy fallback: some old replies embedded the url directly in the text
        || msg.text.contains("telegram.org/tos")
}

// exact replica of python spambot analysis logic
pub fn analyze_spambot_response(messages: &[tl::ParsedMessage]) -> String {
    let msg = match messages.iter().find(|m| !m.text.contains("/start") && m.text.len() > 10) {
        Some(m) => m,
        None => return crate::i18n::t("status_perm_spam"),
    };

    if msg.reply_markup_rows > 0 {
        if msg.reply_markup_rows > 2 {
            if let Some(date_str) = find_utc_time(&msg.text) {
                return format!("{}({})", crate::i18n::t("status_geo_spam"), date_str);
            } else if has_tos_link(msg) {
                return crate::i18n::t("status_frozen");
            } else {
                return crate::i18n::t("status_perm_spam");
            }
        } else {
            if msg.text.contains("Telegram Premium") {
                return crate::i18n::t("status_geo_spam");
            } else {
                return crate::i18n::t("status_clean");
            }
        }
    } else {
        // frozen replies have no buttons at all but still carry the ToS link
        if has_tos_link(msg) {
            return crate::i18n::t("status_frozen");
        }
        if msg.reply_markup_rows == 0 && msg.id == 0 {
            return analyze_spambot_text_fallback(&msg.text);
        }
        return crate::i18n::t("status_perm_spam");
    }
}

fn analyze_spambot_text_fallback(text: &str) -> String {
    if let Some(date_str) = find_utc_time(text) {
        format!("{}({})", crate::i18n::t("status_geo_spam"), date_str)
    } else if text.contains("Telegram Premium") {
        crate::i18n::t("status_geo_spam")
    } else if text.contains("telegram.org/tos") {
        crate::i18n::t("status_frozen")
    } else if text.contains("SpamBlock") || text.contains("spam") {
        crate::i18n::t("status_perm_spam")
    } else {
        crate::i18n::t("status_clean")
    }
}

fn find_utc_time(text: &str) -> Option<String> {
    if let Some(utc_pos) = text.find("UTC") {
        let before = &text[..utc_pos];
        let start = if before.len() > 25 { before.len() - 25 } else { 0 };
        let snippet = &before[start..];
        if snippet.contains(':') {
            return Some(format!("{} UTC", snippet.trim()));
        }
    }
    None
}

pub fn extract_premium_date_from_status(text: &str) -> Option<i64> {
    // telegram returns dates as DD.MM.YYYY in premium promo status
    for i in 0..text.len().saturating_sub(9) {
        let slice = &text[i..];
        if slice.len() < 10 { break; }
        let chunk = &slice[..10];
        let parts: Vec<&str> = chunk.split('.').collect();
        if parts.len() == 3 && parts[0].len() == 2 && parts[1].len() == 2 && parts[2].len() == 4 {
            if let (Ok(day), Ok(month), Ok(year)) = (
                parts[0].parse::<i64>(),
                parts[1].parse::<i64>(),
                parts[2].parse::<i64>(),
            ) {
                if year >= 2020 && year <= 2035 && month >= 1 && month <= 12 && day >= 1 && day <= 31 {
                    return Some(date_to_ts(year, month, day));
                }
            }
        }
    }
    None
}

pub fn date_to_ts(year: i64, month: i64, day: i64) -> i64 {
    let month_days: [i64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut days: i64 = 0;
    for y in 1970..year {
        days += if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) { 366 } else { 365 };
    }
    for m in 1..month {
        days += month_days[(m - 1) as usize];
        if m == 2 && year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
            days += 1;
        }
    }
    days += day - 1;
    days * 86400
}

// BIP39 seed phrase detection
pub fn is_seed_phrase(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 12 || words.len() > 24 { return false; }
    if words.len() % 3 != 0 { return false; }
    words.iter().all(|w| {
        w.len() >= 3 && w.len() <= 8 && w.chars().all(|c| c.is_ascii_lowercase())
    })
}

pub fn chrono_format_ts(ts: i64) -> String {
    let secs_per_day: i64 = 86400;
    let mut days = ts / secs_per_day;
    let mut year = 1970i64;
    loop {
        let days_in_year = if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 366 } else { 365 };
        if days < days_in_year { break; }
        days -= days_in_year;
        year += 1;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 0;
    for (i, &md) in month_days.iter().enumerate() {
        if days < md as i64 { month = i + 1; break; }
        days -= md as i64;
    }
    let day = days + 1;
    format!("{:02}.{:02}.{}", day, month, year)
}
