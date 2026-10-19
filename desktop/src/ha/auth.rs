//! HA's own login flow, driven natively: username and password, then the MFA code if the
//! account has one. HA accepts a loopback client id for native apps, and PKCE binds the
//! authorization code to this process.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::url;

pub const CLIENT_ID: &str = "http://127.0.0.1/iot-desktop";
const REDIRECT_URI: &str = "http://127.0.0.1/iot-desktop/callback";

pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .expect("TLS backend is available")
}

#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn new() -> Self {
        Self::from_verifier(URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()))
    }

    pub fn from_verifier(verifier: String) -> Self {
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self {
            verifier,
            challenge,
        }
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

/// An in-progress login.
#[derive(Debug, Clone)]
pub struct LoginFlow {
    pub server: String,
    flow_id: String,
    pkce: Pkce,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// HA wants these fields, e.g. `username` and `password`, or `code` for MFA.
    Form {
        step_id: String,
        fields: Vec<String>,
        /// Choices for a select field, e.g. the MFA modules when there are several.
        options: Vec<String>,
        error: Option<String>,
    },
    /// Logged in: exchange this code for tokens.
    Done { code: String },
}

#[derive(Deserialize)]
struct FlowResponse {
    #[serde(rename = "type")]
    kind: String,
    flow_id: Option<String>,
    step_id: Option<String>,
    #[serde(default)]
    data_schema: Vec<Value>,
    errors: Option<Map<String, Value>>,
    result: Option<Value>,
    reason: Option<String>,
}

fn parse_step(response: FlowResponse) -> Result<Step> {
    match response.kind.as_str() {
        "form" => {
            let fields = response
                .data_schema
                .iter()
                .filter_map(|field| field.get("name")?.as_str().map(str::to_owned))
                .collect();
            let options = response
                .data_schema
                .iter()
                .filter_map(|field| field.get("options"))
                .flat_map(|options| match options {
                    // [["totp", "Authenticator app"]] or {"totp": "Authenticator app"}
                    Value::Array(items) => items
                        .iter()
                        .filter_map(|item| item.get(0).or(Some(item))?.as_str().map(str::to_owned))
                        .collect::<Vec<_>>(),
                    Value::Object(map) => map.keys().cloned().collect(),
                    _ => Vec::new(),
                })
                .collect();
            let error = response
                .errors
                .unwrap_or_default()
                .values()
                .next()
                .and_then(Value::as_str)
                .map(describe_error);
            Ok(Step::Form {
                step_id: response.step_id.unwrap_or_default(),
                fields,
                options,
                error,
            })
        }
        "create_entry" => {
            let code = response
                .result
                .as_ref()
                .and_then(Value::as_str)
                .context("login finished without an authorization code")?;
            Ok(Step::Done {
                code: code.to_owned(),
            })
        }
        "abort" => bail!(
            "Home Assistant ended the login: {}",
            describe_error(response.reason.as_deref().unwrap_or("unknown"))
        ),
        other => bail!("unexpected login step type {other:?}"),
    }
}

fn describe_error(code: &str) -> String {
    match code {
        "invalid_auth" => "Wrong username or password.".to_owned(),
        "invalid_code" => "That code didn't work. Try the current one.".to_owned(),
        "too_many_retry" => "Too many wrong codes. Start again.".to_owned(),
        "login_expired" => "The login expired. Start again.".to_owned(),
        other => other.replace('_', " "),
    }
}

async fn read_json<T: for<'de> Deserialize<'de>>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    let body = response.text().await?;
    serde_json::from_str(&body).with_context(|| format!("HTTP {status}: {body}"))
}

pub async fn start(http: &reqwest::Client, server: &str) -> Result<(LoginFlow, Step)> {
    let pkce = Pkce::new();
    let response = http
        .post(url(server, "/auth/login_flow"))
        .json(&json!({
            "client_id": CLIENT_ID,
            "redirect_uri": REDIRECT_URI,
            "handler": ["homeassistant", null],
            "type": "authorize",
            "code_challenge": pkce.challenge,
            "code_challenge_method": "S256",
        }))
        .send()
        .await
        .with_context(|| format!("can't reach {server}"))?;
    let response: FlowResponse = read_json(response).await?;
    let flow_id = response.flow_id.clone().context("login flow has no id")?;
    let step = parse_step(response)?;
    Ok((
        LoginFlow {
            server: server.to_owned(),
            flow_id,
            pkce,
        },
        step,
    ))
}

pub async fn submit(
    http: &reqwest::Client,
    flow: &LoginFlow,
    mut input: Map<String, Value>,
) -> Result<Step> {
    input.insert("client_id".into(), CLIENT_ID.into());
    let response = http
        .post(url(
            &flow.server,
            &format!("/auth/login_flow/{}", flow.flow_id),
        ))
        .json(&input)
        .send()
        .await?;
    parse_step(read_json(response).await?)
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

pub async fn exchange(http: &reqwest::Client, flow: &LoginFlow, code: &str) -> Result<Tokens> {
    let response = http
        .post(url(&flow.server, "/auth/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", CLIENT_ID),
            ("code_verifier", &flow.pkce.verifier),
        ])
        .send()
        .await?;
    read_json(response).await
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccessToken {
    pub access_token: String,
    pub expires_in: u64,
}

#[derive(Debug)]
pub enum RefreshError {
    /// HA rejected the refresh token: it was revoked, so log in again.
    Rejected,
    /// Couldn't ask: HA is down or unreachable.
    Unreachable(anyhow::Error),
}

pub async fn refresh(
    http: &reqwest::Client,
    server: &str,
    refresh_token: &str,
) -> Result<AccessToken, RefreshError> {
    let response = http
        .post(url(server, "/auth/token"))
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ])
        .send()
        .await
        .map_err(|err| RefreshError::Unreachable(err.into()))?;
    let status = response.status();
    if status == reqwest::StatusCode::BAD_REQUEST || status == reqwest::StatusCode::FORBIDDEN {
        // {"error": "invalid_grant"}: the token is gone.
        return Err(RefreshError::Rejected);
    }
    if !status.is_success() {
        return Err(RefreshError::Unreachable(anyhow!("HTTP {status}")));
    }
    read_json(response).await.map_err(RefreshError::Unreachable)
}

pub async fn revoke(http: &reqwest::Client, server: &str, refresh_token: &str) -> Result<()> {
    http.post(url(server, "/auth/revoke"))
        .form(&[("token", refresh_token)])
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_rfc_7636_example() {
        let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());
        assert_eq!(
            pkce.challenge,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn new_pkce_verifier_is_43_url_safe_chars() {
        let pkce = Pkce::new();
        assert_eq!(pkce.verifier.len(), 43);
        assert!(
            pkce.verifier
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    fn step(value: Value) -> Step {
        parse_step(serde_json::from_value(value).unwrap()).unwrap()
    }

    #[test]
    fn parses_password_mfa_and_done_steps() {
        assert_eq!(
            step(
                json!({"type": "form", "flow_id": "f", "step_id": "init", "errors": {},
                "data_schema": [{"name": "username", "type": "string"}, {"name": "password", "type": "string"}]})
            ),
            Step::Form {
                step_id: "init".into(),
                fields: vec!["username".into(), "password".into()],
                options: vec![],
                error: None
            }
        );
        assert_eq!(
            step(
                json!({"type": "form", "flow_id": "f", "step_id": "mfa", "errors": {"base": "invalid_code"},
                "data_schema": [{"name": "code", "type": "string"}]})
            ),
            Step::Form {
                step_id: "mfa".into(),
                fields: vec!["code".into()],
                options: vec![],
                error: Some("That code didn't work. Try the current one.".into())
            }
        );
        assert_eq!(
            step(
                json!({"type": "form", "flow_id": "f", "step_id": "select_mfa_module", "errors": null,
                "data_schema": [{"name": "multi_factor_auth_module", "type": "select",
                    "options": [["totp", "Authenticator app"]]}]})
            ),
            Step::Form {
                step_id: "select_mfa_module".into(),
                fields: vec!["multi_factor_auth_module".into()],
                options: vec!["totp".into()],
                error: None
            }
        );
        assert_eq!(
            step(json!({"type": "create_entry", "flow_id": "f", "result": "abc"})),
            Step::Done { code: "abc".into() }
        );
    }
}
