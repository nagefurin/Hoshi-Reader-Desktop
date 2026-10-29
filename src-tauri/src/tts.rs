use std::borrow::Cow;
use std::net::IpAddr;
use std::sync::LazyLock;
use std::time::Duration;

use base64::Engine;
use regex::{Captures, Regex};
use rbook::Epub;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, Url};
use serde::Serialize;
use serde_json::json;
use tauri::AppHandle;

use crate::{book, library};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsChapter {
    pub spine_index: usize,
    pub title: String,
    pub text: String,
}

static RE_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)</?(?:p|div|h[1-6]|li|ul|ol|blockquote|section|article|table|tr|td|th|br|hr|figure|figcaption)\b[^>]*>").unwrap()
});
static RE_RUBY_READING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<ruby\b[^>]*>.*?<rt\b[^>]*>(.*?)</rt>.*?</ruby>").unwrap()
});
static RE_NUMERIC_ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&#(x[0-9a-fA-F]+|[0-9]+);").unwrap());

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
pub async fn tts_load_text(app: AppHandle, id: String, use_ruby_readings: bool) -> Result<Vec<TtsChapter>, String> {
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
        let text = speakable_text(&content, use_ruby_readings);
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
        chapters.push(TtsChapter { spine_index, title, text });
    }

    if chapters.is_empty() {
        return Err("No readable text was found in this EPUB.".to_string());
    }
    Ok(chapters)
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
    vits_model: String,
    speaker_id: u32,
) -> Result<String, String> {
    if text.trim().is_empty() {
        return Err("There is no text to synthesize.".to_string());
    }
    let rate = if rate.is_finite() { rate.clamp(0.25, 4.0) } else { 1.0 };
    let client = request_client()?;

    match engine.as_str() {
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
