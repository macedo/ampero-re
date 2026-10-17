//! Identidade visual da GUI: papéis fixos das cores, escala tipográfica e espaçamentos.
//!
//! Cada cor tem **um** significado, para o estado se ler num relance:
//! - dourado ([`crate::widgets::GOLD`]) = valor ou ativo (bloco ligado, arco dos knobs);
//! - azul ([`CTRL`]) = CTRL;
//! - laranja ([`WARN`]) = atenção ("não salvo");
//! - verde ([`OK`]) / vermelho ([`ERROR`]) = conectado / desconectado ou erro;
//! - contorno claro ([`selected`]) = o que está selecionado.

use eframe::egui::{self, Color32, FontFamily, FontId, TextStyle};

pub const CTRL: Color32 = Color32::from_rgb(55, 138, 221);
pub const WARN: Color32 = Color32::from_rgb(240, 140, 50);
pub const OK: Color32 = Color32::from_rgb(80, 200, 120);
pub const ERROR: Color32 = Color32::from_rgb(220, 80, 80);

/// Escala tipográfica (pt): metadados, corpo, rótulo importante, seção, título do preset.
pub const META: f32 = 11.0;
pub const BODY: f32 = 13.5;
pub const LABEL: f32 = 15.0;
pub const SECTION: f32 = 18.0;
pub const TITLE: f32 = 24.0;

/// Grade de espaçamento (px).
pub const S1: f32 = 4.0;
pub const S2: f32 = 8.0;
pub const S3: f32 = 12.0;
pub const S4: f32 = 16.0;

/// Contorno do item selecionado: quase branco no escuro, quase preto no claro.
pub fn selected(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_gray(225)
    } else {
        Color32::from_gray(40)
    }
}

/// Laranja de atenção legível também sobre fundo claro.
pub fn warn(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        WARN
    } else {
        Color32::from_rgb(200, 100, 10)
    }
}

/// Aplica a escala de fontes, o contraste e os espaçamentos aos dois temas (claro e escuro).
pub fn apply(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (
                TextStyle::Small,
                FontId::new(META, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(BODY, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(BODY, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(BODY - 0.5, FontFamily::Monospace),
            ),
            (
                TextStyle::Heading,
                FontId::new(SECTION, FontFamily::Proportional),
            ),
        ]
        .into();
        // texto "fraco" mais claro: o padrão do egui (60%) fica abaixo do contraste mínimo no escuro
        style.visuals.weak_text_alpha = 0.78;
        style.spacing.item_spacing = egui::vec2(S2, 6.0);
        style.spacing.button_padding = egui::vec2(S2, S1);
        style.spacing.interact_size.y = 22.0;
    });
}
