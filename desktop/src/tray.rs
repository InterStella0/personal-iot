//! The tray icon (StatusNotifierItem): Talk, Open, a quick toggle per device, and Quit.

use std::fmt;

use futures::SinkExt;
use futures::Stream;
use ksni::TrayMethods;
use ksni::menu::{CheckmarkItem, MenuItem, StandardItem};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

#[derive(Debug, Clone)]
pub enum Event {
    Ready(Handle),
    Open,
    Talk,
    Toggle(String),
    Quit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub entity_id: String,
    pub name: String,
    pub on: bool,
    pub available: bool,
}

#[derive(Clone)]
pub struct Handle(ksni::Handle<Tray>);

impl fmt::Debug for Handle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("tray::Handle")
    }
}

impl Handle {
    pub async fn set_devices(self, devices: Vec<Device>) {
        self.0.update(|tray| tray.devices = devices).await;
    }
}

pub struct Tray {
    events: UnboundedSender<Event>,
    devices: Vec<Device>,
}

impl Tray {
    fn item(label: &str, event: Event) -> MenuItem<Self> {
        StandardItem {
            label: label.into(),
            activate: Box::new(move |tray: &mut Self| {
                let _ = tray.events.send(event.clone());
            }),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "iot-desktop".into()
    }

    fn title(&self) -> String {
        "Home".into()
    }

    fn icon_name(&self) -> String {
        "go-home".into()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.events.send(Event::Open);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut menu = vec![
            Self::item("Talk", Event::Talk),
            Self::item("Open Home", Event::Open),
        ];
        if !self.devices.is_empty() {
            menu.push(MenuItem::Separator);
        }
        for device in &self.devices {
            let entity_id = device.entity_id.clone();
            menu.push(
                CheckmarkItem {
                    label: device.name.clone(),
                    checked: device.on,
                    enabled: device.available,
                    activate: Box::new(move |tray: &mut Self| {
                        let _ = tray.events.send(Event::Toggle(entity_id.clone()));
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        menu.push(MenuItem::Separator);
        menu.push(Self::item("Quit", Event::Quit));
        menu
    }
}

pub fn run() -> impl Stream<Item = Event> {
    iced::stream::channel(16, async |mut out| {
        let (events, mut received) = unbounded_channel();
        let tray = Tray {
            events,
            devices: Vec::new(),
        };
        match tray.spawn().await {
            Ok(handle) => {
                let _ = out.send(Event::Ready(Handle(handle))).await;
                while let Some(event) = received.recv().await {
                    if out.send(event).await.is_err() {
                        break;
                    }
                }
            }
            // No tray host: closing the window then quits instead of hiding.
            Err(err) => tracing::warn!("no system tray: {err}"),
        }
        std::future::pending::<()>().await;
    })
}
