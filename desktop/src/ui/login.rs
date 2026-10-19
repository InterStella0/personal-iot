//! Signing in: server, username and password, then the authenticator code if HA asks.

use iced::widget::{button, column, container, text, text_input};
use iced::{Alignment, Element, Length, Task};
use iot_desktop::ha::auth::{self, LoginFlow, Step};
use iot_desktop::ha::store;
use serde_json::{Map, Value};

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
        let input = |placeholder, value: &str, on_input: fn(String) -> Message| {
            text_input(placeholder, value)
                .on_input_maybe((!self.busy).then_some(on_input))
                .on_submit(Message::Submit)
                .padding(10)
                .size(16)
        };
        let mut form = column![
            text("Home").size(32),
            text("Sign in to your Home Assistant").style(text::secondary),
        ]
        .spacing(10)
        .width(Length::Fixed(360.0));

        if self.asking_code {
            form = form
                .push(text("Enter the 6-digit code from your authenticator app."))
                .push(input("123456", &self.code, Message::Code));
        } else {
            form = form
                .push(input(
                    "http://stella-server:8123",
                    &self.server,
                    Message::Server,
                ))
                .push(input("Username", &self.username, Message::Username))
                .push(input("Password", &self.password, Message::Password).secure(true));
        }
        if let Some(error) = &self.error {
            form = form.push(text(error).style(text::danger));
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
            button(text(label).width(Length::Fill).align_x(Alignment::Center))
                .on_press_maybe((ready && !self.busy).then_some(Message::Submit))
                .style(button::primary)
                .padding(10)
                .width(Length::Fill),
        );
        container(container(form).padding(28).style(style::card))
            .center(Length::Fill)
            .into()
    }
}

fn field(name: &str, value: &str) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(name.into(), value.into());
    map
}
