// phone number -> country code (ISO 3166-1 alpha-2) mapping
// uses phonenumber crate for parsing international phone numbers

pub fn phone_to_country_code(phone: &str) -> String {
    if phone.is_empty() {
        return String::new();
    }

    // normalize: ensure starts with +
    let normalized = if phone.starts_with('+') {
        phone.to_string()
    } else {
        format!("+{}", phone)
    };

    // handle special cases that phonenumber lib may get wrong
    if let Some(code) = special_prefix_match(&normalized) {
        return code.to_string();
    }

    match phonenumber::parse(None, &normalized) {
        Ok(pn) => {
            if let Some(country) = pn.country().id() {
                format!("{:?}", country)
            } else {
                String::new()
            }
        }
        Err(_) => String::new(),
    }
}

// handle ambiguous prefixes manually
fn special_prefix_match(phone: &str) -> Option<&'static str> {
    // +888 = anonymous number (Telegram Fragment)
    if phone.starts_with("+888") {
        return Some("ANON");
    }

    // +77 = KZ (Kazakhstan uses +7 7xx)
    if phone.starts_with("+77") {
        return Some("KZ");
    }

    // +7 (not +77) = RU
    if phone.starts_with("+7") {
        return Some("RU");
    }

    // bare 8 prefix (russian domestic stored as +8xxxxxxxxxx) - only 11 digits total
    if phone.starts_with("+89") && phone.len() == 12 {
        return Some("RU");
    }

    // +44 = GB (phonenumber lib may return GG/JE/IM for channel islands)
    if phone.starts_with("+44") {
        return Some("GB");
    }

    None
}

