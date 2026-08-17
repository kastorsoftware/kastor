// randomizer: text character substitution with preset and custom maps.
// supports probability-based replacement so not every character is swapped.

use rand::Rng;
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum RandomizerMode {
    Standard,
    Aggressive,
    Custom,
}

#[derive(Deserialize, Clone, Debug)]
pub struct RandomizerRequest {
    pub mode: RandomizerMode,
    pub text: String,
    pub custom_pairs: Option<String>,
    pub chance: Option<u32>,
}

#[tauri::command]
pub fn randomize_text(req: RandomizerRequest) -> String {
    let pairs = match req.mode {
        RandomizerMode::Standard => standard_pairs(),
        RandomizerMode::Aggressive => aggressive_pairs(),
        RandomizerMode::Custom => parse_custom_pairs(req.custom_pairs.as_deref().unwrap_or("")),
    };
    let chance = req.chance.unwrap_or(100).min(100);
    apply_replacements(&req.text, &pairs, chance)
}

fn apply_replacements(text: &str, pairs: &[(char, char)], chance: u32) -> String {
    use std::collections::HashMap;
    let mut rng = rand::thread_rng();

    let mut map: HashMap<char, Vec<char>> = HashMap::new();
    for &(from, to) in pairs {
        let entry = map.entry(from).or_insert_with(|| vec![from]);
        if !entry.contains(&to) {
            entry.push(to);
        }
    }

    text.chars()
        .map(|c| {
            if let Some(options) = map.get(&c) {
                if chance >= 100 || rng.gen_range(0..100) < chance {
                    let idx = rng.gen_range(0..options.len());
                    return options[idx];
                }
            }
            c
        })
        .collect()
}

fn parse_custom_pairs(input: &str) -> Vec<(char, char)> {
    input
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() { return None; }
            let parts: Vec<&str> = line.splitn(2, ':').collect();
            if parts.len() != 2 { return None; }
            let from_chars: Vec<char> = parts[0].trim().chars().collect();
            let to_chars: Vec<char> = parts[1].trim().chars().collect();
            if from_chars.len() == 1 && to_chars.len() == 1 {
                Some((from_chars[0], to_chars[0]))
            } else {
                None
            }
        })
        .collect()
}

fn standard_pairs() -> Vec<(char, char)> {
    vec![
        ('\u{0430}', 'a'), ('\u{0435}', 'e'), ('\u{043E}', 'o'), ('\u{0440}', 'p'),
        ('\u{0441}', 'c'), ('\u{0443}', 'y'), ('\u{0445}', 'x'), ('\u{0456}', 'i'),
        ('\u{0458}', 'j'), ('\u{0455}', 's'), ('\u{04BB}', 'h'), ('\u{051B}', 'q'),
        ('\u{051D}', 'w'),
        ('\u{0410}', 'A'), ('\u{0412}', 'B'), ('\u{0421}', 'C'), ('\u{0415}', 'E'),
        ('\u{041D}', 'H'), ('\u{0406}', 'I'), ('\u{0408}', 'J'), ('\u{041A}', 'K'),
        ('\u{041C}', 'M'), ('\u{041E}', 'O'), ('\u{0420}', 'P'), ('\u{0405}', 'S'),
        ('\u{0422}', 'T'), ('\u{0425}', 'X'), ('\u{0423}', 'Y'), ('\u{0417}', 'Z'),
        ('a', '\u{0430}'), ('c', '\u{0441}'), ('e', '\u{0435}'), ('o', '\u{043E}'),
        ('p', '\u{0440}'), ('x', '\u{0445}'), ('y', '\u{0443}'), ('s', '\u{0455}'),
        ('i', '\u{0456}'), ('j', '\u{0458}'), ('h', '\u{04BB}'), ('q', '\u{051B}'),
        ('w', '\u{051D}'),
        ('A', '\u{0410}'), ('B', '\u{0412}'), ('C', '\u{0421}'), ('E', '\u{0415}'),
        ('H', '\u{041D}'), ('I', '\u{0406}'), ('J', '\u{0408}'), ('K', '\u{041A}'),
        ('M', '\u{041C}'), ('O', '\u{041E}'), ('P', '\u{0420}'), ('S', '\u{0405}'),
        ('T', '\u{0422}'), ('X', '\u{0425}'), ('Y', '\u{0423}'),
        ('\u{043E}', '\u{03BF}'), ('\u{0440}', '\u{03C1}'), ('\u{0430}', '\u{03B1}'),
        ('a', '\u{1D44E}'), ('b', '\u{1D44F}'), ('c', '\u{1D450}'), ('d', '\u{1D451}'),
        ('e', '\u{1D452}'), ('f', '\u{1D453}'), ('g', '\u{1D454}'), ('n', '\u{1D45B}'),
        ('m', '\u{1D45A}'), ('r', '\u{1D45F}'), ('u', '\u{1D462}'), ('v', '\u{1D463}'),
    ]
}

fn aggressive_pairs() -> Vec<(char, char)> {
    vec![
        ('\u{0430}', '@'), ('\u{0410}', '4'), ('\u{0431}', '6'), ('\u{0411}', '6'),
        ('\u{0432}', '\u{044C}'), ('\u{0433}', 'r'), ('\u{0435}', '3'), ('\u{0415}', '3'),
        ('\u{0437}', '3'), ('\u{0417}', '3'), ('\u{0438}', 'u'), ('\u{0418}', 'U'),
        ('\u{043B}', '\u{043F}'), ('\u{043E}', '0'), ('\u{041E}', '0'),
        ('\u{0441}', '('), ('\u{0421}', '('), ('\u{0442}', '\u{0433}'),
        ('\u{0447}', '4'), ('\u{0427}', '4'), ('\u{0448}', 'w'), ('\u{0428}', 'W'),
        ('a', '@'), ('A', '4'), ('b', '6'), ('B', '8'), ('c', '('), ('C', '('),
        ('e', '3'), ('E', '3'), ('g', '9'), ('G', '6'), ('h', '#'),
        ('i', '1'), ('I', '1'), ('l', '1'), ('L', '7'), ('n', '\u{0438}'),
        ('o', '0'), ('O', '0'), ('q', '9'), ('r', '\u{0433}'),
        ('s', '$'), ('S', '$'), ('t', '+'), ('T', '7'), ('u', 'v'),
        ('z', '2'), ('Z', '2'),
    ]
}

pub fn randomize_text_internal(text: &str, chance: u32) -> String {
    let pairs = standard_pairs();
    apply_replacements(text, &pairs, chance)
}

/// Spintax processor: resolves {вариант1|вариант2|вариант3} syntax recursively.
/// Each `{...}` block picks one random option separated by `|`.
/// Supports nesting: {hello|{hi|hey}} → one of: hello, hi, hey
pub fn spin_text(input: &str) -> String {
    let mut rng = rand::thread_rng();
    spin_recursive(input, &mut rng)
}

fn spin_recursive(input: &str, rng: &mut impl rand::Rng) -> String {
    let bytes = input.as_bytes();
    let mut result = String::with_capacity(input.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'{' {
            // find matching closing brace (respecting nesting)
            let start = i + 1;
            let mut depth = 1;
            let mut end = start;
            while end < bytes.len() && depth > 0 {
                if bytes[end] == b'{' { depth += 1; }
                else if bytes[end] == b'}' { depth -= 1; }
                end += 1;
            }
            if depth == 0 {
                // end-1 is the position of the matching '}'
                let inner = &input[start..end - 1];
                // split by top-level pipes only
                let options = split_top_level_pipes(inner);
                if !options.is_empty() {
                    let chosen = options[rng.gen_range(0..options.len())];
                    // recursively process the chosen option
                    result.push_str(&spin_recursive(chosen, rng));
                }
                i = end;
            } else {
                // unmatched brace — output as-is
                result.push('{');
                i = start;
            }
        } else {
            result.push(input[i..].chars().next().unwrap());
            i += input[i..].chars().next().unwrap().len_utf8();
        }
    }
    result
}

fn split_top_level_pipes(input: &str) -> Vec<&str> {
    let bytes = input.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => { if depth > 0 { depth -= 1; } }
            b'|' if depth == 0 => {
                parts.push(&input[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&input[start..]);
    parts
}
