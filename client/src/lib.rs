//! Protocolo, modelo de preset e conexão MIDI do Hotone Ampero (ver PROTOCOLO.md).

pub mod device;
pub mod logfile;
pub mod mcp;
pub mod params;
pub mod preset;
pub mod protocol;
pub mod resources;
pub mod timbre;

use std::path::PathBuf;

/// Pasta de configuração do usuário: `%APPDATA%\ampero`, `~/Library/Application Support/ampero` ou
/// `~/.config/ampero`.
pub fn config_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join("ampero"))
}
