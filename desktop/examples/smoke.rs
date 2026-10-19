//! End-to-end check of the app's HA client against a test Home Assistant with the `demo`
//! integration, whisper and piper: login with MFA, live state, every kind of button, typed
//! commands, and a spoken command (piper's audio fed back in as the "mic").
//!
//! IOT_DESKTOP_MUTE=1 IOT_DESKTOP_TOKEN_FILE=/tmp/token \
//!     cargo run --example smoke -- <server> <user> <password> <totp-secret>

use std::collections::BTreeMap;
use std::io::Cursor;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use futures::{Stream, StreamExt};
use hmac::{Hmac, Mac};
use iot_desktop::ha::assist::VoiceEvent;
use iot_desktop::ha::auth::{self, Step};
use iot_desktop::ha::entity::{CONTROLLABLE, State, percentage_for_speed};
use iot_desktop::ha::store;
use iot_desktop::ha::ws::{self, Audio, Command, Event, Handle, Session};
use rodio::Source;
use serde_json::{Map, Value, json};

type Events = Pin<Box<dyn Stream<Item = Event> + Send>>;

struct Client {
    events: Events,
    handle: Handle,
    states: BTreeMap<String, State>,
    voice: Vec<VoiceEvent>,
    errors: Vec<String>,
}

impl Client {
    async fn next(&mut self) -> Result<Event> {
        let event = tokio::time::timeout(Duration::from_secs(30), self.events.next())
            .await
            .context("no event for 30 s")?
            .context("connection stream ended")?;
        match &event {
            Event::StateChanged { entity_id, state } => {
                match state {
                    Some(state) => self.states.insert(entity_id.clone(), state.clone()),
                    None => self.states.remove(entity_id),
                };
            }
            Event::Voice(voice) => self.voice.push(voice.clone()),
            Event::Error(error) => self.errors.push(error.clone()),
            Event::Offline(reason) => bail!("went offline: {reason}"),
            _ => {}
        }
        Ok(event)
    }

    async fn until_state(
        &mut self,
        entity_id: &str,
        what: &str,
        done: impl Fn(&State) -> bool,
    ) -> Result<()> {
        loop {
            if self.states.get(entity_id).is_some_and(&done) {
                println!("  ok: {entity_id} {what}");
                return Ok(());
            }
            self.next()
                .await
                .with_context(|| format!("waiting for {entity_id} {what}"))?;
        }
    }

    async fn until_voice(&mut self, done: impl Fn(&VoiceEvent) -> bool) -> Result<VoiceEvent> {
        loop {
            if let Some(event) = self.voice.iter().find(|event| done(event)).cloned() {
                return Ok(event);
            }
            if let Some(error) = self.voice.iter().find_map(|event| match event {
                VoiceEvent::Error(error) => Some(error.clone()),
                _ => None,
            }) {
                bail!("voice error: {error}");
            }
            self.next().await?;
        }
    }

    fn call(&self, entity_id: &str, service: &str, data: Value) {
        self.handle.send(Command::call(entity_id, service, data));
    }
}

fn totp(secret: &str) -> String {
    let key = base32(secret);
    let counter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        / 30;
    let mut mac = Hmac::<sha1::Sha1>::new_from_slice(&key).unwrap();
    mac.update(&counter.to_be_bytes());
    let hash = mac.finalize().into_bytes();
    let offset = usize::from(hash[19] & 0xf);
    let code = u32::from_be_bytes([
        hash[offset] & 0x7f,
        hash[offset + 1],
        hash[offset + 2],
        hash[offset + 3],
    ]);
    format!("{:06}", code % 1_000_000)
}

fn base32(text: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut bits, mut value, mut out) = (0u32, 0u32, Vec::new());
    for c in text.trim_end_matches('=').bytes() {
        let index = ALPHABET
            .iter()
            .position(|&a| a == c.to_ascii_uppercase())
            .unwrap() as u32;
        value = (value << 5) | index;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((value >> bits) as u8);
            value &= (1 << bits) - 1;
        }
    }
    out
}

fn fields(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), Value::from(*v)))
        .collect()
}

/// Piper says `text`; returns it as 16 kHz mono s16le, like the mic would.
async fn speech(
    http: &reqwest::Client,
    server: &str,
    access_token: &str,
    text: &str,
) -> Result<Vec<u8>> {
    let url: Value = http
        .post(format!("{server}/api/tts_get_url"))
        .bearer_auth(access_token)
        .json(&json!({"engine_id": "tts.piper", "message": text}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let path = url["path"].as_str().context("no tts path")?;
    let bytes = http
        .get(format!("{server}{path}"))
        .send()
        .await?
        .bytes()
        .await?;
    let decoder = rodio::Decoder::try_from(Cursor::new(bytes.to_vec()))?;
    let channels = usize::from(decoder.channels().get());
    let rate = f64::from(decoder.sample_rate().get());
    let mono: Vec<f32> = decoder
        .collect::<Vec<_>>()
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect();
    let count = (mono.len() as f64 * 16_000.0 / rate) as usize;
    let mut pcm = Vec::with_capacity(count * 2);
    for i in 0..count {
        let position = i as f64 * rate / 16_000.0;
        let (index, fraction) = (position as usize, (position.fract()) as f32);
        let a = mono.get(index).copied().unwrap_or(0.0);
        let b = mono.get(index + 1).copied().unwrap_or(a);
        let sample = ((a + (b - a) * fraction) * 32767.0).clamp(-32768.0, 32767.0) as i16;
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(pcm)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [server, user, password, secret] = args.as_slice() else {
        bail!("usage: smoke <server> <user> <password> <totp-secret>");
    };
    let http = auth::http_client();

    println!("login");
    let (flow, _) = auth::start(&http, server).await?;
    let step = auth::submit(
        &http,
        &flow,
        fields(&[("username", user), ("password", "wrong")]),
    )
    .await?;
    ensure!(
        matches!(&step, Step::Form { error: Some(_), .. }),
        "wrong password accepted: {step:?}"
    );
    println!("  ok: wrong password rejected");
    let (flow, _) = auth::start(&http, server).await?;
    let step = auth::submit(
        &http,
        &flow,
        fields(&[("username", user), ("password", password)]),
    )
    .await?;
    ensure!(
        matches!(&step, Step::Form { step_id, .. } if step_id == "mfa"),
        "expected the MFA step: {step:?}"
    );
    let step = auth::submit(&http, &flow, fields(&[("code", "000000")])).await?;
    ensure!(
        matches!(&step, Step::Form { error: Some(_), .. }),
        "wrong code accepted: {step:?}"
    );
    println!("  ok: wrong MFA code rejected");
    let step = auth::submit(&http, &flow, fields(&[("code", &totp(secret))])).await?;
    let Step::Done { code } = step else {
        bail!("MFA code refused: {step:?}")
    };
    let tokens = auth::exchange(&http, &flow, &code).await?;
    store::save(&tokens.refresh_token).await?;
    ensure!(store::load().await.as_deref() == Some(tokens.refresh_token.as_str()));
    let access = auth::refresh(&http, server, &tokens.refresh_token)
        .await
        .map_err(|err| anyhow::anyhow!("{err:?}"))?;
    println!("  ok: logged in with password + MFA (PKCE), token stored and refreshed");

    println!("connect");
    let session = Session {
        server: server.clone(),
        refresh_token: tokens.refresh_token.clone(),
    };
    let mut events: Events = Box::pin(ws::connect(&session));
    let handle = loop {
        match events.next().await.context("stream ended")? {
            Event::Online(handle) => break handle,
            Event::Offline(reason) => bail!("offline: {reason}"),
            _ => {}
        }
    };
    let mut client = Client {
        events,
        handle,
        states: BTreeMap::new(),
        voice: Vec::new(),
        errors: Vec::new(),
    };
    loop {
        if let Event::Snapshot { states, .. } = client.next().await? {
            client.states = states
                .into_iter()
                .map(|s| (s.entity_id.clone(), s))
                .collect();
            break;
        }
    }
    let mut per_domain: BTreeMap<&str, usize> = BTreeMap::new();
    for state in client
        .states
        .values()
        .filter(|s| CONTROLLABLE.contains(&s.domain()))
    {
        *per_domain.entry(state.domain()).or_default() += 1;
    }
    println!("  ok: snapshot, controllable devices {per_domain:?}");

    println!("buttons");
    let fan = client
        .states
        .values()
        .find(|s| s.domain() == "fan" && s.fan_speed_count().is_some_and(|n| n >= 3))
        .context("no multi-speed demo fan")?
        .clone();
    let count = fan.fan_speed_count().unwrap();
    client.call(
        &fan.entity_id,
        "set_percentage",
        json!({"percentage": percentage_for_speed(2, count)}),
    );
    client
        .until_state(&fan.entity_id, "at speed 2", |s| s.fan_speed() == Some(2))
        .await?;
    client.call(&fan.entity_id, "turn_off", json!({}));
    client
        .until_state(&fan.entity_id, "off", |s| !s.is_on())
        .await?;

    let climate = client
        .states
        .values()
        .find(|s| {
            s.domain() == "climate"
                && s.number("temperature").is_some()
                && !s.list("fan_modes").is_empty()
        })
        .context("no demo climate with a target and fan modes")?
        .clone();
    let target = climate.number("temperature").unwrap();
    let step = climate.number("target_temp_step").unwrap_or(1.0);
    let wanted = if target + step <= climate.number("max_temp").unwrap_or(30.0) {
        target + step
    } else {
        target - step
    };
    client.call(
        &climate.entity_id,
        "set_temperature",
        json!({"temperature": wanted}),
    );
    client
        .until_state(&climate.entity_id, &format!("target {wanted}"), |s| {
            s.number("temperature") == Some(wanted)
        })
        .await?;
    let mode = climate
        .list("hvac_modes")
        .into_iter()
        .find(|m| m != "off" && *m != climate.state)
        .context("no other hvac mode")?;
    client.call(
        &climate.entity_id,
        "set_hvac_mode",
        json!({"hvac_mode": mode}),
    );
    client
        .until_state(&climate.entity_id, &format!("mode {mode}"), |s| {
            s.state == mode
        })
        .await?;
    let fan_mode = climate
        .list("fan_modes")
        .into_iter()
        .find(|m| Some(m.as_str()) != climate.text("fan_mode"))
        .context("no other fan mode")?;
    client.call(
        &climate.entity_id,
        "set_fan_mode",
        json!({"fan_mode": fan_mode}),
    );
    client
        .until_state(&climate.entity_id, &format!("fan {fan_mode}"), |s| {
            s.text("fan_mode") == Some(fan_mode.as_str())
        })
        .await?;
    client.call(&climate.entity_id, "turn_off", json!({}));
    client
        .until_state(&climate.entity_id, "off", |s| s.state == "off")
        .await?;

    let switch = client
        .states
        .values()
        .find(|s| s.domain() == "switch" && s.available())
        .context("no demo switch")?
        .clone();
    let service = if switch.is_on() {
        "turn_off"
    } else {
        "turn_on"
    };
    let was_on = switch.is_on();
    client.call(&switch.entity_id, service, json!({}));
    client
        .until_state(&switch.entity_id, "toggled", |s| s.is_on() != was_on)
        .await?;

    client.call("light.does_not_exist", "turn_on", json!({}));
    client.call("fan.home_fan_missing", "no_such_service", json!({}));
    while client.errors.is_empty() {
        client.next().await?;
    }
    println!("  ok: a failing button reports \"{}\"", client.errors[0]);

    println!("typed command");
    client.call("light.kitchen_lights", "turn_on", json!({}));
    client
        .until_state("light.kitchen_lights", "on", State::is_on)
        .await?;
    client.voice.clear();
    client
        .handle
        .send(Command::Say("turn off the kitchen lights".into()));
    let reply = client
        .until_voice(|e| matches!(e, VoiceEvent::Reply(_)))
        .await?;
    client
        .until_state("light.kitchen_lights", "off by typed command", |s| {
            !s.is_on()
        })
        .await?;
    client
        .until_voice(|e| matches!(e, VoiceEvent::Done))
        .await?;
    ensure!(
        !client.voice.contains(&VoiceEvent::Speaking),
        "typed commands shouldn't speak"
    );
    println!("  ok: reply {reply:?}, not spoken");

    println!("spoken command");
    let pcm = speech(
        &http,
        server,
        &access.access_token,
        "Turn on the kitchen lights.",
    )
    .await?;
    println!("  piper audio: {:.1} s", pcm.len() as f64 / 32_000.0);
    client.voice.clear();
    client
        .handle
        .send(Command::Listen(Audio::Pcm(Arc::new(pcm))));
    let heard = client
        .until_voice(|e| matches!(e, VoiceEvent::Heard(_)))
        .await?;
    let reply = client
        .until_voice(|e| matches!(e, VoiceEvent::Reply(_)))
        .await?;
    client
        .until_state("light.kitchen_lights", "on by voice", State::is_on)
        .await?;
    client
        .until_voice(|e| matches!(e, VoiceEvent::Speaking))
        .await?;
    client
        .until_voice(|e| matches!(e, VoiceEvent::Done))
        .await?;
    println!("  ok: heard {heard:?}, reply {reply:?}, spoken reply fetched");
    println!(
        "  voice events: {:?}",
        client
            .voice
            .iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
    );

    if client.states.keys().any(|id| id.starts_with("wake_word.")) {
        println!("wake word");
        let pcm = speech(
            &http,
            server,
            &access.access_token,
            "Okay Nabu. Turn off the kitchen lights.",
        )
        .await?;
        client.voice.clear();
        client
            .handle
            .send(Command::AlwaysListen(Some(Audio::Pcm(Arc::new(pcm)))));
        client
            .until_voice(|e| matches!(e, VoiceEvent::Woke))
            .await?;
        let heard = client
            .until_voice(|e| matches!(e, VoiceEvent::Heard(_)))
            .await?;
        client
            .until_state("light.kitchen_lights", "off after the wake word", |s| {
                !s.is_on()
            })
            .await?;
        client.handle.send(Command::AlwaysListen(None));
        println!("  ok: woke on \"okay nabu\", heard {heard:?}");
    }

    println!("all checks passed");
    Ok(())
}
