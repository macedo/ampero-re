//! Preferências da interface, salvas em `gui.json` na pasta de configuração do usuário.

use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
    System,
}

impl Theme {
    pub const ALL: [(Theme, &'static str); 3] = [
        (Theme::System, "Seguir o sistema"),
        (Theme::Light, "Claro"),
        (Theme::Dark, "Escuro"),
    ];

    pub fn apply(self, ctx: &egui::Context) {
        ctx.set_theme(match self {
            Theme::System => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        });
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub theme: Theme,
    /// Servidor MCP ligado (Claude Desktop/Claude Code montam timbres pela GUI aberta).
    pub mcp_enabled: bool,
    pub mcp_port: u16,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: Theme::default(),
            mcp_enabled: false,
            mcp_port: ampero::mcp::DEFAULT_PORT,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        Some(ampero::config_dir()?.join("gui.json"))
    }

    pub fn load() -> Settings {
        Settings::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = Settings::path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}
