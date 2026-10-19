pub mod assist;
pub mod auth;
pub mod entity;
pub mod store;
pub mod ws;

/// `http://host:8123` plus a path, without doubling the slash.
pub fn url(server: &str, path: &str) -> String {
    format!("{}{}", server.trim_end_matches('/'), path)
}
