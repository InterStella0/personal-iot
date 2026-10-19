//! The connection to HA: one websocket, reconnected with backoff. It streams states and voice
//! events to the UI and takes commands back through a [`Handle`].

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use futures::channel::mpsc::Sender;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, Stream, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::assist::{self, Assistant, RunEvent, Start, VoiceEvent};
use super::auth::{self, RefreshError};
use super::entity::{Registry, State};
use crate::audio::{mic, play};

/// A command that takes longer than this is cut off: HA's VAD should end it well before.
const MAX_COMMAND: Duration = Duration::from_secs(15);
const PING_EVERY: Duration = Duration::from_secs(20);
const SILENT_FOR: Duration = Duration::from_secs(50);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Session {
    pub server: String,
    pub refresh_token: String,
}

#[derive(Debug, Clone)]
pub enum Event {
    Online(Handle),
    Offline(String),
    /// HA rejected the saved login.
    SignedOut,
    Snapshot {
        states: Vec<State>,
        registry: Registry,
    },
    StateChanged {
        entity_id: String,
        state: Option<State>,
    },
    /// The assistants set up in HA, and which one HA prefers.
    Assistants {
        list: Vec<Assistant>,
        preferred: Option<String>,
    },
    Voice(VoiceEvent),
    /// A button press failed.
    Error(String),
}

/// Sends commands to the live connection.
#[derive(Clone)]
pub struct Handle(mpsc::UnboundedSender<Command>);

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Handle")
    }
}

impl Handle {
    pub fn send(&self, command: Command) {
        let _ = self.0.send(command);
    }
}

#[derive(Debug, Clone)]
pub enum Audio {
    /// Record from this PulseAudio source, or the default one.
    Mic(Option<String>),
    /// 16 kHz mono s16le, sent as if spoken. Used by the smoke test.
    Pcm(Arc<Vec<u8>>),
}

#[derive(Debug, Clone)]
pub enum Command {
    Call {
        domain: String,
        service: String,
        entity_id: String,
        data: Value,
    },
    Say(String),
    Listen(Audio),
    StopListening,
    /// `Some`: keep a wake word run going on this audio. `None`: stop.
    AlwaysListen(Option<Audio>),
    /// Which assistant handles commands; `None` is HA's preferred one.
    UseAssistant(Option<String>),
}

impl Command {
    pub fn call(entity_id: &str, service: &str, data: Value) -> Self {
        Command::Call {
            domain: entity_id.split('.').next().unwrap_or_default().to_owned(),
            service: service.to_owned(),
            entity_id: entity_id.to_owned(),
            data,
        }
    }
}

pub fn connect(session: &Session) -> impl Stream<Item = Event> + use<> {
    let session = session.clone();
    iced::stream::channel(64, async move |out| run(session, out).await)
}

async fn run(session: Session, mut out: Sender<Event>) {
    let http = auth::http_client();
    let mut delay = Duration::from_secs(1);
    loop {
        let started = Instant::now();
        let reason = match auth::refresh(&http, &session.server, &session.refresh_token).await {
            Err(RefreshError::Rejected) => {
                let _ = out.send(Event::SignedOut).await;
                return std::future::pending().await;
            }
            Err(RefreshError::Unreachable(err)) => format!("Can't reach Home Assistant ({err:#})"),
            Ok(token) => {
                match Connection::open(&http, &session.server, &token.access_token, out.clone())
                    .await
                {
                    Ok((connection, stream, commands, audio)) => {
                        let err = connection.run(stream, commands, audio).await;
                        format!("Lost the connection ({err:#})")
                    }
                    Err(err) => format!("Can't connect to Home Assistant ({err:#})"),
                }
            }
        };
        tracing::warn!("{reason}");
        let _ = out.send(Event::Offline(reason)).await;
        if started.elapsed() > Duration::from_secs(60) {
            delay = Duration::from_secs(1);
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(30));
    }
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

enum Pending {
    Areas,
    Devices,
    Entities,
    States,
    Assistants,
    Config,
    Subscribe,
    Call(String),
    Run,
    Ping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunKind {
    Speech,
    Wake,
    Text,
}

struct Run {
    kind: RunKind,
    spoke: bool,
    woke: bool,
}

/// Audio going into a run.
struct Feed {
    handler: Option<u8>,
    buffered: Vec<Vec<u8>>,
    stop: Arc<AtomicBool>,
    deadline: Option<Instant>,
}

impl Drop for Feed {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct Snapshot {
    areas: Option<Value>,
    devices: Option<Value>,
    entities: Option<Value>,
    states: Option<Vec<State>>,
}

struct Connection {
    out: Sender<Event>,
    sink: SplitSink<Socket, Message>,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    snapshot: Option<Snapshot>,
    state_events: Option<u64>,
    started_events: Option<u64>,
    /// HA has finished starting. Wake word runs wait for it: a run that starts earlier holds up
    /// HA's startup, since HA waits for running tasks before it reports itself started.
    ha_running: bool,
    runs: HashMap<u64, Run>,
    feeds: HashMap<u64, Feed>,
    audio_tx: mpsc::Sender<(u64, Vec<u8>)>,
    wake_audio: Option<Audio>,
    wake_restart: Option<Instant>,
    pipeline: Option<String>,
    conversation_id: Option<String>,
    server: String,
    http: reqwest::Client,
    last_seen: Instant,
    last_ping: Instant,
}

type Opened = (
    Connection,
    SplitStream<Socket>,
    mpsc::UnboundedReceiver<Command>,
    mpsc::Receiver<(u64, Vec<u8>)>,
);

fn websocket_url(server: &str) -> Result<String> {
    let server = server.trim_end_matches('/');
    let rest = server
        .strip_prefix("http://")
        .map(|rest| format!("ws://{rest}"))
        .or_else(|| {
            server
                .strip_prefix("https://")
                .map(|rest| format!("wss://{rest}"))
        })
        .context("the server address must start with http:// or https://")?;
    Ok(format!("{rest}/api/websocket"))
}

fn text(value: &Value) -> Message {
    Message::Text(value.to_string().into())
}

impl Connection {
    async fn open(
        http: &reqwest::Client,
        server: &str,
        access_token: &str,
        mut out: Sender<Event>,
    ) -> Result<Opened> {
        let (socket, _) = tokio::time::timeout(
            Duration::from_secs(15),
            tokio_tungstenite::connect_async(websocket_url(server)?),
        )
        .await
        .context("timed out")??;
        let (mut sink, mut stream) = socket.split();

        let mut authenticated = false;
        while let Some(message) = stream.next().await {
            let Message::Text(message) = message? else {
                continue;
            };
            let message: Value = serde_json::from_str(&message)?;
            match message["type"].as_str() {
                Some("auth_required") => {
                    sink.send(text(&json!({"type": "auth", "access_token": access_token})))
                        .await?;
                }
                Some("auth_ok") => {
                    authenticated = true;
                    break;
                }
                Some("auth_invalid") => bail!("Home Assistant refused the login"),
                _ => {}
            }
        }
        if !authenticated {
            bail!("closed during login");
        }

        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (audio_tx, audio_rx) = mpsc::channel(64);
        out.send(Event::Online(Handle(command_tx))).await?;

        let mut connection = Connection {
            out,
            sink,
            next_id: 1,
            pending: HashMap::new(),
            snapshot: Some(Snapshot::default()),
            state_events: None,
            started_events: None,
            ha_running: false,
            runs: HashMap::new(),
            feeds: HashMap::new(),
            audio_tx,
            wake_audio: None,
            wake_restart: None,
            pipeline: None,
            conversation_id: None,
            server: server.to_owned(),
            http: http.clone(),
            last_seen: Instant::now(),
            last_ping: Instant::now(),
        };
        connection
            .request(json!({"type": "config/area_registry/list"}), Pending::Areas)
            .await?;
        connection
            .request(
                json!({"type": "config/device_registry/list"}),
                Pending::Devices,
            )
            .await?;
        connection
            .request(
                json!({"type": "config/entity_registry/list"}),
                Pending::Entities,
            )
            .await?;
        connection
            .request(json!({"type": "get_states"}), Pending::States)
            .await?;
        connection
            .request(
                json!({"type": "assist_pipeline/pipeline/list"}),
                Pending::Assistants,
            )
            .await?;
        let id = connection
            .request(
                json!({"type": "subscribe_events", "event_type": "state_changed"}),
                Pending::Subscribe,
            )
            .await?;
        connection.state_events = Some(id);
        let id = connection
            .request(
                json!({"type": "subscribe_events", "event_type": "homeassistant_started"}),
                Pending::Subscribe,
            )
            .await?;
        connection.started_events = Some(id);
        connection
            .request(json!({"type": "get_config"}), Pending::Config)
            .await?;
        Ok((connection, stream, command_rx, audio_rx))
    }

    async fn run(
        mut self,
        mut stream: SplitStream<Socket>,
        mut commands: mpsc::UnboundedReceiver<Command>,
        mut audio: mpsc::Receiver<(u64, Vec<u8>)>,
    ) -> anyhow::Error {
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            let result = tokio::select! {
                message = stream.next() => match message {
                    Some(Ok(message)) => self.on_message(message).await,
                    Some(Err(err)) => Err(err.into()),
                    None => Err(anyhow!("Home Assistant closed it")),
                },
                Some(command) = commands.recv() => self.on_command(command).await,
                Some((run, chunk)) = audio.recv() => self.on_audio(run, chunk).await,
                _ = tick.tick() => self.on_tick().await,
            };
            if let Err(err) = result {
                if !self.feeds.is_empty() || self.runs.values().any(|run| run.kind != RunKind::Wake)
                {
                    self.voice(VoiceEvent::Error(
                        "Lost the connection to Home Assistant.".into(),
                    ))
                    .await;
                }
                return err;
            }
        }
    }

    async fn request(&mut self, mut message: Value, pending: Pending) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        message["id"] = id.into();
        self.pending.insert(id, pending);
        self.sink.send(text(&message)).await?;
        Ok(id)
    }

    async fn emit(&mut self, event: Event) {
        let _ = self.out.send(event).await;
    }

    async fn voice(&mut self, event: VoiceEvent) {
        self.emit(Event::Voice(event)).await;
    }

    async fn on_message(&mut self, message: Message) -> Result<()> {
        self.last_seen = Instant::now();
        let Message::Text(message) = message else {
            return Ok(());
        };
        let message: Value = serde_json::from_str(&message)?;
        match message {
            // Coalesced messages, in case HA ever sends them.
            Value::Array(messages) => {
                for message in messages {
                    self.on_json(message).await?;
                }
                Ok(())
            }
            message => self.on_json(message).await,
        }
    }

    async fn on_json(&mut self, message: Value) -> Result<()> {
        let id = message["id"].as_u64().unwrap_or_default();
        match message["type"].as_str() {
            Some("result") => self.on_result(id, &message).await,
            Some("event") if Some(id) == self.started_events => self.on_started().await,
            Some("event") if Some(id) == self.state_events => {
                let data = &message["event"]["data"];
                let entity_id = data["entity_id"].as_str().unwrap_or_default().to_owned();
                let state = serde_json::from_value(data["new_state"].clone()).ok();
                self.emit(Event::StateChanged { entity_id, state }).await;
                Ok(())
            }
            Some("event") if self.runs.contains_key(&id) => {
                let event = assist::parse_event(&message["event"]);
                self.on_run_event(id, event).await
            }
            Some("pong") => {
                self.pending.remove(&id);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn on_result(&mut self, id: u64, message: &Value) -> Result<()> {
        let Some(pending) = self.pending.remove(&id) else {
            return Ok(());
        };
        let success = message["success"].as_bool().unwrap_or(false);
        let error = message["error"]["message"]
            .as_str()
            .unwrap_or("unknown error")
            .to_owned();
        let result = if success {
            message["result"].clone()
        } else {
            Value::Null
        };
        match pending {
            Pending::Areas | Pending::Devices | Pending::Entities | Pending::States => {
                if let Some(snapshot) = self.snapshot.as_mut() {
                    match pending {
                        Pending::Areas => snapshot.areas = Some(result),
                        Pending::Devices => snapshot.devices = Some(result),
                        Pending::Entities => snapshot.entities = Some(result),
                        _ => {
                            snapshot.states =
                                Some(serde_json::from_value(result).unwrap_or_default())
                        }
                    }
                }
                self.maybe_emit_snapshot().await;
            }
            Pending::Config if success && result["state"] == "RUNNING" => self.on_started().await?,
            Pending::Assistants if success => {
                let (list, preferred) = assist::parse_assistants(&result);
                self.emit(Event::Assistants { list, preferred }).await;
            }
            Pending::Subscribe if !success => bail!("couldn't subscribe to state changes: {error}"),
            Pending::Call(what) if !success => {
                self.emit(Event::Error(format!("Couldn't {what}: {error}")))
                    .await;
            }
            Pending::Run if !success => {
                let kind = self.runs.remove(&id).map(|run| run.kind);
                self.feeds.remove(&id);
                if kind == Some(RunKind::Wake) {
                    self.wake_audio = None;
                }
                self.voice(VoiceEvent::Error(error)).await;
            }
            _ => {}
        }
        Ok(())
    }

    async fn maybe_emit_snapshot(&mut self) {
        let ready = self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.areas.is_some()
                && snapshot.devices.is_some()
                && snapshot.entities.is_some()
                && snapshot.states.is_some()
        });
        if !ready {
            return;
        }
        let Snapshot {
            areas,
            devices,
            entities,
            states,
        } = self.snapshot.take().unwrap_or_default();
        let registry = Registry::from_lists(
            areas.unwrap_or_default(),
            devices.unwrap_or_default(),
            entities.unwrap_or_default(),
        );
        let states = states.unwrap_or_default();
        self.emit(Event::Snapshot { states, registry }).await;
    }

    async fn on_command(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Call {
                domain,
                service,
                entity_id,
                data,
            } => {
                let what = format!("{} {entity_id}", service.replace('_', " "));
                self.request(
                    json!({
                        "type": "call_service",
                        "domain": domain,
                        "service": service,
                        "service_data": data,
                        "target": {"entity_id": entity_id},
                    }),
                    Pending::Call(what),
                )
                .await?;
            }
            Command::Say(text) => {
                self.start_run(RunKind::Text, Some(&text), None).await?;
            }
            Command::Listen(audio) => {
                if self.runs.values().any(|run| run.kind == RunKind::Speech) {
                    return Ok(());
                }
                self.start_run(RunKind::Speech, None, Some(audio)).await?;
            }
            Command::StopListening => {
                let speech: Vec<u64> = self
                    .runs
                    .iter()
                    .filter(|(_, run)| run.kind == RunKind::Speech)
                    .map(|(id, _)| *id)
                    .collect();
                for id in speech {
                    self.end_feed(id).await?;
                }
            }
            Command::AlwaysListen(audio) => {
                let wake: Vec<u64> = self
                    .runs
                    .iter()
                    .filter(|(_, run)| run.kind == RunKind::Wake)
                    .map(|(id, _)| *id)
                    .collect();
                self.wake_audio = audio;
                self.wake_restart = None;
                // A running wake word run ends when its audio does; `on_run_event` starts the
                // next one if always-listen is still on.
                for id in wake {
                    self.end_feed(id).await?;
                }
                if wake_is_idle(&self.runs) && self.wake_audio.is_some() {
                    self.start_wake_run().await?;
                }
            }
            Command::UseAssistant(pipeline) => {
                if pipeline != self.pipeline {
                    // A conversation belongs to one agent.
                    self.conversation_id = None;
                }
                self.pipeline = pipeline;
            }
        }
        Ok(())
    }

    async fn on_started(&mut self) -> Result<()> {
        if self.ha_running {
            return Ok(());
        }
        self.ha_running = true;
        if wake_is_idle(&self.runs) {
            self.start_wake_run().await?;
        }
        Ok(())
    }

    async fn start_wake_run(&mut self) -> Result<()> {
        let audio = self.wake_audio.clone();
        if let Some(audio) = audio
            && self.ha_running
        {
            self.start_run(RunKind::Wake, None, Some(audio)).await?;
        }
        Ok(())
    }

    async fn start_run(
        &mut self,
        kind: RunKind,
        text: Option<&str>,
        audio: Option<Audio>,
    ) -> Result<()> {
        let start = match kind {
            RunKind::Speech => Start::Speech,
            RunKind::Wake => Start::WakeWord,
            RunKind::Text => Start::Text,
        };
        let request = assist::run_request(
            start,
            text,
            self.pipeline.as_deref(),
            self.conversation_id.as_deref(),
        );
        let id = self.request(request, Pending::Run).await?;
        self.runs.insert(
            id,
            Run {
                kind,
                spoke: false,
                woke: false,
            },
        );
        if kind == RunKind::Text {
            self.voice(VoiceEvent::Heard(text.unwrap_or_default().to_owned()))
                .await;
            self.voice(VoiceEvent::Thinking).await;
        }
        let Some(audio) = audio else { return Ok(()) };

        let stop = Arc::new(AtomicBool::new(false));
        let tx = self.audio_tx.clone();
        match audio {
            Audio::Mic(source) => {
                let flag = stop.clone();
                let started = mic::record(source.as_deref(), flag, move |chunk| {
                    tx.blocking_send((id, chunk)).is_ok()
                });
                if let Err(err) = started {
                    self.runs.remove(&id);
                    if kind == RunKind::Wake {
                        self.wake_audio = None;
                    }
                    self.voice(VoiceEvent::Error(format!(
                        "Can't open the microphone: {err:#}"
                    )))
                    .await;
                    return Ok(());
                }
            }
            Audio::Pcm(pcm) => {
                let flag = stop.clone();
                tokio::spawn(async move {
                    // The recording, then a second of silence so HA's VAD hears the end.
                    let silence = vec![0u8; assist::SAMPLE_RATE as usize * 2];
                    for chunk in pcm.chunks(3200).chain(silence.chunks(3200)) {
                        if flag.load(Ordering::Relaxed)
                            || tx.send((id, chunk.to_vec())).await.is_err()
                        {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                });
            }
        }
        self.feeds.insert(
            id,
            Feed {
                handler: None,
                buffered: Vec::new(),
                stop,
                deadline: (kind == RunKind::Speech).then(|| Instant::now() + MAX_COMMAND),
            },
        );
        if kind == RunKind::Speech {
            play::chime(play::Chime::Start);
            self.voice(VoiceEvent::Listening).await;
        }
        Ok(())
    }

    async fn on_audio(&mut self, run: u64, chunk: Vec<u8>) -> Result<()> {
        let Some(feed) = self.feeds.get_mut(&run) else {
            return Ok(());
        };
        match feed.handler {
            Some(handler) => {
                self.sink
                    .send(Message::Binary(assist::frame(handler, &chunk).into()))
                    .await?;
            }
            None => feed.buffered.push(chunk),
        }
        Ok(())
    }

    /// Stops the run's audio and tells HA it ended.
    async fn end_feed(&mut self, run: u64) -> Result<()> {
        if let Some(feed) = self.feeds.remove(&run)
            && let Some(handler) = feed.handler
        {
            self.sink
                .send(Message::Binary(assist::frame(handler, &[]).into()))
                .await?;
        }
        Ok(())
    }

    async fn on_run_event(&mut self, id: u64, event: RunEvent) -> Result<()> {
        let Some(kind) = self.runs.get(&id).map(|run| run.kind) else {
            return Ok(());
        };
        match event {
            RunEvent::Started { handler } => {
                if let Some(feed) = self.feeds.get_mut(&id) {
                    feed.handler = handler;
                    if let Some(handler) = handler {
                        let buffered = std::mem::take(&mut feed.buffered);
                        for chunk in buffered {
                            self.sink
                                .send(Message::Binary(assist::frame(handler, &chunk).into()))
                                .await?;
                        }
                    }
                }
            }
            RunEvent::WakeWordDetected => {
                if let Some(run) = self.runs.get_mut(&id) {
                    run.woke = true;
                }
                if let Some(feed) = self.feeds.get_mut(&id) {
                    feed.deadline = Some(Instant::now() + MAX_COMMAND);
                }
                play::chime(play::Chime::Start);
                self.voice(VoiceEvent::Woke).await;
                self.voice(VoiceEvent::Listening).await;
            }
            RunEvent::SpeechEnded => {
                self.end_feed(id).await?;
                play::chime(play::Chime::Stop);
            }
            RunEvent::Transcribed(text) => {
                if self.feeds.contains_key(&id) {
                    self.end_feed(id).await?;
                    play::chime(play::Chime::Stop);
                }
                if !text.is_empty() {
                    self.voice(VoiceEvent::Heard(text)).await;
                }
                self.voice(VoiceEvent::Thinking).await;
            }
            RunEvent::Replied {
                speech,
                conversation_id,
            } => {
                if conversation_id.is_some() {
                    self.conversation_id = conversation_id;
                }
                self.voice(VoiceEvent::Reply(speech)).await;
            }
            RunEvent::Spoken { url } => {
                if let Some(run) = self.runs.get_mut(&id) {
                    run.spoke = true;
                }
                self.speak(url);
            }
            RunEvent::Finished => {
                self.end_feed(id).await?;
                let run = self.runs.remove(&id);
                let spoke = run.as_ref().is_some_and(|run| run.spoke);
                let woke = run.as_ref().is_some_and(|run| run.woke);
                if !spoke && (kind != RunKind::Wake || woke) {
                    self.voice(VoiceEvent::Done).await;
                }
                if kind == RunKind::Wake && self.wake_audio.is_some() {
                    self.start_wake_run().await?;
                }
            }
            RunEvent::Failed(message) => {
                self.end_feed(id).await?;
                let run = self.runs.remove(&id);
                let woke = run.is_some_and(|run| run.woke);
                if kind == RunKind::Wake && !woke {
                    // Before the wake word: a setup problem, or the audio stopped. Retry
                    // shortly, unless there's no wake word engine at all.
                    if message.contains("wake word engine") {
                        self.wake_audio = None;
                        self.voice(VoiceEvent::Error(message)).await;
                    } else if self.wake_audio.is_some() {
                        self.wake_restart = Some(Instant::now() + Duration::from_secs(2));
                    }
                } else {
                    play::chime(play::Chime::Error);
                    self.voice(VoiceEvent::Error(message)).await;
                    if kind == RunKind::Wake && self.wake_audio.is_some() {
                        self.wake_restart = Some(Instant::now() + Duration::from_secs(1));
                    }
                }
            }
            RunEvent::Other => {}
        }
        Ok(())
    }

    fn speak(&self, path: String) {
        let url = super::url(&self.server, &path);
        let http = self.http.clone();
        let mut out = self.out.clone();
        tokio::spawn(async move {
            let _ = out.send(Event::Voice(VoiceEvent::Speaking)).await;
            // tts_proxy URLs carry their own token, so no Authorization header.
            let audio = async {
                let response = http.get(&url).send().await?.error_for_status()?;
                anyhow::Ok(response.bytes().await?.to_vec())
            };
            match audio.await {
                Ok(bytes) => play::speak(bytes).await,
                Err(err) => tracing::warn!("couldn't fetch the spoken reply: {err:#}"),
            }
            let _ = out.send(Event::Voice(VoiceEvent::Done)).await;
        });
    }

    async fn on_tick(&mut self) -> Result<()> {
        let now = Instant::now();
        let expired: Vec<u64> = self
            .feeds
            .iter()
            .filter(|(_, feed)| feed.deadline.is_some_and(|deadline| now >= deadline))
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            self.end_feed(id).await?;
        }
        if self.wake_restart.is_some_and(|at| now >= at) {
            self.wake_restart = None;
            if wake_is_idle(&self.runs) {
                self.start_wake_run().await?;
            }
        }
        if now.duration_since(self.last_seen) > SILENT_FOR {
            bail!("no answer for {} seconds", SILENT_FOR.as_secs());
        }
        if now.duration_since(self.last_ping) > PING_EVERY {
            self.last_ping = now;
            self.request(json!({"type": "ping"}), Pending::Ping).await?;
        }
        Ok(())
    }
}

fn wake_is_idle(runs: &HashMap<u64, Run>) -> bool {
    !runs.values().any(|run| run.kind == RunKind::Wake)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_url_follows_the_scheme() {
        assert_eq!(
            websocket_url("http://stella-server:8123/").unwrap(),
            "ws://stella-server:8123/api/websocket"
        );
        assert_eq!(
            websocket_url("https://ha.example.com").unwrap(),
            "wss://ha.example.com/api/websocket"
        );
        assert!(websocket_url("stella-server:8123").is_err());
    }
}
