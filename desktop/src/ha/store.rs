//! Where the refresh token lives: KWallet (or any Secret Service), else a 0600 file.
//! `IOT_DESKTOP_TOKEN_FILE` forces the file, so tests never touch the real wallet.

use std::path::PathBuf;

use anyhow::Result;

const ATTRIBUTES: [(&str, &str); 1] = [("application", crate::config::APP_ID)];
const LABEL: &str = "Home Assistant login (iot-desktop)";

fn forced_file() -> Option<PathBuf> {
    std::env::var_os("IOT_DESKTOP_TOKEN_FILE").map(PathBuf::from)
}

fn fallback_file() -> PathBuf {
    crate::config::dir().join("token")
}

async fn keyring() -> Option<oo7::Keyring> {
    if forced_file().is_some() {
        return None;
    }
    match oo7::Keyring::new().await {
        Ok(keyring) => {
            let _ = keyring.unlock().await;
            Some(keyring)
        }
        Err(err) => {
            tracing::warn!("no secret service, storing the login in a file: {err}");
            None
        }
    }
}

pub async fn load() -> Option<String> {
    if let Some(keyring) = keyring().await {
        let items = keyring.search_items(&ATTRIBUTES).await.ok()?;
        let secret = items.first()?.secret().await.ok()?;
        return String::from_utf8(secret.as_bytes().to_vec()).ok();
    }
    let path = forced_file().unwrap_or_else(fallback_file);
    std::fs::read_to_string(path)
        .ok()
        .map(|token| token.trim().to_owned())
}

pub async fn save(refresh_token: &str) -> Result<()> {
    if let Some(keyring) = keyring().await {
        keyring
            .create_item(LABEL, &ATTRIBUTES, refresh_token, true)
            .await?;
        return Ok(());
    }
    let path = forced_file().unwrap_or_else(fallback_file);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    write_private(&path, refresh_token)
}

pub async fn clear() {
    if let Some(keyring) = keyring().await {
        let _ = keyring.delete(&ATTRIBUTES).await;
    }
    let _ = std::fs::remove_file(forced_file().unwrap_or_else(fallback_file));
}

fn write_private(path: &std::path::Path, contents: &str) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents.as_bytes())?;
    Ok(())
}
