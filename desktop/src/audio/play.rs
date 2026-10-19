//! Spoken replies and short chimes, on one audio thread that owns the output device.
//! `IOT_DESKTOP_MUTE` turns all sound off (for tests).

use std::io::Cursor;
use std::num::NonZero;
use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

use rodio::Source;
use rodio::source::{Function, SignalGenerator};
use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy)]
pub enum Chime {
    Start,
    Stop,
    Error,
}

enum Job {
    Speak(Vec<u8>, oneshot::Sender<()>),
    Chime(Chime),
}

fn muted() -> bool {
    std::env::var_os("IOT_DESKTOP_MUTE").is_some()
}

fn player() -> &'static Sender<Job> {
    static PLAYER: OnceLock<Sender<Job>> = OnceLock::new();
    PLAYER.get_or_init(|| {
        let (tx, rx) = channel::<Job>();
        std::thread::Builder::new()
            .name("audio-out".into())
            .spawn(move || {
                let mut sink = None;
                for job in rx {
                    if sink.is_none() {
                        sink = match rodio::DeviceSinkBuilder::open_default_sink() {
                            Ok(mut sink) => {
                                sink.log_on_drop(false);
                                Some(sink)
                            }
                            Err(err) => {
                                tracing::warn!("no audio output: {err}");
                                None
                            }
                        };
                    }
                    match (job, sink.as_ref()) {
                        (Job::Speak(bytes, done), Some(sink)) => {
                            match rodio::Decoder::try_from(Cursor::new(bytes)) {
                                Ok(decoder) => {
                                    let player = rodio::Player::connect_new(sink.mixer());
                                    player.append(decoder);
                                    player.sleep_until_end();
                                }
                                Err(err) => tracing::warn!("can't decode the spoken reply: {err}"),
                            }
                            let _ = done.send(());
                        }
                        (Job::Speak(_, done), None) => {
                            let _ = done.send(());
                        }
                        (Job::Chime(chime), Some(sink)) => sink.mixer().add(tone(chime)),
                        (Job::Chime(_), None) => {}
                    }
                }
            })
            .expect("can start the audio thread");
        tx
    })
}

fn tone(chime: Chime) -> impl Source + Send + 'static {
    let rate = NonZero::new(48_000).expect("non-zero");
    let beep = |hz: f32, ms: u64| {
        SignalGenerator::new(rate, hz, Function::Sine)
            .amplify(0.12)
            .take_duration(Duration::from_millis(ms))
            .fade_in(Duration::from_millis(8))
    };
    match chime {
        Chime::Start => beep(880.0, 110).mix(beep(1320.0, 110)),
        Chime::Stop => beep(660.0, 90).mix(beep(440.0, 90)),
        Chime::Error => beep(220.0, 220).mix(beep(233.0, 220)),
    }
}

/// Plays an mp3 or wav reply; resolves when it has finished.
pub async fn speak(bytes: Vec<u8>) {
    if muted() {
        return;
    }
    let (done_tx, done_rx) = oneshot::channel();
    if player().send(Job::Speak(bytes, done_tx)).is_ok() {
        let _ = done_rx.await;
    }
}

pub fn chime(chime: Chime) {
    if !muted() {
        let _ = player().send(Job::Chime(chime));
    }
}
