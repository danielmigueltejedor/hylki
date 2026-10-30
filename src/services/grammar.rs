//! Grammar and style checking (#452): what is written in the composer goes
//! to a LanguageTool server, one the user runs or one they have an account
//! with, and the problems it finds come back as places in that text.
//!
//! The settings live in `grammar.toml`; a LanguageTool Premium key, when
//! there is one, is in the keyring. Each paragraph is checked in the
//! language it is written in, told offline from among the languages chosen
//! for spelling, so a message written partly in French and partly in English
//! is checked as both.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::i18n::{i18n, i18n_f};

mod language;

use language::{pieces, server_code};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Settings {
    /// Check what is written as it is written.
    #[serde(default)]
    pub(crate) enabled: bool,
    /// The server, e.g. `http://localhost:8081` or
    /// `https://api.languagetoolplus.com`.
    #[serde(default)]
    pub(crate) url: String,
    /// LanguageTool Premium: the account's email address, sent with the key.
    #[serde(default)]
    pub(crate) username: String,
}

impl Settings {
    /// Whether there is anything to check with.
    pub(crate) fn active(&self) -> bool {
        self.enabled && endpoint(&self.url).is_some()
    }
}

/// One problem in the text: where it is, in UTF-16 code units as the
/// editor's script counts them (and as the server does), what is wrong,
/// and what could go there instead.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Issue {
    pub(crate) offset: usize,
    pub(crate) length: usize,
    pub(crate) message: String,
    pub(crate) replacements: Vec<String>,
    pub(crate) rule: String,
    /// A matter of style rather than a mistake, marked more quietly.
    pub(crate) style: bool,
}

/// The most suggestions a problem keeps: the menu shows them all, and the
/// server can offer dozens for a single word.
const SUGGESTIONS: usize = 5;
/// The most characters sent in one request: under the 20,000 the public
/// server takes from an account without a key.
const PIECE: usize = 15_000;
/// Past this nothing is checked: a long forwarded log pasted in would
/// take minutes and use up an account's allowance.
const LIMIT: usize = 100_000;

// ─── Settings file and key ────────────────────────────────────────────────

fn path() -> Option<PathBuf> {
    crate::config::config_file("grammar.toml")
}

pub(crate) fn load() -> Settings {
    crate::config::read_toml(path()).unwrap_or_default()
}

pub(crate) fn save(settings: &Settings) {
    let Some(path) = path() else { return };
    match toml::to_string_pretty(settings) {
        Ok(text) => {
            if let Err(e) = crate::config::write_private_file(&path, &text) {
                tracing::warn!("could not save the grammar settings: {e}");
            }
        }
        Err(e) => tracing::warn!("could not save the grammar settings: {e}"),
    }
}

const KEY_NAME: &str = "grammar:languagetool";

/// The key as last read from or written to the keyring, `None` until then,
/// so the composer reads the keyring once a session and not per check.
static KEY: Mutex<Option<String>> = Mutex::new(None);

/// The key, from the keyring (blocking: the keyring thread's job).
pub(crate) fn load_key() -> String {
    let key = crate::config::load_cloud_password(KEY_NAME)
        .map(|k| k.trim().to_string())
        .unwrap_or_default();
    if let Ok(mut k) = KEY.lock() {
        *k = Some(key.clone());
    }
    key
}

/// The key as already read, if it has been.
pub(crate) fn known_key() -> Option<String> {
    KEY.lock().ok()?.clone()
}

pub(crate) fn store_key(key: &str) {
    let key = key.trim();
    if key.is_empty() {
        crate::config::delete_cloud_password(KEY_NAME);
    } else if let Err(e) = crate::config::store_cloud_password(KEY_NAME, key) {
        tracing::warn!("could not store the LanguageTool key: {e}");
    }
    if let Ok(mut k) = KEY.lock() {
        *k = Some(key.to_string());
    }
}

// ─── Ignoring ─────────────────────────────────────────────────────────────

/// Problems the user said to leave alone, by rule and the text it was
/// found in, for the rest of the session and in every composer.
static IGNORED: Mutex<Option<HashSet<(String, String)>>> = Mutex::new(None);

pub(crate) fn ignore(rule: &str, text: &str) {
    if let Ok(mut set) = IGNORED.lock() {
        set.get_or_insert_with(Default::default)
            .insert((rule.to_string(), text.to_string()));
    }
}

fn ignored(rule: &str, text: &str) -> bool {
    IGNORED.lock().ok().is_some_and(|set| {
        set.as_ref()
            .is_some_and(|s| s.contains(&(rule.to_string(), text.to_string())))
    })
}

// ─── Checking ─────────────────────────────────────────────────────────────

/// The server's check address for what was typed into Settings: a bare
/// host, or one with `/v2` or `/v2/check` already on it.
fn endpoint(url: &str) -> Option<String> {
    let base = url.trim().trim_end_matches('/');
    if !(base.starts_with("https://") || base.starts_with("http://")) {
        return None;
    }
    let base = base
        .strip_suffix("/v2/check")
        .or_else(|| base.strip_suffix("/v2"))
        .unwrap_or(base);
    Some(format!("{base}/v2"))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build()
}

/// Check `text` (blocking; call off the UI thread). `languages` are the
/// spelling languages ("fr_FR"); `spelling` says the spell checker is on,
/// which then keeps the misspelled words to itself.
pub(crate) fn check(
    settings: &Settings,
    key: &str,
    text: &str,
    languages: &[String],
    spelling: bool,
) -> Result<Vec<Issue>, String> {
    let Some(base) = endpoint(&settings.url) else {
        return Err(i18n(
            "Enter the LanguageTool server's address in Settings → Composing.",
        ));
    };
    if text.chars().count() > LIMIT {
        return Ok(Vec::new());
    }
    let agent = agent();
    let supported = supported_languages(&agent, &base);
    let mut codes: Vec<String> = Vec::new();
    for l in languages {
        if let Some(c) = server_code(l, &supported) {
            if !codes.contains(&c) {
                codes.push(c);
            }
        }
    }
    let mut out = Vec::new();
    for piece in pieces(text, &codes, PIECE) {
        let found = request(
            &agent,
            &base,
            settings,
            key,
            piece.text,
            &piece.language,
            &codes,
        )?;
        for m in found {
            let issue_type = m.rule.issue_type.as_str();
            if spelling && is_speller(&m.rule.id) {
                continue;
            }
            let offset = piece.offset + m.offset;
            if ignored(&m.rule.id, &utf16_slice(text, offset, m.length)) {
                continue;
            }
            out.push(Issue {
                offset,
                length: m.length,
                message: if m.short_message.trim().is_empty() {
                    m.message
                } else {
                    m.short_message
                },
                replacements: m
                    .replacements
                    .into_iter()
                    .map(|r| r.value)
                    .take(SUGGESTIONS)
                    .collect(),
                rule: m.rule.id,
                style: matches!(
                    issue_type,
                    "style" | "register" | "locale-violation" | "inconsistency"
                ),
            });
        }
    }
    Ok(out)
}

/// Whether a rule is the server's spell checker ("MORFOLOGIK_RULE_EN_US",
/// "GERMAN_SPELLER_RULE", "FR_SPELLING_RULE"), which the spell checker
/// here already covers, with the user's own added words. Not every rule
/// the server calls a misspelling is one: "Their is" is a grammar rule.
fn is_speller(rule: &str) -> bool {
    [
        "MORFOLOGIK_RULE",
        "HUNSPELL",
        "SPELLER_RULE",
        "SPELLING_RULE",
    ]
    .iter()
    .any(|k| rule.contains(k))
}

/// Ask the server whether it answers: its name and version, or why not.
pub(crate) fn check_server(settings: &Settings, key: &str) -> Result<String, String> {
    let Some(base) = endpoint(&settings.url) else {
        return Err(i18n("Enter the server's address first."));
    };
    let agent = agent();
    let mut form = vec![("text", "This are a test."), ("language", "en-US")];
    if !settings.username.trim().is_empty() && !key.is_empty() {
        form.push(("username", settings.username.trim()));
        form.push(("apiKey", key));
    }
    let answer: Answer = post(&agent, &base, &form)?;
    Ok(format!(
        "{} {}",
        answer.software.name, answer.software.version
    ))
}

fn request(
    agent: &ureq::Agent,
    base: &str,
    settings: &Settings,
    key: &str,
    text: &str,
    language: &str,
    preferred: &[String],
) -> Result<Vec<Match>, String> {
    let preferred = preferred.join(",");
    let mut form = vec![("text", text), ("language", language)];
    if language == "auto" && !preferred.is_empty() {
        form.push(("preferredLanguages", &preferred));
    }
    if !settings.username.trim().is_empty() && !key.is_empty() {
        form.push(("username", settings.username.trim()));
        form.push(("apiKey", key));
    }
    Ok(post::<Answer>(agent, base, &form)?.matches)
}

fn post<T: serde::de::DeserializeOwned>(
    agent: &ureq::Agent,
    base: &str,
    form: &[(&str, &str)],
) -> Result<T, String> {
    match agent.post(&format!("{base}/check")).send_form(form) {
        Ok(resp) => resp
            .into_json::<T>()
            .map_err(|_| i18n("The server's answer could not be read.")),
        Err(ureq::Error::Status(code, resp)) => {
            Err(refusal(code, &resp.into_string().unwrap_or_default()))
        }
        Err(ureq::Error::Transport(t)) => Err(unreachable(t.kind(), &t.to_string())),
    }
}

/// Why nothing answered, in words: the cause of a refused connection is
/// the server not running, and the user can do something about that.
fn unreachable(kind: ureq::ErrorKind, detail: &str) -> String {
    match kind {
        ureq::ErrorKind::ConnectionFailed => {
            i18n("Nothing answers at that address. Check that the server is running.")
        }
        ureq::ErrorKind::Dns => i18n("The server's name could not be found."),
        ureq::ErrorKind::InvalidUrl | ureq::ErrorKind::UnknownScheme => {
            i18n("The server's address is not valid.")
        }
        _ => i18n_f("The server could not be reached: {e}", &[("e", detail)]),
    }
}

/// What a refusal means, in words. The server explains itself in plain
/// text, as its first line.
fn refusal(code: u16, body: &str) -> String {
    let said: String = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .trim_start_matches("Error:")
        .trim()
        .chars()
        .take(200)
        .collect();
    let what = match code {
        401 | 403 => i18n("The server did not accept the username and key."),
        413 => i18n("The message is too long for the server."),
        429 => i18n("Too many requests: try again in a moment."),
        _ => i18n_f(
            "The server refused the request (error {code}).",
            &[("code", &code.to_string())],
        ),
    };
    if said.is_empty() {
        what
    } else {
        format!("{what} ({said})")
    }
}

/// The languages a server checks, by the codes it takes ("fr", "en-US"),
/// asked for once per server a session. Empty when it would not say,
/// and then the spelling languages go as they are.
fn supported_languages(agent: &ureq::Agent, base: &str) -> Vec<String> {
    static KNOWN: Mutex<Option<HashMap<String, Vec<String>>>> = Mutex::new(None);
    if let Some(list) = KNOWN
        .lock()
        .ok()
        .and_then(|m| m.as_ref()?.get(base).cloned())
    {
        return list;
    }
    #[derive(Deserialize)]
    struct Language {
        #[serde(rename = "longCode")]
        long_code: String,
    }
    let Some(list) = agent
        .get(&format!("{base}/languages"))
        .call()
        .ok()
        .and_then(|r| r.into_json::<Vec<Language>>().ok())
    else {
        return Vec::new();
    };
    let list: Vec<String> = list.into_iter().map(|l| l.long_code).collect();
    if let Ok(mut m) = KNOWN.lock() {
        m.get_or_insert_with(Default::default)
            .insert(base.to_string(), list.clone());
    }
    list
}

/// The part of `text` at a UTF-16 offset and length.
fn utf16_slice(text: &str, offset: usize, length: usize) -> String {
    let units: Vec<u16> = text.encode_utf16().skip(offset).take(length).collect();
    String::from_utf16_lossy(&units)
}

#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    software: Software,
    #[serde(default)]
    matches: Vec<Match>,
}

#[derive(Default, Deserialize)]
struct Software {
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
}

#[derive(Deserialize)]
struct Match {
    #[serde(default)]
    message: String,
    #[serde(default, rename = "shortMessage")]
    short_message: String,
    offset: usize,
    length: usize,
    #[serde(default)]
    replacements: Vec<Replacement>,
    #[serde(default)]
    rule: Rule,
}

#[derive(Deserialize)]
struct Replacement {
    value: String,
}

#[derive(Default, Deserialize)]
struct Rule {
    #[serde(default)]
    id: String,
    #[serde(default, rename = "issueType")]
    issue_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_takes_any_common_form() {
        for url in [
            "http://localhost:8081",
            "http://localhost:8081/",
            "http://localhost:8081/v2",
            "http://localhost:8081/v2/check",
        ] {
            assert_eq!(
                endpoint(url).as_deref(),
                Some("http://localhost:8081/v2"),
                "{url}"
            );
        }
        assert_eq!(endpoint("localhost:8081"), None);
        assert_eq!(endpoint(""), None);
    }

    #[test]
    fn offsets_count_as_the_editor_does() {
        // "😀" is two UTF-16 units, "é" one.
        assert_eq!(utf16_slice("😀 é a test", 3, 1), "é");
        assert_eq!(utf16_slice("😀 é a test", 0, 2), "😀");
    }

    #[test]
    fn a_server_that_is_not_running_says_so() {
        let settings = Settings {
            enabled: true,
            url: "http://127.0.0.1:1".into(),
            username: String::new(),
        };
        let why = check_server(&settings, "").unwrap_err();
        assert!(why.starts_with("Nothing answers"), "{why}");
    }

    #[test]
    fn refusals_say_what_went_wrong() {
        let r = refusal(400, "Error: Missing 'text' or 'data' parameter\n\tat ...");
        assert!(r.contains("400") && r.contains("Missing 'text'"), "{r}");
        assert_eq!(
            refusal(403, ""),
            i18n("The server did not accept the username and key.")
        );
    }

    #[test]
    fn only_the_spell_checkers_rules_are_left_to_the_spell_checker() {
        for rule in [
            "MORFOLOGIK_RULE_EN_US",
            "GERMAN_SPELLER_RULE",
            "FR_SPELLING_RULE",
            "HUNSPELL_RULE",
        ] {
            assert!(is_speller(rule), "{rule}");
        }
        assert!(!is_speller("THEIR_IS"));
        assert!(!is_speller("NON3PRS_VERB"));
    }

    #[test]
    fn the_answer_is_read() {
        let json = r#"{"software":{"name":"LanguageTool","version":"6.8"},
            "matches":[{"message":"Use “à”.","shortMessage":"","offset":35,"length":1,
            "replacements":[{"value":"à"}],"rule":{"id":"A_ACCENT","issueType":"grammar"}}]}"#;
        let a: Answer = serde_json::from_str(json).unwrap();
        assert_eq!(a.software.version, "6.8");
        assert_eq!(a.matches[0].offset, 35);
        assert_eq!(a.matches[0].replacements[0].value, "à");
        assert_eq!(a.matches[0].rule.issue_type, "grammar");
    }

    /// Against a real server:
    /// `LANGUAGETOOL_LIVE=http://localhost:8010 cargo test -- --ignored live_languagetool`.
    #[test]
    #[ignore]
    fn live_languagetool_checks_each_language() {
        let url = std::env::var("LANGUAGETOOL_LIVE").unwrap();
        let settings = Settings {
            enabled: true,
            url,
            username: String::new(),
        };
        let text = "Je suis allé a la plage hier avec mes amis et nous avons mangé des glaces.\n\n\
                    Yesterday we goes to the beach with our friends and ate some ice cream.";
        let found = check(&settings, "", text, &["en_US".into(), "fr_FR".into()], true).unwrap();
        let words: Vec<String> = found
            .iter()
            .map(|i| utf16_slice(text, i.offset, i.length))
            .collect();
        assert!(words.iter().any(|w| w == "a"), "{words:?}");
        assert!(words.iter().any(|w| w.contains("goes")), "{words:?}");
        let server = check_server(&settings, "").unwrap();
        assert!(server.starts_with("LanguageTool "), "{server}");
    }
}
