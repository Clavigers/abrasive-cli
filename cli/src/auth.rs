//! Abrasive API token paste flow + local credentials cache.
//!
//! `paste_login()` prompts the user to paste a token from the dashboard
//! at https://abrasive.netlify.app/me and writes it to
//! `~/.abrasive/credentials.toml`. `saved_token()` reads it back for
//! subsequent requests against the daemon.

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::errors::AuthError;

const ABRASIVE_WEB_URL: &str = "https://abrasive.netlify.app";
const TOKEN_PREFIX: &str = "abrasive_";

pub fn paste_login() -> Result<String, AuthError> {
    eprintln!(
        "please paste the token found on {}/me below",
        ABRASIVE_WEB_URL
    );
    let _ = io::stderr().flush();

    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(AuthError::ReadStdin)?;

    let token = line.trim();
    if token.is_empty() {
        return Err(AuthError::EmptyToken);
    }
    if !token.starts_with(TOKEN_PREFIX) {
        return Err(AuthError::InvalidToken);
    }

    write_credentials(token)?;
    eprintln!("       Login token for `abrasive` saved");
    Ok(token.to_string())
}

pub fn saved_token() -> Option<String> {
    let path = credentials_path()?;
    let raw = fs::read_to_string(&path).ok()?;
    let parsed: toml::Value = toml::from_str(&raw).ok()?;
    parsed
        .get("abrasive")?
        .get("token")?
        .as_str()
        .map(String::from)
}

fn credentials_path() -> Option<PathBuf> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    Some(home.join(".abrasive").join("credentials.toml"))
}

fn write_credentials(token: &str) -> Result<(), AuthError> {
    let path = credentials_path().ok_or(AuthError::NoHome)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(AuthError::WriteToken)?;
    }
    let content = format!("[abrasive]\ntoken = \"{}\"\n", token);
    fs::write(&path, content).map_err(AuthError::WriteToken)?;
    chmod_600(&path);
    Ok(())
}

#[cfg(unix)]
fn chmod_600(path: &Path) {
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn chmod_600(_path: &Path) {}
