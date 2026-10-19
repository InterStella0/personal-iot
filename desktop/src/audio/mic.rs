//! Microphones through PulseAudio (PipeWire's pulse server on Plasma): listing them and
//! recording 16 kHz mono s16le, the format HA's Assist pipeline takes.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Result, anyhow, bail};
use libpulse_binding as pulse;
use libpulse_simple_binding::Simple;
use pulse::callbacks::ListResult;
use pulse::context::{Context, FlagSet, State};
use pulse::mainloop::standard::{IterateResult, Mainloop};
use pulse::sample::{Format, Spec};
use pulse::stream::Direction;

use crate::ha::assist::SAMPLE_RATE;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mic {
    /// The source name, e.g. `alsa_input.usb-...mono-fallback`.
    pub name: String,
    pub description: String,
    /// Bluetooth headsets switch to their call profile while a mic is open.
    pub bluetooth: bool,
}

/// Every real input; monitors of outputs are left out.
pub fn list() -> Result<Vec<Mic>> {
    let mut mainloop = Mainloop::new().ok_or_else(|| anyhow!("no PulseAudio main loop"))?;
    let mut context =
        Context::new(&mainloop, "Home").ok_or_else(|| anyhow!("no PulseAudio context"))?;
    context.connect(None, FlagSet::NOFLAGS, None)?;
    loop {
        match mainloop.iterate(true) {
            IterateResult::Success(_) => {}
            IterateResult::Quit(_) | IterateResult::Err(_) => bail!("PulseAudio main loop failed"),
        }
        match context.get_state() {
            State::Ready => break,
            State::Failed | State::Terminated => bail!("can't connect to PulseAudio"),
            _ => {}
        }
    }

    let mics = Rc::new(RefCell::new(Vec::new()));
    let done = Rc::new(RefCell::new(false));
    let operation = {
        let mics = mics.clone();
        let done = done.clone();
        context
            .introspect()
            .get_source_info_list(move |result| match result {
                ListResult::Item(info) => {
                    if info.monitor_of_sink.is_some() {
                        return;
                    }
                    let name = info.name.as_deref().unwrap_or_default().to_owned();
                    let bus = info.proplist.get_str("device.bus").unwrap_or_default();
                    mics.borrow_mut().push(Mic {
                        bluetooth: bus == "bluetooth" || name.starts_with("bluez_"),
                        description: info.description.as_deref().unwrap_or(&name).to_owned(),
                        name,
                    });
                }
                ListResult::End | ListResult::Error => *done.borrow_mut() = true,
            })
    };
    while !*done.borrow() {
        if let IterateResult::Quit(_) | IterateResult::Err(_) = mainloop.iterate(true) {
            break;
        }
    }
    drop(operation);
    context.disconnect();
    Ok(Rc::try_unwrap(mics)
        .map(RefCell::into_inner)
        .unwrap_or_default())
}

/// Records on its own thread, handing 100 ms chunks to `on_chunk` until it returns false or
/// `stop` is set. Returns once the source is open, or with the error that stopped it.
pub fn record(
    source: Option<&str>,
    stop: Arc<AtomicBool>,
    mut on_chunk: impl FnMut(Vec<u8>) -> bool + Send + 'static,
) -> Result<()> {
    let source = source.map(str::to_owned);
    let (opened_tx, opened_rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("mic".into())
        .spawn(move || {
            let spec = Spec {
                format: Format::S16le,
                channels: 1,
                rate: SAMPLE_RATE,
            };
            let simple = Simple::new(
                None,
                "Home",
                Direction::Record,
                source.as_deref(),
                "Voice command",
                &spec,
                None,
                None,
            );
            let simple = match simple {
                Ok(simple) => {
                    let _ = opened_tx.send(Ok(()));
                    simple
                }
                Err(err) => {
                    let _ = opened_tx.send(Err(anyhow!("{err}")));
                    return;
                }
            };
            let mut chunk = vec![0u8; SAMPLE_RATE as usize / 10 * 2];
            while !stop.load(Ordering::Relaxed) {
                if simple.read(&mut chunk).is_err() || !on_chunk(chunk.clone()) {
                    break;
                }
            }
        })?;
    opened_rx
        .recv()
        .unwrap_or_else(|_| Err(anyhow!("the recording thread died")))
}
