//! Assist pipeline runs over the websocket: the requests, the events they produce, and the
//! binary audio frames.

use serde_json::{Value, json};

/// What the UI shows and plays for a pipeline run.
#[derive(Debug, Clone, PartialEq)]
pub enum VoiceEvent {
    /// The wake word was heard (always-listen mode).
    Woke,
    Listening,
    Heard(String),
    Thinking,
    Reply(String),
    Speaking,
    Done,
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunEvent {
    /// The run started; audio goes to this binary handler.
    Started {
        handler: Option<u8>,
    },
    WakeWordDetected,
    /// HA's VAD heard the end of the command: stop sending audio.
    SpeechEnded,
    Transcribed(String),
    Replied {
        speech: String,
        conversation_id: Option<String>,
    },
    Spoken {
        url: String,
    },
    Finished,
    Failed(String),
    Other,
}

pub const SAMPLE_RATE: u32 = 16_000;

/// `start_stage` for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    WakeWord,
    Speech,
    Text,
}

/// An assistant ("pipeline") set up in HA: its agent, speech-to-text and voice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assistant {
    pub id: String,
    pub name: String,
}

/// The result of `assist_pipeline/pipeline/list`: every assistant, and HA's preferred one.
pub fn parse_assistants(result: &Value) -> (Vec<Assistant>, Option<String>) {
    let assistants = result["pipelines"]
        .as_array()
        .map(|pipelines| {
            pipelines
                .iter()
                .filter_map(|pipeline| {
                    Some(Assistant {
                        id: pipeline["id"].as_str()?.to_owned(),
                        name: pipeline["name"].as_str()?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let preferred = result["preferred_pipeline"].as_str().map(str::to_owned);
    (assistants, preferred)
}

/// `pipeline`: an assistant id, or `None` for HA's preferred assistant.
pub fn run_request(
    start: Start,
    text: Option<&str>,
    pipeline: Option<&str>,
    conversation_id: Option<&str>,
) -> Value {
    // Typed commands only need the reply text; spoken ones get spoken back.
    let (stage, end, input) = match start {
        // No silence timeout: wait for the wake word as long as the run lasts.
        Start::WakeWord => (
            "wake_word",
            "tts",
            json!({"sample_rate": SAMPLE_RATE, "timeout": 0}),
        ),
        Start::Speech => ("stt", "tts", json!({"sample_rate": SAMPLE_RATE})),
        Start::Text => (
            "intent",
            "intent",
            json!({"text": text.unwrap_or_default()}),
        ),
    };
    let mut request = json!({
        "type": "assist_pipeline/run",
        "start_stage": stage,
        "end_stage": end,
        "input": input,
    });
    if start == Start::WakeWord {
        // HA ends a run after 5 minutes by default; the app starts a new one when it ends.
        request["timeout"] = 3600.into();
    }
    if let Some(id) = pipeline {
        request["pipeline"] = id.into();
    }
    if let Some(id) = conversation_id {
        request["conversation_id"] = id.into();
    }
    request
}

/// A websocket `event` payload from a pipeline run.
pub fn parse_event(event: &Value) -> RunEvent {
    let data = &event["data"];
    match event["type"].as_str().unwrap_or_default() {
        "run-start" => RunEvent::Started {
            handler: data["runner_data"]["stt_binary_handler_id"]
                .as_u64()
                .and_then(|id| u8::try_from(id).ok()),
        },
        "wake_word-end" => {
            if data["wake_word_output"]
                .as_object()
                .is_some_and(|output| !output.is_empty())
            {
                RunEvent::WakeWordDetected
            } else {
                RunEvent::Other
            }
        }
        "stt-vad-end" => RunEvent::SpeechEnded,
        "stt-end" => RunEvent::Transcribed(
            data["stt_output"]["text"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_owned(),
        ),
        "intent-end" => {
            let output = &data["intent_output"];
            RunEvent::Replied {
                speech: output["response"]["speech"]["plain"]["speech"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                conversation_id: output["conversation_id"].as_str().map(str::to_owned),
            }
        }
        "tts-end" => match data["tts_output"]["url"].as_str() {
            Some(url) => RunEvent::Spoken {
                url: url.to_owned(),
            },
            None => RunEvent::Other,
        },
        "run-end" => RunEvent::Finished,
        "error" => RunEvent::Failed(describe_error(
            data["code"].as_str().unwrap_or_default(),
            data["message"].as_str().unwrap_or("Assist failed"),
        )),
        _ => RunEvent::Other,
    }
}

fn describe_error(code: &str, message: &str) -> String {
    match code {
        "stt-no-text-recognized" => "Didn't catch that.".to_owned(),
        "wake-engine-missing" | "wake-provider-missing" => {
            "No wake word engine is set up in Home Assistant.".to_owned()
        }
        "stt-provider-missing" => "No speech-to-text is set up in Home Assistant.".to_owned(),
        _ => message.to_owned(),
    }
}

/// One binary websocket message: the handler id, then raw PCM. An empty `pcm` ends the audio.
pub fn frame(handler: u8, pcm: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(pcm.len() + 1);
    frame.push(handler);
    frame.extend_from_slice(pcm);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_voice_run() {
        let events = [
            json!({"type": "run-start", "data": {"pipeline": "p", "language": "en",
                "runner_data": {"stt_binary_handler_id": 3, "timeout": 300}}}),
            json!({"type": "stt-start", "data": {}}),
            json!({"type": "stt-vad-end", "data": {"timestamp": 1200}}),
            json!({"type": "stt-end", "data": {"stt_output": {"text": " Turn on the fan."}}}),
            json!({"type": "intent-end", "data": {"intent_output": {"conversation_id": "c1",
                "response": {"speech": {"plain": {"speech": "Turned on the fan"}}}}}}),
            json!({"type": "tts-end", "data": {"tts_output": {"media_id": "m", "url": "/api/tts_proxy/x.mp3", "mime_type": "audio/mpeg"}}}),
            json!({"type": "run-end", "data": null}),
        ];
        let parsed: Vec<_> = events.iter().map(parse_event).collect();
        assert_eq!(
            parsed,
            [
                RunEvent::Started { handler: Some(3) },
                RunEvent::Other,
                RunEvent::SpeechEnded,
                RunEvent::Transcribed("Turn on the fan.".into()),
                RunEvent::Replied {
                    speech: "Turned on the fan".into(),
                    conversation_id: Some("c1".into())
                },
                RunEvent::Spoken {
                    url: "/api/tts_proxy/x.mp3".into()
                },
                RunEvent::Finished,
            ]
        );
    }

    #[test]
    fn parses_errors_and_wake_word() {
        assert_eq!(
            parse_event(
                &json!({"type": "error", "data": {"code": "stt-no-text-recognized", "message": "No text recognized"}})
            ),
            RunEvent::Failed("Didn't catch that.".into())
        );
        assert_eq!(
            parse_event(
                &json!({"type": "wake_word-end", "data": {"wake_word_output": {"wake_word_id": "ok_nabu"}}})
            ),
            RunEvent::WakeWordDetected
        );
        assert_eq!(
            parse_event(&json!({"type": "wake_word-end", "data": {"wake_word_output": {}}})),
            RunEvent::Other
        );
    }

    #[test]
    fn parses_the_assistant_list() {
        let (assistants, preferred) = parse_assistants(&json!({
            "pipelines": [{"id": "a", "name": "Home Assistant"}, {"id": "b", "name": "Voice-Jev"}],
            "preferred_pipeline": "a"
        }));
        assert_eq!(
            assistants,
            [
                Assistant {
                    id: "a".into(),
                    name: "Home Assistant".into()
                },
                Assistant {
                    id: "b".into(),
                    name: "Voice-Jev".into()
                },
            ]
        );
        assert_eq!(preferred.as_deref(), Some("a"));
    }

    #[test]
    fn frames_prefix_the_handler_and_end_with_an_empty_payload() {
        assert_eq!(frame(7, &[1, 2, 3]), [7, 1, 2, 3]);
        assert_eq!(frame(7, &[]), [7]);
    }

    #[test]
    fn builds_requests() {
        let speech = run_request(Start::Speech, None, Some("p1"), Some("c1"));
        assert_eq!(speech["start_stage"], "stt");
        assert_eq!(speech["input"]["sample_rate"], 16000);
        assert_eq!(speech["pipeline"], "p1");
        assert_eq!(speech["conversation_id"], "c1");
        let text = run_request(Start::Text, Some("turn off the fan"), None, None);
        assert_eq!(text["start_stage"], "intent");
        assert_eq!(text["end_stage"], "intent");
        assert_eq!(text["input"]["text"], "turn off the fan");
        assert!(text.get("conversation_id").is_none());
        assert!(text.get("pipeline").is_none());
        let wake = run_request(Start::WakeWord, None, None, None);
        assert_eq!(wake["start_stage"], "wake_word");
        assert_eq!(wake["input"]["timeout"], 0);
        assert_eq!(wake["timeout"], 3600);
    }
}
