//! Signing in: server, username and password, then the authenticator code if HA asks.

use iced::widget::{button, column, container, row, text, text_input};
use iced::{Alignment, Element, Length, Task};
use iot_desktop::ha::auth::{self, LoginFlow, Step};
use iot_desktop::ha::store;
use serde_json::{Map, Value};

use super::icon::{Icon, icon};
use super::style;

#[derive(Debug, Clone)]
pub enum Message {
    Server(String),
    Username(String),
    Password(String),
    Code(String),
    Submit,
    Stepped(Result<(LoginFlow, Step), String>),
    Saved(Result<String, String>),
}

/// What the app does after a login message.
pub enum Action {
    None,
    Run(Task<Message>),
    /// Logged in; the refresh token is stored.
    Done {
        server: String,
        refresh_token: String,
    },
}

pub struct Login {
    server: String,
    username: String,
    password: String,
    code: String,
    flow: Option<LoginFlow>,
    asking_code: bool,
    busy: bool,
    error: Option<String>,
}

impl Login {
    pub fn new(server: String, error: Option<String>) -> Self {
        Self {
            server,
            username: String::new(),
            password: String::new(),
            code: String::new(),
            flow: None,
            asking_code: false,
            busy: false,
            error,
        }
    }

    pub fn update(&mut self, message: Message) -> Action {
        match message {
            Message::Server(server) => self.server = server,
            Message::Username(username) => self.username = username,
            Message::Password(password) => self.password = password,
            Message::Code(code) => {
                self.code = code.chars().filter(char::is_ascii_digit).take(6).collect();
            }
            Message::Submit => {
                if self.busy {
                    return Action::None;
                }
                self.busy = true;
                self.error = None;
                let http = auth::http_client();
                if self.asking_code
                    && let Some(flow) = self.flow.clone()
                {
                    let code = self.code.clone();
                    return Action::Run(Task::perform(
                        async move {
                            let step = auth::submit(&http, &flow, field("code", &code)).await?;
                            anyhow::Ok((flow, step))
                        },
                        |result| Message::Stepped(result.map_err(|err| format!("{err:#}"))),
                    ));
                }
                let server = self.server.trim().trim_end_matches('/').to_owned();
                self.server.clone_from(&server);
                let mut credentials = field("username", self.username.trim());
                credentials.insert("password".into(), self.password.clone().into());
                return Action::Run(Task::perform(
                    async move {
                        let (flow, _) = auth::start(&http, &server).await?;
                        let step = auth::submit(&http, &flow, credentials).await?;
                        anyhow::Ok((flow, step))
                    },
                    |result| Message::Stepped(result.map_err(|err| format!("{err:#}"))),
                ));
            }
            Message::Stepped(Err(error)) => {
                self.busy = false;
                self.error = Some(error);
                self.flow = None;
                self.asking_code = false;
            }
            Message::Stepped(Ok((flow, step))) => match step {
                Step::Done { code } => {
                    return Action::Run(Task::perform(
                        async move {
                            let tokens = auth::exchange(&auth::http_client(), &flow, &code).await?;
                            store::save(&tokens.refresh_token).await?;
                            anyhow::Ok(tokens.refresh_token)
                        },
                        |result| Message::Saved(result.map_err(|err| format!("{err:#}"))),
                    ));
                }
                // Several MFA modules: the authenticator app is the one this screen asks for.
                Step::Form {
                    step_id, options, ..
                } if step_id == "select_mfa_module" => {
                    let module = options
                        .iter()
                        .find(|option| *option == "totp")
                        .or(options.first())
                        .cloned()
                        .unwrap_or_default();
                    let http = auth::http_client();
                    return Action::Run(Task::perform(
                        async move {
                            let step = auth::submit(
                                &http,
                                &flow,
                                field("multi_factor_auth_module", &module),
                            )
                            .await?;
                            anyhow::Ok((flow, step))
                        },
                        |result| Message::Stepped(result.map_err(|err| format!("{err:#}"))),
                    ));
                }
                Step::Form { fields, error, .. } => {
                    self.busy = false;
                    self.error = error;
                    if fields.iter().any(|field| field == "code") {
                        self.flow = Some(flow);
                        self.asking_code = true;
                        self.code.clear();
                    } else {
                        // Wrong password: the next try starts a fresh login.
                        self.flow = None;
                        self.asking_code = false;
                    }
                }
            },
            Message::Saved(Ok(refresh_token)) => {
                self.busy = false;
                self.password.clear();
                return Action::Done {
                    server: self.server.clone(),
                    refresh_token,
                };
            }
            Message::Saved(Err(error)) => {
                self.busy = false;
                self.error = Some(format!("Logged in, but couldn't save the login: {error}"));
            }
        }
        Action::None
    }

    pub fn view(&self) -> Element<'_, Message> {
        let field = |label: &'static str,
                     placeholder: &'static str,
                     value: &str,
                     on_input: fn(String) -> Message| {
            column![
                text(label).font(style::MEDIUM).size(13).style(style::muted),
                text_input(placeholder, value)
                    .on_input_maybe((!self.busy).then_some(on_input))
                    .on_submit(Message::Submit)
                    .padding([12, 14])
                    .size(15)
                    .style(style::input(12.0)),
            ]
            .spacing(6)
        };
        let (glyph, title, subtitle) = if self.asking_code {
            (
                Icon::ShieldCheck,
                "Two-factor login",
                "Enter the 6-digit code from your authenticator app.",
            )
        } else {
            (
                Icon::House,
                "Welcome home",
                "Sign in with your Home Assistant account.",
            )
        };
        let badge = container(icon(glyph, 28.0, style::BLUE))
            .center(Length::Fixed(60.0))
            .style(style::badge(style::tint(style::BLUE, 0.16)));
        let mut form = column![
            badge,
            column![
                text(title).font(style::BOLD).size(26),
                text(subtitle).size(14).style(style::muted),
            ]
            .spacing(6),
        ]
        .spacing(20)
        .width(Length::Fixed(360.0));

        if self.asking_code {
            form = form.push(
                text_input("123456", &self.code)
                    .on_input_maybe((!self.busy).then_some(Message::Code as fn(String) -> Message))
                    .on_submit(Message::Submit)
                    .padding([14, 16])
                    .size(24)
                    .font(style::MEDIUM)
                    .align_x(Alignment::Center)
                    .style(style::input(12.0)),
            );
        } else {
            form = form.push(
                column![
                    field(
                        "Server",
                        "http://stella-server:8123",
                        &self.server,
                        Message::Server
                    ),
                    field("Username", "", &self.username, Message::Username),
                    column![
                        text("Password")
                            .font(style::MEDIUM)
                            .size(13)
                            .style(style::muted),
                        text_input("", &self.password)
                            .on_input_maybe(
                                (!self.busy).then_some(Message::Password as fn(String) -> Message)
                            )
                            .on_submit(Message::Submit)
                            .secure(true)
                            .padding([12, 14])
                            .size(15)
                            .style(style::input(12.0)),
                    ]
                    .spacing(6),
                ]
                .spacing(14),
            );
        }
        if let Some(error) = &self.error {
            form = form.push(
                container(
                    row![
                        icon(Icon::CircleAlert, 16.0, style::RED),
                        text(error).size(13).width(Length::Fill),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                )
                .padding([10, 14])
                .width(Length::Fill)
                .style(style::notice),
            );
        }
        let ready = if self.asking_code {
            self.code.len() == 6
        } else {
            !self.username.trim().is_empty() && !self.password.is_empty()
        };
        let label = match (self.busy, self.asking_code) {
            (true, _) => "Signing in…",
            (false, true) => "Verify",
            (false, false) => "Sign in",
        };
        form = form.push(
            button(
                text(label)
                    .font(style::SEMIBOLD)
                    .size(15)
                    .width(Length::Fill)
                    .align_x(Alignment::Center),
            )
            .on_press_maybe((ready && !self.busy).then_some(Message::Submit))
            .style(style::solid(style::BLUE, 14.0))
            .padding(14)
            .width(Length::Fill),
        );
        container(container(form).padding(36).style(style::tile))
            .center(Length::Fill)
            .into()
    }
}

fn field(name: &str, value: &str) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(name.into(), value.into());
    map
}
