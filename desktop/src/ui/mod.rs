//! The app: one window that can hide to the tray, signed in to one HA.

mod dashboard;
mod icon;
mod login;
mod settings;
pub mod style;
mod voice;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use iced::widget::{Row, button, column, container, row, space, text, tooltip};
use iced::{Alignment, Color, Element, Length, Size, Subscription, Task, Theme, window};
use iot_desktop::audio::mic::{self, Mic};
use iot_desktop::config::{APP_ID, Config};
use iot_desktop::ha::assist::Assistant;
use iot_desktop::ha::entity::{Registry, State};
use iot_desktop::ha::store;
use iot_desktop::ha::ws::{self, Audio, Command, Event, Handle, Session};
use serde_json::{Value, json};

use crate::{shortcut, tray};
use icon::{Icon, icon};
use settings::{AssistantChoice, MicChoice};
use voice::{Phase, Voice};

pub struct Options {
    pub hidden: bool,
    /// Save a PNG of the window after it loads, then quit (for checking the layout).
    pub screenshot: Option<PathBuf>,
    pub start_in_settings: bool,
    pub start_room: Option<String>,
}

enum Screen {
    Starting,
    Login(Box<login::Login>),
    Home,
}

#[derive(Debug, Clone, PartialEq)]
enum Link {
    Connecting,
    Online,
    Offline(String),
}

pub struct App {
    theme: Theme,
    config: Config,
    screen: Screen,
    window: Option<window::Id>,
    session: Option<Session>,
    handle: Option<Handle>,
    link: Link,
    states: BTreeMap<String, State>,
    registry: Registry,
    loaded: bool,
    voice: Voice,
    command: String,
    settings: bool,
    /// Show only this area's devices.
    room: Option<String>,
    mics: Vec<Mic>,
    assistants: Vec<Assistant>,
    preferred_assistant: Option<String>,
    notice: Option<String>,
    shortcut_problem: Option<String>,
    tray: Option<tray::Handle>,
    tray_devices: Vec<tray::Device>,
    screenshot: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Loaded(Option<String>),
    WindowOpened(window::Id),
    CloseRequested(window::Id),
    WindowClosed(window::Id),
    Login(login::Message),
    SignOut,
    Ha(Event),
    Call(String, String, Value),
    Talk,
    CommandChanged(String),
    SendCommand,
    ToggleSettings,
    Room(Option<String>),
    RefreshMics,
    Mics(Result<Vec<Mic>, String>),
    MicSelected(MicChoice),
    AssistantSelected(AssistantChoice),
    WakeWord(bool),
    DismissNotice,
    Tray(tray::Event),
    Shortcut(shortcut::Event),
    TakeScreenshot,
    Screenshot(window::Screenshot),
}

impl App {
    pub fn new(options: Options) -> (Self, Task<Message>) {
        let app = Self {
            theme: style::theme(),
            config: Config::load(),
            screen: Screen::Starting,
            window: None,
            session: None,
            handle: None,
            link: Link::Connecting,
            states: BTreeMap::new(),
            registry: Registry::default(),
            loaded: false,
            voice: Voice::default(),
            command: String::new(),
            settings: options.start_in_settings,
            room: options.start_room,
            mics: Vec::new(),
            assistants: Vec::new(),
            preferred_assistant: None,
            notice: None,
            shortcut_problem: None,
            tray: None,
            tray_devices: Vec::new(),
            screenshot: options.screenshot,
        };
        let mut tasks = vec![Task::perform(store::load(), Message::Loaded), list_mics()];
        if !options.hidden {
            tasks.push(open_window());
        }
        (app, Task::batch(tasks))
    }

    pub fn title(&self, _window: window::Id) -> String {
        "Home".into()
    }

    pub fn theme(&self, _window: window::Id) -> Theme {
        self.theme.clone()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            window::close_requests().map(Message::CloseRequested),
            window::close_events().map(Message::WindowClosed),
        ];
        // A layout screenshot shouldn't add a tray icon or ask Plasma for a shortcut.
        if self.screenshot.is_none() {
            subscriptions.push(Subscription::run(tray::run).map(Message::Tray));
            subscriptions.push(Subscription::run(shortcut::run).map(Message::Shortcut));
        }
        if let Some(session) = &self.session {
            subscriptions
                .push(Subscription::run_with(session.clone(), ws::connect).map(Message::Ha));
        }
        Subscription::batch(subscriptions)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Loaded(Some(refresh_token)) => {
                self.screen = Screen::Home;
                self.session = Some(Session {
                    server: self.config.server.clone(),
                    refresh_token,
                });
            }
            Message::Loaded(None) => {
                self.screen = Screen::Login(Box::new(login::Login::new(
                    self.config.server.clone(),
                    None,
                )));
            }
            Message::WindowOpened(id) => {
                self.window = Some(id);
                if self.screenshot.is_some() {
                    let delay = std::env::var("IOT_DESKTOP_SCREENSHOT_DELAY")
                        .ok()
                        .and_then(|seconds| seconds.parse().ok())
                        .unwrap_or(5);
                    return Task::perform(tokio::time::sleep(Duration::from_secs(delay)), |_| {
                        Message::TakeScreenshot
                    });
                }
            }
            Message::CloseRequested(id) => {
                // With a tray icon, closing hides the window; without one, it quits.
                if self.tray.is_none() {
                    return iced::exit();
                }
                self.window = None;
                return window::close(id);
            }
            Message::WindowClosed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                }
            }
            Message::Login(message) => {
                let Screen::Login(login) = &mut self.screen else {
                    return Task::none();
                };
                match login.update(message) {
                    login::Action::None => {}
                    login::Action::Run(task) => return task.map(Message::Login),
                    login::Action::Done {
                        server,
                        refresh_token,
                    } => {
                        self.config.server.clone_from(&server);
                        self.save_config();
                        self.screen = Screen::Home;
                        self.link = Link::Connecting;
                        self.session = Some(Session {
                            server,
                            refresh_token,
                        });
                    }
                }
            }
            Message::SignOut => {
                let session = self.session.take();
                self.sign_out(None);
                return Task::future(async move {
                    if let Some(session) = session {
                        let _ = iot_desktop::ha::auth::revoke(
                            &iot_desktop::ha::auth::http_client(),
                            &session.server,
                            &session.refresh_token,
                        )
                        .await;
                    }
                    store::clear().await;
                })
                .discard();
            }
            Message::Ha(event) => return self.on_ha(event),
            Message::Call(entity_id, service, data) => {
                self.send(Command::call(&entity_id, &service, data));
            }
            Message::Talk => {
                if self.voice.phase == Phase::Listening {
                    self.send(Command::StopListening);
                } else if self.handle.is_some() {
                    self.send(Command::Listen(Audio::Mic(self.config.mic.clone())));
                } else {
                    self.voice.fail("Not connected to Home Assistant.".into());
                }
            }
            Message::CommandChanged(command) => self.command = command,
            Message::SendCommand => {
                let command = std::mem::take(&mut self.command);
                if !command.trim().is_empty() {
                    self.send(Command::Say(command.trim().to_owned()));
                }
            }
            Message::ToggleSettings => {
                self.settings = !self.settings;
                if self.settings {
                    return list_mics();
                }
            }
            Message::Room(room) => self.room = room,
            Message::RefreshMics => return list_mics(),
            Message::Mics(Ok(mics)) => self.mics = mics,
            Message::Mics(Err(error)) => tracing::warn!("can't list microphones: {error}"),
            Message::MicSelected(choice) => {
                self.config.mic = choice.name;
                self.save_config();
                self.update_always_listen();
            }
            Message::WakeWord(on) => {
                self.config.wake_word = on;
                self.save_config();
                self.update_always_listen();
            }
            Message::AssistantSelected(choice) => {
                self.config.assistant = choice.id;
                self.save_config();
                self.use_assistant();
            }
            Message::DismissNotice => self.notice = None,
            Message::Tray(event) => return self.on_tray(event),
            Message::Shortcut(shortcut::Event::Talk) => return self.update(Message::Talk),
            Message::Shortcut(shortcut::Event::Unavailable(problem)) => {
                self.shortcut_problem = Some(problem)
            }
            Message::TakeScreenshot => {
                if let Some(id) = self.window {
                    return window::screenshot(id).map(Message::Screenshot);
                }
            }
            Message::Screenshot(shot) => {
                if let Some(path) = &self.screenshot {
                    if let Err(err) = save_png(path, &shot) {
                        eprintln!("couldn't save the screenshot: {err:#}");
                    }
                    return iced::exit();
                }
            }
        }
        Task::none()
    }

    fn on_ha(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Online(handle) => {
                self.handle = Some(handle);
                self.link = Link::Online;
                self.use_assistant();
            }
            Event::Offline(reason) => {
                self.handle = None;
                self.link = Link::Offline(reason);
                if self.voice.phase != Phase::Idle {
                    self.voice
                        .fail("Lost the connection to Home Assistant.".into());
                }
            }
            Event::SignedOut => {
                self.session = None;
                self.sign_out(Some(
                    "Your login expired or was revoked. Sign in again.".into(),
                ));
                return Task::future(store::clear()).discard();
            }
            Event::Snapshot { states, registry } => {
                self.states = states
                    .into_iter()
                    .map(|state| (state.entity_id.clone(), state))
                    .collect();
                self.registry = registry;
                self.loaded = true;
                return self.sync_tray();
            }
            Event::StateChanged { entity_id, state } => {
                match state {
                    Some(state) => self.states.insert(entity_id, state),
                    None => self.states.remove(&entity_id),
                };
                return self.sync_tray();
            }
            Event::Voice(event) => {
                if let Some(message) = self.voice.apply(event)
                    && self.window.is_none()
                {
                    return notify(message);
                }
            }
            Event::Assistants { list, preferred } => {
                // The chosen assistant was deleted in HA: fall back to HA's preferred one.
                if let Some(id) = &self.config.assistant
                    && !list.iter().any(|assistant| &assistant.id == id)
                {
                    self.config.assistant = None;
                    self.save_config();
                    self.use_assistant();
                }
                self.assistants = list;
                self.preferred_assistant = preferred;
            }
            Event::Error(error) => self.notice = Some(error),
        }
        Task::none()
    }

    fn on_tray(&mut self, event: tray::Event) -> Task<Message> {
        match event {
            tray::Event::Ready(handle) => {
                self.tray = Some(handle);
                self.tray_devices.clear();
                return self.sync_tray();
            }
            tray::Event::Open => {
                return match self.window {
                    Some(id) => window::gain_focus(id),
                    None => open_window(),
                };
            }
            tray::Event::Talk => return self.update(Message::Talk),
            tray::Event::Toggle(entity_id) => {
                if let Some(state) = self.states.get(&entity_id) {
                    let service = if state.is_on() { "turn_off" } else { "turn_on" };
                    self.send(Command::call(&entity_id, service, json!({})));
                }
            }
            tray::Event::Quit => return iced::exit(),
        }
        Task::none()
    }

    fn send(&mut self, command: Command) {
        match &self.handle {
            Some(handle) => handle.send(command),
            None => self.notice = Some("Not connected to Home Assistant.".into()),
        }
    }

    fn sign_out(&mut self, error: Option<String>) {
        self.handle = None;
        self.link = Link::Connecting;
        self.states.clear();
        self.registry = Registry::default();
        self.loaded = false;
        self.voice = Voice::default();
        self.settings = false;
        self.room = None;
        self.screen = Screen::Login(Box::new(login::Login::new(
            self.config.server.clone(),
            error,
        )));
    }

    fn save_config(&self) {
        if let Err(err) = self.config.save() {
            tracing::warn!("couldn't save settings: {err:#}");
        }
    }

    /// Tells the connection which assistant to use, then restarts always-listen on it.
    fn use_assistant(&mut self) {
        if let Some(handle) = &self.handle {
            handle.send(Command::UseAssistant(self.config.assistant.clone()));
        }
        self.update_always_listen();
    }

    fn update_always_listen(&mut self) {
        if let Some(handle) = &self.handle {
            let audio = self
                .config
                .wake_word
                .then(|| Audio::Mic(self.config.mic.clone()));
            handle.send(Command::AlwaysListen(audio));
        }
    }

    /// Mirrors the devices into the tray menu when they change.
    fn sync_tray(&mut self) -> Task<Message> {
        let Some(handle) = self.tray.clone() else {
            return Task::none();
        };
        let mut devices: Vec<tray::Device> = self
            .states
            .values()
            .filter(|state| state.controllable() && !self.registry.is_hidden(&state.entity_id))
            .map(|state| tray::Device {
                entity_id: state.entity_id.clone(),
                name: state.name().to_owned(),
                on: state.is_on(),
                available: state.available(),
            })
            .collect();
        devices.sort_by_key(|device| device.name.to_lowercase());
        if devices == self.tray_devices {
            return Task::none();
        }
        self.tray_devices.clone_from(&devices);
        Task::future(handle.set_devices(devices)).discard()
    }

    pub fn view(&self, _window: window::Id) -> Element<'_, Message> {
        match &self.screen {
            Screen::Starting => container(
                column![
                    icon(Icon::House, 40.0, style::FAINT),
                    text("Home")
                        .font(style::SEMIBOLD)
                        .size(18)
                        .style(style::muted),
                ]
                .spacing(12)
                .align_x(Alignment::Center),
            )
            .center(Length::Fill)
            .into(),
            Screen::Login(login) => login.view().map(Message::Login),
            Screen::Home => self.home(),
        }
    }

    fn home(&self) -> Element<'_, Message> {
        let sections = dashboard::sections(&self.states, &self.registry);
        let summary = if self.loaded {
            dashboard::summary(&sections)
        } else {
            "Connecting to your Home Assistant…".to_owned()
        };

        let settings_button = button(
            container(icon(
                Icon::Settings,
                20.0,
                if self.settings {
                    Color::WHITE
                } else {
                    style::TEXT
                },
            ))
            .center(Length::Fill),
        )
        .width(44)
        .height(44)
        .padding(0)
        .on_press(Message::ToggleSettings)
        .style(if self.settings {
            style::solid(style::BLUE, 22.0)
        } else {
            style::solid(style::SURFACE, 22.0)
        });
        let header = row![
            column![
                text("Home").font(style::BOLD).size(34),
                text(summary).size(14).style(style::muted),
            ]
            .spacing(2),
            space::horizontal(),
            self.connection(),
            settings_button,
        ]
        .spacing(12)
        .align_y(Alignment::Center);

        let mut page = column![header].spacing(20).padding([22, 28]);
        if let Some(notice) = &self.notice {
            page = page.push(
                container(
                    row![
                        icon(Icon::CircleAlert, 18.0, style::RED),
                        text(notice.as_str()).size(14).width(Length::Fill),
                        button(container(icon(Icon::X, 16.0, style::TEXT)).center(Length::Fill))
                            .width(30)
                            .height(30)
                            .padding(0)
                            .on_press(Message::DismissNotice)
                            .style(style::ghost(style::TEXT)),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .padding([8, 14])
                .style(style::notice),
            );
        }
        if !self.settings && sections.len() > 1 {
            page = page.push(self.rooms(&sections));
        }
        let body = if self.settings {
            settings::view(
                &self.config,
                &self.mics,
                &self.assistants,
                self.preferred_assistant.as_deref(),
                self.shortcut_problem.as_deref(),
            )
        } else {
            let room = self
                .room
                .as_deref()
                .filter(|room| sections.iter().any(|section| section.area == *room));
            dashboard::view(sections, room, self.loaded)
        };
        page.push(container(body).height(Length::Fill))
            .push(voice::view(
                &self.voice,
                &self.command,
                self.handle.is_some(),
            ))
            .into()
    }

    /// A pill showing whether the app is connected, with the reason on hover when it isn't.
    fn connection(&self) -> Element<'_, Message> {
        let (color, label) = match &self.link {
            Link::Online => (style::GREEN, "Connected"),
            Link::Connecting => (style::ORANGE, "Connecting"),
            Link::Offline(_) => (style::RED, "Offline"),
        };
        let dot = container(text(""))
            .width(8)
            .height(8)
            .style(style::badge(color));
        let pill = container(
            row![dot, text(label).font(style::MEDIUM).size(13)]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .padding([10, 14])
        .style(style::badge(style::SURFACE));
        match &self.link {
            Link::Offline(reason) => tooltip(
                pill,
                container(text(reason.as_str()).size(13))
                    .padding([8, 12])
                    .max_width(420)
                    .style(style::tile),
                tooltip::Position::Bottom,
            )
            .gap(8)
            .into(),
            _ => pill.into(),
        }
    }

    /// Chips to show one area's devices, or all of them.
    fn rooms(&self, sections: &[dashboard::Section<'_>]) -> Element<'_, Message> {
        let chip = |label: String, count: usize, room: Option<String>| {
            let active = self.room == room
                || (room.is_none()
                    && self
                        .room
                        .as_ref()
                        .is_none_or(|current| !sections.iter().any(|s| s.area == current)));
            let (text_color, count_color) = if active {
                (style::BACKGROUND, style::tint(style::BACKGROUND, 0.6))
            } else {
                (style::TEXT, style::FAINT)
            };
            button(
                row![
                    text(label).font(style::MEDIUM).size(14).color(text_color),
                    text(count.to_string()).size(13).color(count_color),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding([9, 16])
            .on_press(Message::Room(room))
            .style(move |theme, status| {
                if active {
                    style::choice(true, style::TEXT, 999.0)(theme, status)
                } else {
                    style::solid(style::SURFACE, 999.0)(theme, status)
                }
            })
            .into()
        };
        let total = sections.iter().map(|section| section.states.len()).sum();
        let mut chips = vec![chip("All".into(), total, None)];
        for section in sections {
            chips.push(chip(
                section.area.to_owned(),
                section.states.len(),
                Some(section.area.to_owned()),
            ));
        }
        Row::with_children(chips).spacing(8).wrap().into()
    }
}

fn open_window() -> Task<Message> {
    let (_, opened) = window::open(window::Settings {
        size: Size::new(1100.0, 760.0),
        min_size: Some(Size::new(620.0, 480.0)),
        exit_on_close_request: false,
        platform_specific: window::settings::PlatformSpecific {
            application_id: APP_ID.into(),
            ..Default::default()
        },
        ..Default::default()
    });
    opened.map(Message::WindowOpened)
}

fn list_mics() -> Task<Message> {
    Task::perform(
        async {
            tokio::task::spawn_blocking(mic::list)
                .await
                .map_err(|err| err.to_string())?
                .map_err(|err| format!("{err:#}"))
        },
        Message::Mics,
    )
}

fn notify(body: String) -> Task<Message> {
    Task::future(async move {
        let _ = notify_rust::Notification::new()
            .appname("Home")
            .summary("Home")
            .body(&body)
            .icon("go-home")
            .show_async()
            .await;
    })
    .discard()
}

fn save_png(path: &std::path::Path, shot: &window::Screenshot) -> anyhow::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, shot.size.width, shot.size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&shot.rgba)?;
    Ok(())
}
