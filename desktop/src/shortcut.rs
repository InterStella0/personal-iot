//! The global "Talk" shortcut, through the desktop portal so it works on Wayland. Plasma
//! asks once which key to use; it can be changed in System Settings → Shortcuts.

use anyhow::{Context, Result};
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures::channel::mpsc::Sender;
use futures::{SinkExt, Stream, StreamExt};
use iot_desktop::config::APP_ID;

#[derive(Debug, Clone)]
pub enum Event {
    Talk,
    Unavailable(String),
}

pub fn run() -> impl Stream<Item = Event> {
    iced::stream::channel(8, async |mut out| {
        if let Err(err) = listen(&mut out).await {
            tracing::warn!("no global shortcut: {err:#}");
            let _ = out.send(Event::Unavailable(format!("{err:#}"))).await;
        }
        std::future::pending::<()>().await;
    })
}

async fn listen(out: &mut Sender<Event>) -> Result<()> {
    // Unsandboxed apps tell the portal who they are, before any other portal call.
    let app_id = APP_ID.parse().context("bad app id")?;
    if let Err(err) = ashpd::register_host_app(app_id).await {
        tracing::warn!("portal registration failed: {err}");
    }

    let portal = GlobalShortcuts::new().await?;
    let session = portal.create_session(Default::default()).await?;
    let shortcuts =
        [NewShortcut::new("talk", "Talk to Home Assistant").preferred_trigger("CTRL+ALT+H")];
    portal
        .bind_shortcuts(&session, &shortcuts, None, Default::default())
        .await?
        .response()
        .context("the shortcut wasn't set up")?;

    let mut activated = portal.receive_activated().await?;
    while let Some(event) = activated.next().await {
        if event.shortcut_id() == "talk" && out.send(Event::Talk).await.is_err() {
            break;
        }
    }
    // Keep the session alive as long as we listen.
    drop(session);
    Ok(())
}
