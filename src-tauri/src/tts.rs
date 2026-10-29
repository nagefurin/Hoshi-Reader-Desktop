use std::borrow::Cow;
use std::net::IpAddr;
use std::sync::LazyLock;
use std::time::Duration;

use base64::Engine;
use chrono::{Datelike, Timelike, Utc};
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        http::header::{CACHE_CONTROL, COOKIE, ORIGIN, PRAGMA, USER_AGENT},
        http::HeaderValue,
        protocol::Message,
    },
};
use regex::{Captures, Regex};
use rbook::Epub;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::AppHandle;

use crate::{book, library};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsChapter {
    pub spine_index: usize,
    pub title: String,
    pub text: String,
    pub ruby_text: String,
    pub image_offsets: Vec<usize>,
}

static RE_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)</?(?:p|div|h[1-6]|li|ul|ol|blockquote|section|article|table|tr|td|th|br|hr|figure|figcaption)\b[^>]*>").unwrap()
});
static RE_RUBY_READING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<ruby\b[^>]*>.*?<rt\b[^>]*>(.*?)</rt>.*?</ruby>").unwrap()
});
static RE_NUMERIC_ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&#(x[0-9a-fA-F]+|[0-9]+);").unwrap());
static RE_IMAGE_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<(?:img|image)\b[^>]*>").unwrap());
const IMAGE_MARKER: char = '\u{E000}';

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
        RE_RUBY_READING.replace_all(body, "$1")
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

fn speakable_text_with_images(html: &str) -> (String, Vec<usize>) {
    let body = library::body(html);
    let marked = RE_IMAGE_TAG.replace_all(body, IMAGE_MARKER.to_string());
    let with_breaks = RE_BLOCK.replace_all(&marked, "\n");
    let decoded = decode_entities(&with_breaks);
    let plain = library::strip(&decoded);
    let formatted = plain
        .split('\\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut text = String::with_capacity(formatted.len());
    let mut image_offsets = Vec::new();
    let mut character_count = 0usize;
    for character in formatted.chars() {
        if character == IMAGE_MARKER {
            image_offsets.push(character_count);
        } else {
            text.push(character);
            character_count += 1;
        }
    }
    (text, image_offsets)
}

fn local_or_private_host(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            address.is_loopback() || address.is_private() || address.is_link_local() || address.is_unspecified()
        }
        Ok(IpAddr::V6(address)) => address.is_loopback() || address.is_unspecified() || address.is_unique_local() || address.is_unicast_link_local(),
        Err(_) => false,
    }
}

fn validate_endpoint(value: &str) -> Result<Url, String> {
    let url = Url::parse(value.trim()).map_err(|_| "Enter a valid TTS endpoint URL.".to_string())?;
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("The TTS endpoint URL cannot contain credentials, a query, or a fragment.".to_string());
    }
    match url.scheme() {
        "https" => {}
        "http" if local_or_private_host(url.host_str().unwrap_or_default()) => {}
        _ => return Err("Remote TTS endpoints must use HTTPS. HTTP is allowed only for localhost or a private-network host.".to_string()),
    }
    Ok(url)
}

fn endpoint_url(base: Url, suffix: &str) -> Url {
    let current_path = base.path().trim_end_matches('/');
    let suffix = suffix.trim_start_matches('/');
    let path = if current_path.ends_with(suffix) {
        current_path.to_string()
    } else if current_path.is_empty() {
        format!("/{suffix}")
    } else {
        format!("{current_path}/{suffix}")
    };
    let mut url = base;
    url.set_path(&path);
    url
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

    for entry in epub.spine().iter() {
        let Some(item) = entry.manifest_entry() else {
            continue;
        };
        let spine_index = compact_spine_index;
        compact_spine_index += 1;

        if !entry.is_linear() || item.properties().as_str().contains("nav") {
            continue;
        }
        let Ok(content) = item.read_str() else {
            continue;
        };
        let (text, image_offsets) = speakable_text_with_images(&content);
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
        chapters.push(TtsChapter { spine_index, title, text, ruby_text, image_offsets });
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

#[derive(Deserialize)]
struct EdgeVoice {
    #[serde(rename = "Locale")]
    locale: String,
    #[serde(rename = "ShortName")]
    short_name: String,
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

async fn tts_edge_voice_list() -> Result<Vec<String>, String> {
    let url = edge_url("/voices/list", false)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not initialize the Edge TTS client.".to_string())?;
    let response = client.get(url)
        .header(reqwest::header::USER_AGENT, EDGE_USER_AGENT)
        .header(reqwest::header::ORIGIN, EDGE_ORIGIN)
        .header(reqwest::header::COOKIE, edge_cookie()?.to_str().map_err(|_| "Invalid Edge request cookie.")?)
        .send().await
        .map_err(|_| "Could not connect to Microsoft Edge TTS.".to_string())?;
    if !response.status().is_success() {
        return Err(format!("Microsoft Edge TTS voice list returned HTTP {}.", response.status()));
    }
    let content_type = response.headers().get(CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or("unknown content type").to_string();
    let status = response.status();
    let bytes = response.bytes().await.map_err(|_| "Could not read Edge TTS voices.".to_string())?;
    if bytes.len() > 5 * 1024 * 1024 {
        return Err("Edge TTS voice list response was unexpectedly large.".to_string());
    }
    let values: Vec<EdgeVoice> = serde_json::from_slice(&bytes).map_err(|error| {
        let preview = String::from_utf8_lossy(&bytes).chars().take(240).collect::<String>();
        format!("Microsoft Edge TTS returned an invalid voice list (HTTP {status}, {content_type}): {error}. Response: {preview}")
    })?;
    let mut voices: Vec<String> = values.into_iter()
        .filter(|voice| voice.locale.eq_ignore_ascii_case("ja-JP"))
        .map(|voice| voice.short_name)
        .filter(|voice| !voice.trim().is_empty())
        .collect();
    voices.sort();
    voices.dedup();
    Ok(voices)
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

fn edge_ssml(text: &str, voice: &str, rate: f32, pitch: f32) -> String {
    let percentage = ((rate.clamp(0.5, 3.0) - 1.0) * 100.0).round() as i32;
    format!(
        "<speak version=\"1.0\" xmlns=\"http://www.w3.org/2001/10/synthesis\" xml:lang=\"ja-JP\"><voice name=\"{}\"><prosody rate=\"{:+}%\" pitch=\"{:+}Hz\" volume=\"+0%\">{}</prosody></voice></speak>",
        edge_xml(voice),
        percentage,
        pitch.clamp(-50.0, 50.0).round() as i32,
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

async fn tts_edge_synthesize(text: &str, voice: &str, rate: f32, pitch: f32) -> Result<Vec<u8>, String> {
    if voice.trim().is_empty() {
        return Err("Choose an Edge Japanese voice first.".to_string());
    }
    let url = edge_url("/edge/v1", true)?;
    let mut request = url.as_str().into_client_request()
        .map_err(|_| "Could not prepare the Edge TTS websocket request.".to_string())?;
    request.headers_mut().insert(USER_AGENT, HeaderValue::from_static(EDGE_USER_AGENT));
    request.headers_mut().insert(ORIGIN, HeaderValue::from_static(EDGE_ORIGIN));
    request.headers_mut().insert(COOKIE, edge_cookie()?);
    request.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    request.headers_mut().insert(PRAGMA, HeaderValue::from_static("no-cache"));

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
            edge_ssml(text, voice, rate, pitch)
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

#[tauri::command]
pub async fn tts_edge_voices() -> Result<Vec<String>, String> {
    tts_edge_voice_list().await
}

#[cfg(test)]
mod edge_token_tests {
    use super::edge_gec;

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
    model: String,
    voice: String,
    rate: f32,
    pitch: f32,
    vits_model: String,
    speaker_id: u32,
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("There is no text to synthesize.".to_string());
    }
    let rate = if rate.is_finite() { rate.clamp(0.25, 4.0) } else { 1.0 };
    let client = request_client()?;

    match engine.as_str() {
        "edge" => {
            let bytes = tts_edge_synthesize(&text, &voice, rate, pitch).await?;
            Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
        }
        "googleTranslate" => {
            if text.chars().count() > 190 {
                return Err("Google Translate TTS accepts at most 190 characters per request.".to_string());
            }
            let mut url = Url::parse("https://translate.google.com/translate_tts").unwrap();
            url.query_pairs_mut()
                .append_pair("ie", "UTF-8")
                .append_pair("client", "tw-ob")
                .append_pair("tl", "ja")
                .append_pair("q", &text)
                .append_pair("ttsspeed", if rate < 0.75 { "0.24" } else { "1.0" });
            let response = client
                .get(url)
                .header("User-Agent", "Mozilla/5.0 Hoshi-Reader-Desktop")
                .header("Referer", "https://translate.google.com/")
                .send()
                .await
                .map_err(|_| "Could not connect to Google Translate TTS.".to_string())?;
            finish_audio(response).await
        }
        "openAi" => {
            let base = validate_endpoint(&base_url)?;
            if model.trim().is_empty() || voice.trim().is_empty() {
                return Err("OpenAI-compatible TTS requires a model and voice.".to_string());
            }
            let endpoint = endpoint_url(base, "audio/speech");
            let mut request = client.post(endpoint).json(&json!({
                "model": model.trim(),
                "input": text,
                "voice": voice.trim(),
                "speed": rate,
                "response_format": "mp3"
            }));
            if !api_key.trim().is_empty() {
                request = request.bearer_auth(api_key.trim());
            }
            let response = request
                .header("Accept", "audio/mpeg, audio/*")
                .send()
                .await
                .map_err(|_| "Could not connect to the configured OpenAI-compatible TTS endpoint.".to_string())?;
            finish_audio(response).await
        }
        "vits" => {
            let base = validate_endpoint(&base_url)?;
            let model = match vits_model.as_str() {
                "vits" | "w2v2-vits" | "bert-vits2" | "gpt-sovits" => vits_model.as_str(),
                _ => return Err("Choose a supported VITS model.".to_string()),
            };
            let endpoint = endpoint_url(base, &format!("voice/{model}"));
            let mut request = client.post(endpoint).json(&json!({
                "text": text,
                "id": speaker_id,
                "lang": "ja",
                "format": "mp3",
                "streaming": false,
                "length": 1.0 / rate
            }));
            if !api_key.trim().is_empty() {
                request = request.header("X-API-KEY", api_key.trim());
            }
            let response = request
                .header("Accept", "audio/mpeg, audio/*")
                .send()
                .await
                .map_err(|_| "Could not connect to the configured VITS endpoint.".to_string())?;
            finish_audio(response).await
        }
        _ => Err("Unsupported TTS engine.".to_string()),
    }
}
