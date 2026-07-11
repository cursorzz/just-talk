use std::io::{Read, Write};

use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use serde::Deserialize;

const FULL_CLIENT_REQUEST: u8 = 0x1;
const AUDIO_ONLY_REQUEST: u8 = 0x2;
const FULL_SERVER_RESPONSE: u8 = 0x9;
const SERVER_ERROR_RESPONSE: u8 = 0xf;

#[derive(Debug)]
pub enum ServerMessage {
    Response { flags: u8, json: String },
    Error(String),
    Other,
}

fn header(message_type: u8, flags: u8, serialization: u8, compression: u8) -> [u8; 4] {
    [
        0x11,
        (message_type << 4) | flags,
        (serialization << 4) | compression,
        0,
    ]
}

fn gzip(data: &[u8], enabled: bool) -> Result<Vec<u8>, String> {
    if !enabled {
        return Ok(data.to_vec());
    }
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}

pub fn full_request(json: &str, use_gzip: bool) -> Result<Vec<u8>, String> {
    let payload = gzip(json.as_bytes(), use_gzip)?;
    let mut result = header(FULL_CLIENT_REQUEST, 0, 1, u8::from(use_gzip)).to_vec();
    result.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    result.extend_from_slice(&payload);
    Ok(result)
}

pub fn audio_request(pcm: &[u8], last: bool, use_gzip: bool) -> Result<Vec<u8>, String> {
    let payload = gzip(pcm, use_gzip)?;
    let flags = if last { 0b0010 } else { 0b0000 };
    let mut result = header(AUDIO_ONLY_REQUEST, flags, 0, u8::from(use_gzip)).to_vec();
    result.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    result.extend_from_slice(&payload);
    Ok(result)
}

pub fn parse(data: &[u8]) -> Result<ServerMessage, String> {
    if data.len() < 4 {
        return Err("服务端消息过短".into());
    }
    let header_words = (data[0] & 0x0f) as usize;
    let offset = header_words * 4;
    if data.len() < offset + 4 {
        return Err("服务端消息头无效".into());
    }
    let message_type = data[1] >> 4;
    let flags = data[1] & 0x0f;
    let compression = data[2] & 0x0f;
    let mut cursor = offset;
    if matches!(message_type, FULL_SERVER_RESPONSE | SERVER_ERROR_RESPONSE) {
        cursor += 4;
    }
    if data.len() < cursor + 4 {
        return Err("服务端消息缺少载荷长度".into());
    }
    let size = u32::from_be_bytes(data[cursor..cursor + 4].try_into().unwrap()) as usize;
    cursor += 4;
    if data.len() < cursor + size {
        return Err("服务端消息载荷不完整".into());
    }
    let mut payload = data[cursor..cursor + size].to_vec();
    if compression == 1 {
        let mut decoded = Vec::new();
        GzDecoder::new(payload.as_slice())
            .read_to_end(&mut decoded)
            .map_err(|e| e.to_string())?;
        payload = decoded;
    }
    let text = String::from_utf8_lossy(&payload).into_owned();
    Ok(match message_type {
        FULL_SERVER_RESPONSE => ServerMessage::Response { flags, json: text },
        SERVER_ERROR_RESPONSE => ServerMessage::Error(text),
        _ => ServerMessage::Other,
    })
}

#[derive(Deserialize)]
struct RecognitionEnvelope {
    result: Option<RecognitionResult>,
}
#[derive(Deserialize)]
struct RecognitionResult {
    text: Option<String>,
    utterances: Option<Vec<Utterance>>,
}
#[derive(Deserialize)]
struct Utterance {
    text: Option<String>,
    definite: Option<bool>,
    end_time: Option<i64>,
}

pub fn recognition_text(json: &str, last_end_time: &mut i64) -> Result<(String, String), String> {
    let envelope: RecognitionEnvelope =
        serde_json::from_str(json).map_err(|e| format!("识别响应格式错误：{e}"))?;
    let Some(result) = envelope.result else {
        return Ok((String::new(), String::new()));
    };
    if let Some(items) = result.utterances {
        let mut committed = String::new();
        let mut partial = String::new();
        for item in items {
            let text = item.text.unwrap_or_default().trim().to_owned();
            if text.is_empty() {
                continue;
            }
            if item.definite.unwrap_or(false) {
                let end = item.end_time.unwrap_or_default();
                if end > *last_end_time {
                    committed.push_str(&text);
                    *last_end_time = end;
                }
            } else {
                partial = text;
            }
        }
        Ok((committed, partial))
    } else {
        Ok((String::new(), result.text.unwrap_or_default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_audio_last_frame() {
        let frame = audio_request(&[1, 2], true, false).unwrap();
        assert_eq!(frame, vec![0x11, 0x22, 0x00, 0x00, 0, 0, 0, 2, 1, 2]);
        assert_eq!(
            u32::from_be_bytes(frame[4..8].try_into().unwrap()),
            frame[8..].len() as u32
        );
    }

    #[test]
    fn extracts_only_new_definite_utterances() {
        let json = r#"{"result":{"utterances":[{"text":"你好","definite":true,"end_time":10},{"text":"世界","definite":false,"end_time":20}]}}"#;
        let mut end = -1;
        let (committed, partial) = recognition_text(json, &mut end).unwrap();
        assert_eq!(committed, "你好");
        assert_eq!(partial, "世界");
    }
}
