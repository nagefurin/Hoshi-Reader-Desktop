use std::borrow::Cow;
use std::sync::LazyLock;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use chrono::{Datelike, Timelike, Utc};
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::header::{COOKIE, ORIGIN, USER_AGENT},
        http::HeaderValue,
        protocol::Message,
    },
};
use regex::{Captures, Regex};
use rbook::Epub;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::sync::Mutex;

#[cfg(windows)]
use windows::{
    Foundation::AsyncStatus,
    Media::SpeechSynthesis::SpeechSynthesizer,
    Storage::Streams::DataReader,
    core::HSTRING,
    Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_SINGLETHREADED},
};

use crate::{book, library};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsChapter {
    pub spine_index: usize,
    pub title: String,
    pub text: String,
    pub ruby_text: String,
    pub ruby_ranges: Vec<TtsRubyRange>,
    pub image_offsets: Vec<usize>,
    pub image_index_start: usize,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TtsRubyRange {
    pub start: usize,
    pub end: usize,
    pub reading: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct TtsVoiceInfo {
    pub id: String,
    pub styles: Vec<String>,
    pub roles: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SystemTtsVoiceInfo {
    pub id: String,
    pub name: String,
    pub language: String,
    pub is_default: bool,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default, rename_all = "camelCase")]
pub struct TtsSynthesisOptions {
    pub volume: f32,
    pub style: String,
    pub style_degree: f32,
    pub role: String,
    pub leading_ms: i32,
    pub trailing_ms: i32,
    pub comma_ms: i32,
    pub semicolon_ms: i32,
    pub enumeration_comma_ms: i32,
    pub sentence_boundary_ms: i32,
    pub format: String,
    pub sample_rate: u32,
    pub effects_profile: String,
}

impl Default for TtsSynthesisOptions {
    fn default() -> Self {
        Self {
            volume: 0.0,
            style: String::new(),
            style_degree: 1.0,
            role: String::new(),
            leading_ms: -1,
            trailing_ms: -1,
            comma_ms: -1,
            semicolon_ms: -1,
            enumeration_comma_ms: -1,
            sentence_boundary_ms: -1,
            format: "mp3".to_string(),
            sample_rate: 24000,
            effects_profile: String::new(),
        }
    }
}

impl TtsSynthesisOptions {
    fn sample_rate(&self) -> u32 {
        if [16000, 24000, 48000].contains(&self.sample_rate) { self.sample_rate } else { 24000 }
    }

    fn is_wav(&self) -> bool {
        self.format.eq_ignore_ascii_case("wav")
    }

    fn volume(&self, google_cloud: bool) -> f32 {
        if !self.volume.is_finite() { 0.0 }
        else if google_cloud { self.volume.clamp(-96.0, 16.0) }
        else { self.volume.clamp(-100.0, 100.0) }
    }

    fn pause_ms(value: i32) -> Option<u32> {
        (value >= 0).then_some(value.clamp(0, 1000) as u32)
    }
}

static RE_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)</?(?:p|div|h[1-6]|li|ul|ol|blockquote|section|article|table|tr|td|th|br|hr|figure|figcaption)\b[^>]*>").unwrap()
});
static RE_RUBY_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<ruby\b[^>]*>.*?</ruby>").unwrap()
});
static RE_RUBY_RT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<rt\b[^>]*>(.*?)</rt>").unwrap()
});
static RE_RUBY_RP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<rp\b[^>]*>.*?</rp>").unwrap()
});
static RE_NUMERIC_ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&#(x[0-9a-fA-F]+|[0-9]+);").unwrap());
static RE_IMAGE_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<(?:img|image)\b[^>]*>").unwrap());
const IMAGE_MARKER: char = '\u{E000}';
const RUBY_START_MARKER: char = '\u{E001}';
const RUBY_END_MARKER: char = '\u{E002}';

fn decode_entities(value: &str) -> String {
    let decoded = RE_NUMERIC_ENTITY.replace_all(value, |captures: &Captures<'_>| {
        let number = &captures[1];
        let code = if number.starts_with('x') || number.starts_with('X') {
            u32::from_str_radix(&number[1..], 16).ok()
        } else {
            number.parse::<u32>().ok()
        };
        code.and_then(char::from_u32).unwrap_or('\u{fffd}').to_string()
    });
    decoded
        .replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn speakable_text(html: &str, use_ruby_readings: bool) -> String {
    let body = library::body(html);
    let ruby: Cow<'_, str> = if use_ruby_readings {
        RE_RUBY_BLOCK.replace_all(body, |captures: &Captures<'_>| {
            let ruby_markup = &captures[0];
            let readings = RE_RUBY_RT.captures_iter(ruby_markup)
                .map(|capture| library::strip(&capture[1]))
                .collect::<Vec<_>>()
                .join("");
            if readings.trim().is_empty() {
                library::strip(ruby_markup)
            } else {
                readings
            }
        })
    } else {
        Cow::Borrowed(body)
    };
    let with_breaks = RE_BLOCK.replace_all(&ruby, "\n");
    let decoded = decode_entities(&with_breaks);
    let plain = library::strip(&decoded);
    plain
        .split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn speakable_text_with_images(html: &str) -> (String, Vec<usize>, Vec<TtsRubyRange>) {
    let body = library::body(html);
    let mut readings = Vec::<String>::new();
    let marked_ruby = RE_RUBY_BLOCK.replace_all(body, |captures: &Captures<'_>| {
        let ruby_markup = &captures[0];
        let reading = RE_RUBY_RT.captures_iter(ruby_markup)
            .map(|capture| decode_entities(&library::strip(&capture[1])).trim().to_string())
            .collect::<Vec<_>>()
            .join("");
        let without_rt = RE_RUBY_RT.replace_all(ruby_markup, "");
        let base = RE_RUBY_RP.replace_all(&without_rt, "");
        if reading.trim().is_empty() {
            base.into_owned()
        } else {
            readings.push(reading);
            format!("{RUBY_START_MARKER}{}{RUBY_END_MARKER}", base)
        }
    });
    let marked = RE_IMAGE_TAG.replace_all(marked_ruby.as_ref(), IMAGE_MARKER.to_string());
    let with_breaks = RE_BLOCK.replace_all(&marked, "\n");
    let decoded = decode_entities(&with_breaks);
    let plain = library::strip(&decoded);
    let formatted = plain
        .split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    let mut text = String::with_capacity(formatted.len());
    let mut image_offsets = Vec::new();
    let mut ruby_ranges = Vec::new();
    let mut reading_iter = readings.into_iter();
    let mut ruby_start = None;
    let mut character_count = 0usize;
    for character in formatted.chars() {
        match character {
            RUBY_START_MARKER => {
                ruby_start = Some(character_count);
            }
            RUBY_END_MARKER => {
                if let (Some(start), Some(reading)) = (ruby_start.take(), reading_iter.next()) {
                    ruby_ranges.push(TtsRubyRange { start, end: character_count, reading });
                }
            }
            IMAGE_MARKER => image_offsets.push(character_count),
            _ => {
                text.push(character);
                character_count += 1;
            }
        }
    }
    (text, image_offsets, ruby_ranges)
}


fn request_client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(75))
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not initialize the TTS network client.".to_string())
}

async fn finish_audio(response: reqwest::Response) -> Result<String, String> {
    let status = response.status();
    if !status.is_success() {
        let details = response.text().await.unwrap_or_default();
        let details = details.chars().take(300).collect::<String>();
        return Err(if details.trim().is_empty() {
            format!("TTS endpoint returned HTTP {status}.")
        } else {
            format!("TTS endpoint returned HTTP {status}: {details}")
        });
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !(content_type.starts_with("audio/") || content_type.contains("octet-stream")) {
        return Err("The TTS endpoint did not return an audio response. Check the endpoint, model, and credentials.".to_string());
    }
    let bytes = response.bytes().await.map_err(|_| "Could not read the generated audio.".to_string())?;
    if bytes.is_empty() || bytes.len() > 25 * 1024 * 1024 {
        return Err("The TTS endpoint returned empty audio or an audio file larger than 25 MB.".to_string());
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

#[tauri::command]
pub async fn tts_load_text(app: AppHandle, id: String, _use_ruby_readings: bool) -> Result<Vec<TtsChapter>, String> {
    let path = library::book_epub_path(&app, &id).ok_or_else(|| format!("Book {id} was not found"))?;
    let epub = Epub::open(path).map_err(|error| error.to_string())?;
    let document = book::book_document(&epub);
    let mut chapters = Vec::new();
    let mut compact_spine_index = 0usize;
    let mut global_image_index = 0usize;

    for entry in epub.spine().iter() {
        let Some(item) = entry.manifest_entry() else {
            continue;
        };
        let spine_index = compact_spine_index;
        compact_spine_index += 1;
        let Ok(content) = item.read_str() else {
            continue;
        };

        let body = library::body(&content);
        let image_index_start = global_image_index;
        global_image_index += RE_IMAGE_TAG.find_iter(body).count();

        if !entry.is_linear() || item.properties().as_str().contains("nav") {
            continue;
        }
        let (text, image_offsets, ruby_ranges) = speakable_text_with_images(&content);
        let ruby_text = speakable_text(&content, true);
        if text.trim().is_empty() {
            continue;
        }
        let title = document
            .toc
            .iter()
            .find(|item| item.spine_index == spine_index)
            .map(|item| item.label.trim())
            .filter(|label| !label.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Chapter {}", spine_index + 1));
        chapters.push(TtsChapter { spine_index, title, text, ruby_text, ruby_ranges, image_offsets, image_index_start });
    }

    if chapters.is_empty() {
        return Err("No readable text was found in this EPUB.".to_string());
    }
    Ok(chapters)
}


const EDGE_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const EDGE_VERSION: &str = "143.0.3650.75";
const EDGE_BASE: &str = "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud";
const EDGE_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36 Edg/143.0.0.0";
const EDGE_ORIGIN: &str = "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold";
const EDGE_MAX_AUDIO_BYTES: usize = 16 * 1024 * 1024;



#[derive(Clone)]
struct MicrosoftTranslatorToken {
    region: String,
    bearer: String,
    expires_at: u64,
}

static MICROSOFT_TRANSLATOR_TOKEN: LazyLock<Mutex<Option<MicrosoftTranslatorToken>>> =
    LazyLock::new(|| Mutex::new(None));

fn japanese_microsoft_voices(bytes: &[u8], provider: &str) -> Result<Vec<TtsVoiceInfo>, String> {
    let root: Value = serde_json::from_slice(bytes).map_err(|error| {
        let preview = String::from_utf8_lossy(bytes).chars().take(240).collect::<String>();
        format!("{provider} returned an invalid voice list: {error}. Response: {preview}")
    })?;
    let values = root.as_array().ok_or_else(|| {
        let preview = String::from_utf8_lossy(bytes).chars().take(240).collect::<String>();
        format!("{provider} returned an invalid voice list: expected a JSON array. Response: {preview}")
    })?;
    let mut voices: Vec<TtsVoiceInfo> = Vec::new();
    for value in values {
        let Some(locale) = value.get("Locale").and_then(Value::as_str) else { continue };
        if !locale.eq_ignore_ascii_case("ja-JP") {
            continue;
        }
        let id = value.get("ShortName").and_then(Value::as_str)
            .ok_or_else(|| format!("{provider} voice entry did not contain ShortName."))?;
        let read_list = |key: &str| -> Vec<String> {
            value.get(key).and_then(Value::as_array).into_iter().flatten()
                .filter_map(Value::as_str).map(str::to_string).collect()
        };
        voices.push(TtsVoiceInfo {
            id: id.to_string(),
            styles: read_list("StyleList"),
            roles: read_list("RolePlayList"),
        });
    }
    voices.sort_by(|a, b| a.id.cmp(&b.id));
    voices.dedup_by(|a, b| a.id == b.id);
    Ok(voices)
}

fn azure_base(region: &str) -> Result<Url, String> {
    let region = region.trim().to_ascii_lowercase();
    if !Regex::new(r"^[a-z][a-z0-9]{1,39}$").unwrap().is_match(&region) {
        return Err("Enter a valid Azure Speech region, such as japaneast.".to_string());
    }
    Url::parse(&format!("https://{region}.tts.speech.microsoft.com"))
        .map_err(|_| "Could not prepare the Azure Speech endpoint.".to_string())
}

async fn tts_azure_voice_list(region: &str, api_key: &str) -> Result<Vec<TtsVoiceInfo>, String> {
    if api_key.trim().is_empty() {
        return Err("Enter an Azure Speech subscription key first.".to_string());
    }
    let client = request_client()?;
    let mut url = azure_base(region)?;
    url.set_path("/cognitiveservices/voices/list");
    let response = client.get(url)
        .header("Ocp-Apim-Subscription-Key", api_key.trim())
        .header(reqwest::header::USER_AGENT, "Hoshi Reader Desktop")
        .send().await
        .map_err(|error| format!("Could not connect to Azure Speech: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default().chars().take(240).collect::<String>();
        return Err(format!("Azure Speech voice list returned HTTP {status}: {detail}"));
    }
    let bytes = response.bytes().await.map_err(|error| format!("Could not read Azure Speech voices: {error}"))?;
    if bytes.len() > 5 * 1024 * 1024 {
        return Err("Azure Speech voice list response was unexpectedly large.".to_string());
    }
    japanese_microsoft_voices(&bytes, "Azure Speech")
}

async fn tts_google_cloud_voice_list(api_key: &str) -> Result<Vec<TtsVoiceInfo>, String> {
    if api_key.trim().is_empty() {
        return Err("Enter a Google Cloud Text-to-Speech API key first.".to_string());
    }
    let client = request_client()?;
    let mut url = Url::parse("https://texttospeech.googleapis.com/v1/voices").unwrap();
    url.query_pairs_mut().append_pair("languageCode", "ja-JP");
    let response = client.get(url)
        .header("X-Goog-Api-Key", api_key.trim())
        .send().await
        .map_err(|error| format!("Could not connect to Google Cloud Text-to-Speech: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default().chars().take(240).collect::<String>();
        return Err(format!("Google Cloud voice list returned HTTP {status}: {detail}"));
    }
    let bytes = response.bytes().await.map_err(|error| format!("Could not read Google Cloud voices: {error}"))?;
    if bytes.len() > 5 * 1024 * 1024 {
        return Err("Google Cloud voice list response was unexpectedly large.".to_string());
    }
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Google Cloud returned an invalid voice list: {error}"))?;
    let mut voices: Vec<TtsVoiceInfo> = data.get("voices").and_then(Value::as_array).into_iter().flatten()
        .filter(|voice| voice.get("languageCodes").and_then(Value::as_array).is_some_and(|languages| {
            languages.iter().any(|language| language.as_str() == Some("ja-JP"))
        }))
        .filter_map(|voice| voice.get("name").and_then(Value::as_str).map(|id| TtsVoiceInfo {
            id: id.to_string(),
            styles: Vec::new(),
            roles: Vec::new(),
        }))
        .collect();
    voices.sort_by(|a, b| a.id.cmp(&b.id));
    voices.dedup_by(|a, b| a.id == b.id);
    Ok(voices)
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut key_block = if key.len() > BLOCK {
        Sha256::digest(key).to_vec()
    } else {
        key.to_vec()
    };
    key_block.resize(BLOCK, 0);
    let mut inner = Vec::with_capacity(BLOCK + message.len());
    inner.extend(key_block.iter().map(|byte| *byte ^ 0x36));
    inner.extend_from_slice(message);
    let inner_hash = Sha256::digest(&inner);
    let mut outer = Vec::with_capacity(BLOCK + inner_hash.len());
    outer.extend(key_block.iter().map(|byte| *byte ^ 0x5c));
    outer.extend_from_slice(&inner_hash);
    Sha256::digest(&outer).to_vec()
}

fn microsoft_translator_signature(unix_seconds: u64, nonce: &str) -> Result<String, String> {
    const APPLICATION: &str = "MSTranslatorAndroidApp";
    const SIGNING_KEY: &str = "oik6PdDdMnOXemTbwvMn9de/h9lFnfBaCWbGMMZqqoSaQaqUOqjVGm5NqsmjcBI1x+sS9ugjB55HEJWRiFXYFw==";
    const ENDPOINT: &str = "https://dev.microsofttranslator.com/apps/endpoint?api-version=1.0";
    let date = chrono::DateTime::<Utc>::from_timestamp(unix_seconds as i64, 0)
        .ok_or_else(|| "System time is invalid for Microsoft Translator.".to_string())?
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
        .to_ascii_lowercase();
    let encoded_url = urlencoding::encode(ENDPOINT.trim_start_matches("https://")).into_owned();
    let input = format!("{APPLICATION}{encoded_url}{date}{nonce}").to_ascii_lowercase();
    let key = STANDARD.decode(SIGNING_KEY)
        .map_err(|_| "Could not initialize Microsoft Translator authorization.".to_string())?;
    let signature = STANDARD.encode(hmac_sha256(&key, input.as_bytes()));
    Ok(format!("{APPLICATION}::{signature}::{date}::{nonce}"))
}

async fn microsoft_translator_token(client: &Client) -> Result<MicrosoftTranslatorToken, String> {
    const ENDPOINT: &str = "https://dev.microsofttranslator.com/apps/endpoint?api-version=1.0";
    let mut cached = MICROSOFT_TRANSLATOR_TOKEN.lock().await;
    let now = Utc::now().timestamp().max(0) as u64;
    if let Some(token) = cached.as_ref().filter(|token| token.expires_at > now + 60) {
        return Ok(token.clone());
    }
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let signature = microsoft_translator_signature(now, &nonce)?;
    let response = client.post(ENDPOINT)
        .header("X-ClientVersion", "4.0.530a 5fe1dc6c")
        .header("X-UserId", &uuid::Uuid::new_v4().simple().to_string()[..16])
        .header("X-ClientTraceId", uuid::Uuid::new_v4().to_string())
        .header("X-MT-Signature", signature)
        .header("Accept-Language", "ja-JP")
        .body(Vec::new())
        .send().await
        .map_err(|error| format!("Could not obtain Microsoft Translator authorization: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default().chars().take(240).collect::<String>();
        return Err(format!("Microsoft Translator authorization returned HTTP {status}: {detail}"));
    }
    let bytes = response.bytes().await.map_err(|error| format!("Could not read Microsoft Translator authorization: {error}"))?;
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Microsoft Translator returned invalid authorization data: {error}"))?;
    let region = data.get("r").and_then(Value::as_str)
        .ok_or_else(|| "Microsoft Translator authorization did not include a region.".to_string())?
        .to_string();
    azure_base(&region)?;
    let bearer = data.get("t").and_then(Value::as_str)
        .ok_or_else(|| "Microsoft Translator authorization did not include a token.".to_string())?
        .to_string();
    if bearer.len() > 16_384 || bearer.chars().any(|character| character <= ' ' || !character.is_ascii()) {
        return Err("Microsoft Translator returned an invalid bearer token.".to_string());
    }
    let claims = bearer.split('.').nth(1)
        .ok_or_else(|| "Microsoft Translator returned a malformed bearer token.".to_string())?;
    let payload = URL_SAFE_NO_PAD.decode(claims).or_else(|_| URL_SAFE.decode(claims))
        .map_err(|_| "Microsoft Translator returned a malformed bearer token.".to_string())?;
    let claims: Value = serde_json::from_slice(&payload)
        .map_err(|_| "Microsoft Translator returned invalid token claims.".to_string())?;
    let expires_at = claims.get("exp").and_then(Value::as_u64)
        .ok_or_else(|| "Microsoft Translator token did not contain an expiration time.".to_string())?;
    if expires_at <= now {
        return Err("Microsoft Translator returned an expired token.".to_string());
    }
    let token = MicrosoftTranslatorToken { region, bearer, expires_at };
    *cached = Some(token.clone());
    Ok(token)
}

async fn tts_translator_voice_list() -> Result<Vec<TtsVoiceInfo>, String> {
    let client = request_client()?;
    let token = microsoft_translator_token(&client).await?;
    let mut url = azure_base(&token.region)?;
    url.set_path("/cognitiveservices/voices/list");
    let response = client.get(url)
        .bearer_auth(&token.bearer)
        .header(reqwest::header::USER_AGENT, "Hoshi Reader Desktop")
        .send().await
        .map_err(|error| format!("Could not connect to Microsoft Translator voices: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default().chars().take(240).collect::<String>();
        return Err(format!("Microsoft Translator voice list returned HTTP {status}: {detail}"));
    }
    let bytes = response.bytes().await.map_err(|error| format!("Could not read Microsoft Translator voices: {error}"))?;
    if bytes.len() > 5 * 1024 * 1024 {
        return Err("Microsoft Translator voice list response was unexpectedly large.".to_string());
    }
    japanese_microsoft_voices(&bytes, "Microsoft Translator")
}

fn azure_format(options: &TtsSynthesisOptions) -> String {
    let sample_rate = options.sample_rate();
    let khz = sample_rate / 1000;
    if options.is_wav() {
        format!("riff-{khz}khz-16bit-mono-pcm")
    } else {
        let bitrate = match sample_rate { 16000 => 64, 24000 => 96, _ => 192 };
        format!("audio-{khz}khz-{bitrate}kbitrate-mono-mp3")
    }
}

fn azure_ssml(text: &str, voice: &str, rate: f32, pitch: f32, options: &TtsSynthesisOptions) -> String {
    let rate_percent = ((rate.clamp(0.5, 2.0) - 1.0) * 100.0).round() as i32;
    let pitch = pitch.clamp(-12.0, 7.0).round() as i32;
    let volume = options.volume(false).round() as i32;
    let degree = if options.style_degree.is_finite() { options.style_degree.clamp(0.01, 2.0) } else { 1.0 };
    let content = format!(
        "<prosody rate=\"{:+}%\" pitch=\"{:+}st\" volume=\"{:+}%\">{}</prosody>",
        rate_percent, pitch, volume, edge_xml(text)
    );
    let styled = if options.style.trim().is_empty() {
        content
    } else {
        let role = if options.role.trim().is_empty() {
            String::new()
        } else {
            format!(" role=\"{}\"", edge_xml(&options.role))
        };
        format!(
            "<mstts:express-as style=\"{}\" styledegree=\"{degree}\"{role}>{content}</mstts:express-as>",
            edge_xml(&options.style)
        )
    };
    let pauses = [
        ("Leading-exact", options.leading_ms),
        ("Tailing-exact", options.trailing_ms),
        ("Comma-exact", options.comma_ms),
        ("Semicolon-exact", options.semicolon_ms),
        ("Enumerationcomma-exact", options.enumeration_comma_ms),
        ("Sentenceboundary-exact", options.sentence_boundary_ms),
    ].into_iter().filter_map(|(kind, value)| {
        TtsSynthesisOptions::pause_ms(value).map(|ms| {
            format!("<mstts:silence type=\"{kind}\" value=\"{ms}ms\"/>")
        })
    }).collect::<String>();
    format!(
        "<speak version=\"1.0\" xmlns=\"http://www.w3.org/2001/10/synthesis\" xmlns:mstts=\"https://www.w3.org/2001/mstts\" xml:lang=\"ja-JP\"><voice name=\"{}\">{pauses}{styled}</voice></speak>",
        edge_xml(voice)
    )
}

async fn tts_azure_synthesize(
    client: &Client, region: &str, api_key: &str, text: &str, voice: &str, rate: f32, pitch: f32,
    options: &TtsSynthesisOptions,
) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err("Enter an Azure Speech subscription key first.".to_string());
    }
    let mut url = azure_base(region)?;
    url.set_path("/cognitiveservices/v1");
    let output_format = azure_format(options);
    let response = client.post(url)
        .header("Ocp-Apim-Subscription-Key", api_key.trim())
        .header("X-Microsoft-OutputFormat", output_format)
        .header("User-Agent", "Hoshi Reader Desktop")
        .header(CONTENT_TYPE, "application/ssml+xml")
        .body(azure_ssml(text, voice, rate, pitch, options))
        .send().await
        .map_err(|error| format!("Could not connect to Azure Speech: {error}"))?;
    finish_audio(response).await
}

async fn tts_google_cloud_synthesize(
    client: &Client, api_key: &str, text: &str, voice: &str, rate: f32, pitch: f32,
    options: &TtsSynthesisOptions,
) -> Result<String, String> {
    if api_key.trim().is_empty() {
        return Err("Enter a Google Cloud Text-to-Speech API key first.".to_string());
    }
    let voice = if voice.trim().is_empty() { "ja-JP-Neural2-B" } else { voice.trim() };
    let mut audio_config = json!({
        "audioEncoding": if options.is_wav() { "LINEAR16" } else { "MP3" },
        "speakingRate": rate.clamp(0.25, 2.0),
        "sampleRateHertz": options.sample_rate(),
        "volumeGainDb": options.volume(true),
    });
    if !voice.contains("Chirp") && !voice.contains("Journey") {
        audio_config["pitch"] = json!(pitch.clamp(-20.0, 20.0));
    }
    let profile = options.effects_profile.trim();
    if !profile.is_empty() {
        audio_config["effectsProfileId"] = json!([profile]);
    }
    let response = client.post("https://texttospeech.googleapis.com/v1/text:synthesize")
        .header("X-Goog-Api-Key", api_key.trim())
        .json(&json!({
            "input": { "text": text },
            "voice": { "languageCode": "ja-JP", "name": voice },
            "audioConfig": audio_config
        }))
        .send().await
        .map_err(|error| format!("Could not connect to Google Cloud Text-to-Speech: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default().chars().take(300).collect::<String>();
        return Err(format!("Google Cloud synthesis returned HTTP {status}: {detail}"));
    }
    let bytes = response.bytes().await.map_err(|error| format!("Could not read Google Cloud audio response: {error}"))?;
    if bytes.len() > EDGE_MAX_AUDIO_BYTES {
        return Err("Google Cloud audio exceeds the 16 MB safety limit.".to_string());
    }
    let data: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Google Cloud returned invalid synthesis data: {error}"))?;
    let encoded = data.get("audioContent").and_then(Value::as_str)
        .ok_or_else(|| "Google Cloud returned no audioContent.".to_string())?;
    let audio = STANDARD.decode(encoded)
        .map_err(|_| "Google Cloud returned invalid base64 audio.".to_string())?;
    if audio.is_empty() {
        return Err("Google Cloud returned empty audio.".to_string());
    }
    Ok(STANDARD.encode(audio))
}

async fn tts_translator_synthesize(
    client: &Client, text: &str, voice: &str, rate: f32, pitch: f32, options: &TtsSynthesisOptions,
) -> Result<String, String> {
    let mut attempt = 0;
    loop {
        let token = microsoft_translator_token(client).await?;
        let mut url = azure_base(&token.region)?;
        url.set_path("/cognitiveservices/v1");
        let response = client.post(url)
            .bearer_auth(&token.bearer)
            .header("X-Microsoft-OutputFormat", azure_format(options))
            .header("User-Agent", "Hoshi Reader Desktop")
            .header(CONTENT_TYPE, "application/ssml+xml")
            .body(azure_ssml(text, voice, rate, pitch, options))
            .send().await
            .map_err(|error| format!("Could not connect to Microsoft Translator TTS: {error}"))?;
        if response.status().as_u16() == 401 && attempt == 0 {
            *MICROSOFT_TRANSLATOR_TOKEN.lock().await = None;
            attempt += 1;
            continue;
        }
        return finish_audio(response).await;
    }
}

fn google_translate_body(text: &str, slow: bool) -> String {
    let params = json!([text, "ja", if slow { Value::Bool(true) } else { Value::Null }, "null"]).to_string();
    json!([[["jQ1olc", params, Value::Null, "generic"]]]).to_string()
}

fn google_translate_audio(response: &str) -> Result<Vec<u8>, String> {
    for line in response.lines().filter(|line| line.trim_start().starts_with('[')) {
        let Ok(rows) = serde_json::from_str::<Value>(line) else { continue };
        let Some(rows) = rows.as_array() else { continue };
        for row in rows {
            let Some(values) = row.as_array() else { continue };
            if values.get(1).and_then(Value::as_str) != Some("jQ1olc") { continue; }
            let Some(payload) = values.get(2).and_then(Value::as_str) else { continue };
            let parsed: Value = serde_json::from_str(payload)
                .map_err(|_| "Google Translate returned invalid audio data.".to_string())?;
            let audio = parsed.get(0).and_then(Value::as_str)
                .ok_or_else(|| "Google Translate returned no audio data.".to_string())?;
            return STANDARD.decode(audio).map_err(|_| "Google Translate returned invalid base64 audio.".to_string());
        }
    }
    Err("Google Translate returned an invalid audio response.".to_string())
}

fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn rotate_cs64(value: u32) -> u32 {
    value.rotate_left(16)
}

// Edge's Sec-MS-GEC v2 checksum. Arithmetic intentionally wraps modulo 2^32.
fn edge_cs64(data: &[u8], digest: &[u8]) -> [u8; 8] {
    let key_a = u32::from_le_bytes(digest[0..4].try_into().unwrap()) | 1;
    let key_b = u32::from_le_bytes(digest[4..8].try_into().unwrap()) | 1;
    let mut padded = data.to_vec();
    padded.resize((padded.len() + 7) / 8 * 8, 0);
    let (mut swap_last, mut swap_sum, mut state, mut sum) = (0u32, 0u32, 0u32, 0u32);
    for pair in padded.chunks_exact(8) {
        let x = u32::from_le_bytes(pair[0..4].try_into().unwrap());
        let y = u32::from_le_bytes(pair[4..8].try_into().unwrap());

        let mut q = rotate_cs64(rotate_cs64(x).wrapping_mul(0xEF0569FB).wrapping_add(x.wrapping_mul(key_a)))
            .wrapping_mul(0x76F50F34);
        swap_sum = swap_sum.wrapping_add(rotate_cs64(q).wrapping_mul(0xC3EFEA97).wrapping_add(q));
        q = rotate_cs64(rotate_cs64(y).wrapping_mul(0xC31713DB).wrapping_add(y.wrapping_mul(key_b)))
            .wrapping_mul(0x1881CE3C);
        swap_last = rotate_cs64(q).wrapping_mul(0x35BD1EC9).wrapping_add(q);
        swap_sum = swap_sum.wrapping_add(swap_last);

        state = state.wrapping_add(x).wrapping_mul(key_a);
        state = rotate_cs64(state).wrapping_mul(0xCF98B111);
        state = rotate_cs64(state).wrapping_mul(0x87085B9F);
        state = rotate_cs64(state).wrapping_mul(0x12CEB96D);
        state = rotate_cs64(state).wrapping_mul(0x257E1D83);
        sum = sum.wrapping_add(state);

        let mut next = state.wrapping_add(y).wrapping_mul(key_b);
        next = rotate_cs64(next).wrapping_mul(0xA27416F5);
        next = rotate_cs64(next).wrapping_mul(0xD38396FF);
        state = rotate_cs64(rotate_cs64(next).wrapping_mul(0x7C932B89)).wrapping_mul(0xBFA49F69);
        sum = sum.wrapping_add(state);
    }
    let mut output = [0u8; 8];
    output[0..4].copy_from_slice(&(swap_last ^ state).to_le_bytes());
    output[4..8].copy_from_slice(&(swap_sum ^ sum).to_le_bytes());
    output
}

fn edge_gec(unix_seconds: u64) -> String {
    let filetime_seconds = unix_seconds.saturating_add(11_644_473_600);
    let ticks = (filetime_seconds - filetime_seconds % 300) * 10_000_000;
    let data = format!("Copyright (C) Microsoft Corporation. All rights reserved.{ticks}{EDGE_TOKEN}");
    let digest = Sha256::digest(data.as_bytes());
    format!("{}-{}", hex_upper(&edge_cs64(data.as_bytes(), &digest)), hex_upper(&digest))
}

fn edge_url(path: &str, websocket: bool) -> Result<Url, String> {
    let mut url = Url::parse(&format!("{EDGE_BASE}{path}"))
        .map_err(|_| "Could not prepare the Edge TTS endpoint.".to_string())?;
    if websocket {
        url.set_scheme("wss").map_err(|_| "Could not prepare the Edge TTS websocket.".to_string())?;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "System time is invalid.".to_string())?
        .as_secs();
    url.query_pairs_mut()
        .append_pair("TrustedClientToken", EDGE_TOKEN)
        .append_pair("Sec-MS-GEC", &edge_gec(now))
        .append_pair("Sec-MS-GEC-Version", &format!("2-{EDGE_VERSION}"));
    if websocket {
        url.query_pairs_mut().append_pair("ConnectionId", &uuid::Uuid::new_v4().simple().to_string());
    }
    Ok(url)
}

fn edge_cookie() -> Result<HeaderValue, String> {
    let value = format!("muid={};", uuid::Uuid::new_v4().simple().to_string().to_uppercase());
    HeaderValue::from_str(&value).map_err(|_| "Could not prepare Edge TTS request headers.".to_string())
}

async fn tts_edge_voice_list() -> Result<Vec<TtsVoiceInfo>, String> {
    let url = edge_url("/voices/list", false)?;
    // Mirror Android's default redirect-following HTTP client for the voice catalog.
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|_| "Could not initialize the Edge TTS client.".to_string())?;
    let response = client.get(url)
        .header(reqwest::header::USER_AGENT, EDGE_USER_AGENT)
        .header(reqwest::header::ORIGIN, EDGE_ORIGIN)
        .header(reqwest::header::COOKIE, edge_cookie()?.to_str().map_err(|_| "Invalid Edge request cookie.")?)
        .send().await
        .map_err(|error| format!("Could not connect to Microsoft Edge TTS: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default().chars().take(240).collect::<String>();
        return Err(format!("Microsoft Edge TTS voice list returned HTTP {status}: {detail}"));
    }
    let content_type = response.headers().get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok()).unwrap_or("unknown content type").to_string();
    let status = response.status();
    let bytes = response.bytes().await.map_err(|error| format!("Could not read Edge TTS voices: {error}"))?;
    if bytes.len() > 5 * 1024 * 1024 {
        return Err("Edge TTS voice list response was unexpectedly large.".to_string());
    }
    japanese_microsoft_voices(&bytes, &format!("Microsoft Edge (HTTP {status}, {content_type})"))
}

fn edge_xml(value: &str) -> String {
    value.chars().map(|ch| match ch {
        '&' => "&amp;".to_string(),
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        '"' => "&quot;".to_string(),
        '\'' => "&apos;".to_string(),
        c if c.is_control() && c != '\n' && c != '\r' && c != '\t' => " ".to_string(),
        c => c.to_string(),
    }).collect()
}

fn edge_ssml(text: &str, voice: &str, rate: f32, pitch: f32, options: &TtsSynthesisOptions) -> String {
    let percentage = ((rate.clamp(0.5, 3.0) - 1.0) * 100.0).round() as i32;
    let volume = options.volume(false).round() as i32;
    format!(
        "<speak version=\"1.0\" xmlns=\"http://www.w3.org/2001/10/synthesis\" xml:lang=\"ja-JP\"><voice name=\"{}\"><prosody rate=\"{:+}%\" pitch=\"{:+}Hz\" volume=\"{:+}%\">{}</prosody></voice></speak>",
        edge_xml(voice),
        percentage,
        pitch.clamp(-50.0, 50.0).round() as i32,
        volume,
        edge_xml(text)
    )
}

fn edge_audio_frame(frame: &[u8]) -> Result<&[u8], String> {
    if frame.len() < 2 {
        return Err("Microsoft Edge TTS returned an invalid audio frame.".to_string());
    }
    let header_length = (usize::from(frame[0]) << 8) | usize::from(frame[1]);
    if header_length > frame.len().saturating_sub(2) {
        return Err("Microsoft Edge TTS returned a truncated audio frame.".to_string());
    }
    let headers = std::str::from_utf8(&frame[2..2 + header_length])
        .map_err(|_| "Microsoft Edge TTS returned invalid audio headers.".to_string())?;
    if !headers.lines().any(|line| line.trim().eq_ignore_ascii_case("Path:audio")) {
        return Err("Microsoft Edge TTS returned a non-audio binary frame.".to_string());
    }
    Ok(&frame[2 + header_length..])
}

async fn tts_edge_synthesize(text: &str, voice: &str, rate: f32, pitch: f32, options: &TtsSynthesisOptions) -> Result<Vec<u8>, String> {
    if voice.trim().is_empty() {
        return Err("Choose an Edge Japanese voice first.".to_string());
    }
    let url = edge_url("/edge/v1", true)?;
    let mut request = url.as_str().into_client_request()
        .map_err(|_| "Could not prepare the Edge TTS websocket request.".to_string())?;
    request.headers_mut().insert(USER_AGENT, HeaderValue::from_static(EDGE_USER_AGENT));
    request.headers_mut().insert(ORIGIN, HeaderValue::from_static(EDGE_ORIGIN));
    request.headers_mut().insert(COOKIE, edge_cookie()?);

    let result = tokio::time::timeout(Duration::from_secs(60), async move {
        let (mut socket, _) = connect_async(request).await
            .map_err(|error| format!("Could not connect to the Microsoft Edge TTS websocket: {error}"))?;
        let now = Utc::now();
        const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        let timestamp = format!(
            "{} {} {:02} {} {:02}:{:02}:{:02} GMT+0000 (Coordinated Universal Time)",
            DAYS[now.weekday().num_days_from_monday() as usize],
            MONTHS[now.month0() as usize],
            now.day(), now.year(), now.hour(), now.minute(), now.second()
        );
        let config = format!(
            "X-Timestamp:{timestamp}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n{{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}}}}\r\n"
        );
        let speech = format!(
            "X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{timestamp}Z\r\nPath:ssml\r\n\r\n{}",
            uuid::Uuid::new_v4().simple(),
            edge_ssml(text, voice, rate, pitch, options)
        );
        socket.send(Message::Text(config.into())).await
            .map_err(|_| "Could not send the Edge TTS audio configuration.".to_string())?;
        socket.send(Message::Text(speech.into())).await
            .map_err(|_| "Could not send speech text to Edge TTS.".to_string())?;

        let mut audio = Vec::new();
        while let Some(message) = socket.next().await {
            match message.map_err(|_| "The Edge TTS connection ended unexpectedly.".to_string())? {
                Message::Binary(frame) => {
                    let payload = edge_audio_frame(frame.as_ref())?;
                    if audio.len().saturating_add(payload.len()) > EDGE_MAX_AUDIO_BYTES {
                        return Err("Edge TTS audio exceeds the 16 MB safety limit.".to_string());
                    }
                    audio.extend_from_slice(payload);
                }
                Message::Text(frame) => {
                    let headers = frame.split("\r\n\r\n").next().unwrap_or_default();
                    if headers.lines().any(|line| line.trim().eq_ignore_ascii_case("Path:turn.end")) {
                        if audio.is_empty() {
                            return Err("Edge TTS completed without returning audio.".to_string());
                        }
                        let _ = socket.close(None).await;
                        return Ok(audio);
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
        Err("Edge TTS closed the connection before completing speech synthesis.".to_string())
    }).await.map_err(|_| "Edge TTS timed out while generating speech.".to_string())?;
    result
}


#[cfg(windows)]
struct WindowsSpeechApartment;

#[cfg(windows)]
impl WindowsSpeechApartment {
    fn new() -> Result<Self, String> {
        unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }
            .map_err(|error| format!("Could not initialize the Windows speech apartment: {error}"))?;
        Ok(Self)
    }
}

#[cfg(windows)]
impl Drop for WindowsSpeechApartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() };
    }
}

#[cfg(windows)]
fn windows_system_voice_list() -> Result<Vec<SystemTtsVoiceInfo>, String> {
    let _apartment = WindowsSpeechApartment::new()?;
    let voices = SpeechSynthesizer::AllVoices()
        .map_err(|error| format!("Windows could not enumerate installed speech voices: {error}"))?;
    let default_id = SpeechSynthesizer::DefaultVoice()
        .ok()
        .and_then(|voice| voice.Id().ok())
        .map(|id| id.to_string());
    let count = voices.Size()
        .map_err(|error| format!("Could not read the Windows speech voice list: {error}"))?;
    let mut result = Vec::new();
    for index in 0..count {
        let voice = voices.GetAt(index)
            .map_err(|error| format!("Could not read Windows speech voice {index}: {error}"))?;
        let id = voice.Id()
            .map_err(|error| format!("Could not read a Windows speech voice ID: {error}"))?
            .to_string();
        let name = voice.DisplayName()
            .map_err(|error| format!("Could not read the name of Windows speech voice {id}: {error}"))?
            .to_string();
        let language = voice.Language()
            .map_err(|error| format!("Could not read the language of Windows speech voice {id}: {error}"))?
            .to_string();
        result.push(SystemTtsVoiceInfo {
            is_default: default_id.as_deref() == Some(id.as_str()),
            id,
            name,
            language,
        });
    }
    result.sort_by(|a, b| {
        let a_japanese = !a.language.to_ascii_lowercase().starts_with("ja");
        let b_japanese = !b.language.to_ascii_lowercase().starts_with("ja");
        a_japanese.cmp(&b_japanese).then_with(|| a.name.cmp(&b.name))
    });
    Ok(result)
}

#[cfg(not(windows))]
fn windows_system_voice_list() -> Result<Vec<SystemTtsVoiceInfo>, String> {
    Err("Native Windows speech voices are only available on Windows.".to_string())
}

#[tauri::command]
pub async fn tts_system_voices() -> Result<Vec<SystemTtsVoiceInfo>, String> {
    tokio::task::spawn_blocking(windows_system_voice_list)
        .await
        .map_err(|error| format!("Windows speech voice enumeration failed: {error}"))?
}

#[cfg(windows)]
fn wait_for_speech_operation<T>(
    status: impl Fn() -> windows::core::Result<AsyncStatus>,
    result: impl FnOnce() -> windows::core::Result<T>,
    label: &str,
) -> Result<T, String> {
    let start = std::time::Instant::now();
    loop {
        let current = status()
            .map_err(|error| format!("{label} failed: {error}"))?;
        match current {
            AsyncStatus::Completed => {
                return result().map_err(|error| format!("{label} failed: {error}"));
            }
            AsyncStatus::Error | AsyncStatus::Canceled => {
                return Err(format!("{label} was cancelled or failed."));
            }
            AsyncStatus::Started => {
                if start.elapsed() > Duration::from_secs(60) {
                    return Err(format!("{label} timed out after 60 seconds."));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => {
                if start.elapsed() > Duration::from_secs(60) {
                    return Err(format!("{label} timed out after 60 seconds."));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

#[cfg(windows)]
fn windows_system_synthesize(text: &str, voice_id: &str, rate: f32, pitch: f32) -> Result<Vec<u8>, String> {
    let _apartment = WindowsSpeechApartment::new()?;
    let voices = SpeechSynthesizer::AllVoices()
        .map_err(|error| format!("Windows could not enumerate installed speech voices: {error}"))?;
    let count = voices.Size()
        .map_err(|error| format!("Could not read the Windows speech voice list: {error}"))?;

    let mut selected = None;
    let mut first_japanese = None;
    let mut default_japanese = None;
    let default_id = SpeechSynthesizer::DefaultVoice()
        .ok()
        .and_then(|voice| voice.Id().ok())
        .map(|id| id.to_string());

    for index in 0..count {
        let voice = voices.GetAt(index)
            .map_err(|error| format!("Could not read Windows speech voice {index}: {error}"))?;
        let id = voice.Id()
            .map_err(|error| format!("Could not read a Windows speech voice ID: {error}"))?
            .to_string();
        let language = voice.Language()
            .map_err(|error| format!("Could not read the language of Windows speech voice {id}: {error}"))?
            .to_string();
        if !language.trim().to_ascii_lowercase().starts_with("ja") {
            continue;
        }
        if !voice_id.trim().is_empty() && id == voice_id {
            selected = Some(voice.clone());
            break;
        }
        if first_japanese.is_none() {
            first_japanese = Some(voice.clone());
        }
        if default_id.as_deref() == Some(id.as_str()) {
            default_japanese = Some(voice.clone());
        }
    }

    let voice = selected.or(default_japanese).or(first_japanese)
        .ok_or_else(|| "Windows native speech synthesis has no Japanese voice installed. Install the Japanese Speech language feature in Windows Language options, then refresh the TTS voice list.".to_string())?;
    let synthesizer = SpeechSynthesizer::new()
        .map_err(|error| format!("Could not create the Windows speech synthesizer: {error}"))?;
    synthesizer.SetVoice(&voice)
        .map_err(|error| format!("Could not select the Windows speech voice: {error}"))?;

    let normalized_rate = (if rate.is_finite() { rate } else { 1.0 }).clamp(0.5, 3.0);
    let normalized_pitch = (if pitch.is_finite() { pitch } else { 1.0 }).clamp(0.5, 2.0);
    let rate_percent = ((normalized_rate - 1.0) * 100.0).round() as i32;
    let pitch_percent = ((normalized_pitch - 1.0) * 100.0).round() as i32;
    let ssml = format!(
        "<speak version=\"1.0\" xmlns=\"http://www.w3.org/2001/10/synthesis\" xml:lang=\"ja-JP\"><prosody rate=\"{rate_percent:+}%\" pitch=\"{pitch_percent:+}%\">{}</prosody></speak>",
        edge_xml(text)
    );
    let operation = synthesizer.SynthesizeSsmlToStreamAsync(&HSTRING::from(ssml))
        .map_err(|error| format!("Windows could not synthesize the text: {error}"))?;
    let stream = wait_for_speech_operation(
        || operation.Status(),
        || operation.GetResults(),
        "Windows speech synthesis",
    )?;

    let size = stream.Size()
        .map_err(|error| format!("Could not read generated Windows speech audio: {error}"))?;
    if size == 0 || size > EDGE_MAX_AUDIO_BYTES as u64 {
        return Err("Windows speech audio was empty or exceeded the 16 MB limit.".to_string());
    }
    let input = stream.GetInputStreamAt(0)
        .map_err(|error| format!("Could not open generated Windows speech audio: {error}"))?;
    let reader = DataReader::CreateDataReader(&input)
        .map_err(|error| format!("Could not read generated Windows speech audio: {error}"))?;
    let load = reader.LoadAsync(size as u32)
        .map_err(|error| format!("Could not load generated Windows speech audio: {error}"))?;
    wait_for_speech_operation(
        || load.Status(),
        || load.GetResults(),
        "Reading Windows speech audio",
    )?;
    let mut bytes = vec![0; size as usize];
    reader.ReadBytes(&mut bytes)
        .map_err(|error| format!("Could not copy generated Windows speech audio: {error}"))?;
    Ok(bytes)
}

#[cfg(not(windows))]
async fn tts_system_synthesize(_text: String, _voice_id: String, _rate: f32, _pitch: f32) -> Result<String, String> {
    Err("Native Windows speech synthesis is only available on Windows.".to_string())
}

#[cfg(windows)]
async fn tts_system_synthesize(text: String, voice_id: String, rate: f32, pitch: f32) -> Result<String, String> {
    let bytes = tokio::task::spawn_blocking(move || windows_system_synthesize(&text, &voice_id, rate, pitch))
        .await
        .map_err(|error| format!("Windows speech synthesis worker failed: {error}"))??;
    Ok(STANDARD.encode(bytes))
}

#[tauri::command]
pub async fn tts_edge_voices() -> Result<Vec<TtsVoiceInfo>, String> {
    tts_edge_voice_list().await
}

#[tauri::command]
pub async fn tts_azure_voices(region: String, api_key: String) -> Result<Vec<TtsVoiceInfo>, String> {
    tts_azure_voice_list(&region, &api_key).await
}

#[tauri::command]
pub async fn tts_microsoft_translator_voices() -> Result<Vec<TtsVoiceInfo>, String> {
    tts_translator_voice_list().await
}

#[tauri::command]
pub async fn tts_google_cloud_voices(api_key: String) -> Result<Vec<TtsVoiceInfo>, String> {
    tts_google_cloud_voice_list(&api_key).await
}

#[cfg(test)]
mod edge_token_tests {
    use super::edge_gec;

    #[test]
    fn ruby_readings_use_all_rt_elements_in_a_ruby() {
        let html = "<html><body><p><ruby><rb>漢</rb><rt>かん</rt><rb>字</rb><rt>じ</rt></ruby>を読む。</p></body></html>";
        assert_eq!(super::speakable_text(html, true), "かんじを読む。");
        assert_eq!(super::speakable_text(html, false), "漢字を読む。");
    }

    #[test]
    fn ruby_readings_strip_nested_markup_inside_rt() {
        let html = "<html><body><p><ruby><rb>今日</rb><rt><span>きょう</span></rt></ruby>です。</p></body></html>";
        assert_eq!(super::speakable_text(html, true), "きょうです。");
    }

    #[test]
    fn ruby_ranges_preserve_source_text_offsets_and_readings() {
        let html = "<html><body><p>今日は<ruby>晴天<rp>(</rp><rt>せいてん</rt><rp>)</rp></ruby>です。</p></body></html>";
        let (text, _, ranges) = super::speakable_text_with_images(html);
        assert_eq!(text, "今日は晴天です。");
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 3);
        assert_eq!(ranges[0].end, 5);
        assert_eq!(ranges[0].reading, "せいてん");
    }

    #[test]
    fn image_backed_ruby_produces_zero_width_range_and_image_offset() {
        let html = "<html><body><p>前<ruby><img src='glyph.png'/><rt>たず</rt></ruby>ねた。</p></body></html>";
        let (text, images, ranges) = super::speakable_text_with_images(html);
        assert_eq!(text, "前ねた。");
        assert_eq!(images, vec![1]);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].start, 1);
        assert_eq!(ranges[0].end, 1);
        assert_eq!(ranges[0].reading, "たず");
    }

    #[test]
    fn edge_gec_matches_upstream_vectors() {
        let expected = "492D7694A183A45E-56E821F95D4579C5B7C7E33DC7303FF379EC36D510213555BBBB9EB17EF15876";
        assert_eq!(edge_gec(1_791_888_000), expected);
        assert_eq!(edge_gec(1_791_888_299), expected);
        assert_eq!(edge_gec(1_791_888_300),
            "6D26B5F5B70269AD-280E82E1BBE24E4E8866E3502EB6277B9436F61ABDC1C224518A038BBD7BBDAD");
    }
}

#[tauri::command]
pub async fn tts_synthesize(
    engine: String,
    text: String,
    base_url: String,
    api_key: String,
    voice: String,
    rate: f32,
    pitch: f32,
    options: TtsSynthesisOptions,
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("There is no text to synthesize.".to_string());
    }
    let rate = if rate.is_finite() { rate.clamp(0.25, 4.0) } else { 1.0 };
    let client = request_client()?;

    match engine.as_str() {
        "system" => tts_system_synthesize(text, voice, rate, pitch).await,
        "edge" => {
            let bytes = tts_edge_synthesize(&text, &voice, rate, pitch, &options).await?;
            Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
        }
        "googleTranslate" => {
            let request_body = google_translate_body(&text, rate < 1.0);
            let response = client
                .post("https://translate.google.com/_/TranslateWebserverUi/data/batchexecute?rpcids=jQ1olc")
                .header("User-Agent", "Mozilla/5.0")
                .header("Referer", "https://translate.google.com/")
                .form(&[("f.req", request_body)])
                .send().await
                .map_err(|error| format!("Could not connect to Google Translate TTS: {error}"))?;
            if !response.status().is_success() {
                let status = response.status();
                let detail = response.text().await.unwrap_or_default().chars().take(240).collect::<String>();
                return Err(format!("Google Translate TTS returned HTTP {status}: {detail}"));
            }
            let response_text = response.text().await
                .map_err(|error| format!("Could not read Google Translate audio response: {error}"))?;
            let audio = google_translate_audio(&response_text)?;
            if audio.is_empty() || audio.len() > EDGE_MAX_AUDIO_BYTES {
                return Err("Google Translate returned empty or oversized audio.".to_string());
            }
            Ok(STANDARD.encode(audio))
        }
        "azure" => tts_azure_synthesize(&client, &base_url, &api_key, &text, &voice, rate, pitch, &options).await,
        "microsoftTranslator" => tts_translator_synthesize(&client, &text, &voice, rate, pitch, &options).await,
        "googleCloud" => tts_google_cloud_synthesize(&client, &api_key, &text, &voice, rate, pitch, &options).await,
        _ => Err("Unsupported TTS engine.".to_string()),
    }
}
