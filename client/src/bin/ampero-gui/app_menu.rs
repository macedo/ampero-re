//! Menu ⚙ do app (tema, sobre, versão, verificar atualizações) e a faixa que avisa de versão nova,
//! com "Atualizar e reiniciar" no Windows.

use eframe::egui::{self, RichText};
use std::sync::mpsc::Receiver;

use crate::self_update::{self, AppRelease, CheckResult, UpdateMsg, VERSION};
use crate::settings::{Settings, Theme};
use crate::widgets::accent;

const REPO_URL: &str = "https://github.com/macedo/ampero-re";
const RELEASES_URL: &str = "https://github.com/macedo/ampero-re/releases";

pub struct AppMenu {
    settings: Settings,
    show_about: bool,
    check_rx: Option<Receiver<CheckResult>>,
    /// Verificação pedida pelo usuário: mostra também "já está na mais recente" e erros.
    manual_check: bool,
    update: Option<AppRelease>,
    update_dismissed: bool,
    download_rx: Option<Receiver<UpdateMsg>>,
    download_progress: f32,
    /// Mensagem na faixa (resultado da verificação manual, erro do download…).
    notice: Option<String>,
}

impl AppMenu {
    pub fn new(ctx: &egui::Context) -> AppMenu {
        let settings = Settings::load();
        settings.theme.apply(ctx);
        AppMenu {
            settings,
            show_about: false,
            // verificação silenciosa ao abrir
            check_rx: Some(self_update::check_for_update(ctx.clone())),
            manual_check: false,
            update: None,
            update_dismissed: false,
            download_rx: None,
            download_progress: 0.0,
            notice: None,
        }
    }

    /// Processa respostas das threads de verificação e download. Chamar uma vez por frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.check_rx
            && let Ok(result) = rx.try_recv()
        {
            self.check_rx = None;
            match result {
                Ok(Some(release)) => {
                    self.update = Some(release);
                    self.update_dismissed = false;
                    self.notice = None;
                }
                Ok(None) if self.manual_check => {
                    self.notice = Some(format!("Você já está na versão mais recente ({VERSION})."));
                }
                Err(e) if self.manual_check => {
                    self.notice = Some(format!("Verificação falhou: {e}"))
                }
                _ => {} // verificação automática: sem novidade ou offline, não incomoda
            }
        }

        let Some(rx) = &self.download_rx else { return };
        let msgs: Vec<UpdateMsg> = rx.try_iter().collect();
        for msg in msgs {
            match msg {
                UpdateMsg::Progress(f) => self.download_progress = f,
                UpdateMsg::Ready(path) => {
                    self.download_rx = None;
                    match self_update::run_installer(&path) {
                        // o instalador troca o executável e abre a versão nova
                        Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                        Err(e) => {
                            self.notice = Some(format!("Não deu para abrir o instalador: {e}"))
                        }
                    }
                    return;
                }
                UpdateMsg::Failed(e) => {
                    self.download_rx = None;
                    self.notice = Some(format!("Atualização falhou: {e}"));
                    return;
                }
            }
        }
    }

    fn start_check(&mut self, ctx: &egui::Context) {
        if self.check_rx.is_none() {
            self.manual_check = true;
            self.notice = Some("Procurando atualizações…".into());
            self.check_rx = Some(self_update::check_for_update(ctx.clone()));
        }
    }

    /// Botão ⚙ (no fim da barra superior). `act` = false nas passadas descartadas do egui.
    pub fn menu_button(&mut self, ui: &mut egui::Ui, act: bool) {
        ui.menu_button(RichText::new("⚙").size(16.0), |ui| {
            ui.label(RichText::new("Tema").weak());
            for (theme, label) in Theme::ALL {
                if ui
                    .radio_value(&mut self.settings.theme, theme, label)
                    .clicked()
                    && act
                {
                    theme.apply(ui.ctx());
                    self.settings.save();
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Verificar atualizações").clicked() && act {
                self.start_check(ui.ctx());
                ui.close();
            }
            if ui.button("Sobre o Ampero…").clicked() && act {
                self.show_about = true;
                ui.close();
            }
            ui.separator();
            ui.label(RichText::new(format!("Versão {VERSION}")).weak());
        })
        .response
        .on_hover_text("Configurações e ajuda");
    }

    /// A faixa só aparece com versão nova ou com uma mensagem para mostrar.
    pub fn wants_banner(&self) -> bool {
        (self.update.is_some() && !self.update_dismissed) || self.notice.is_some()
    }

    pub fn banner(&mut self, ui: &mut egui::Ui, act: bool) {
        ui.horizontal(|ui| {
            if let Some(release) = self.update.clone().filter(|_| !self.update_dismissed) {
                ui.label(RichText::new(format!("Ampero {} disponível", release.version)).strong().color(accent(ui)));
                ui.label(format!("(você tem {VERSION})"));
                ui.hyperlink_to("Novidades", &release.notes_url);
                if let Some(notice) = &self.notice {
                    ui.label(RichText::new(notice).weak());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.download_rx.is_some() {
                        ui.add(
                            egui::ProgressBar::new(self.download_progress)
                                .desired_width(160.0)
                                .show_percentage(),
                        );
                        ui.label("Baixando a atualização…");
                        return;
                    }
                    if ui.small_button("✖").on_hover_text("Dispensar").clicked() && act {
                        self.update_dismissed = true;
                        self.notice = None;
                    }
                    if release.can_self_update() {
                        let button = ui.button("⬇ Atualizar e reiniciar").on_hover_text(
                            "Baixa a versão nova, confere o SHA-256 com a release e instala; \
                             o app reinicia sozinho. Presets e ajustes ficam como estão.",
                        );
                        if button.clicked() && act {
                            self.notice = None;
                            self.download_progress = 0.0;
                            self.download_rx = Some(self_update::download_update(release.clone(), ui.ctx().clone()));
                        }
                    } else if ui
                        .button("Baixar na página da release")
                        .on_hover_text(if cfg!(windows) {
                            "Esta cópia não foi instalada pelo instalador, então não se atualiza sozinha"
                        } else {
                            "No macOS e no Linux, baixe o pacote novo na página da release"
                        })
                        .clicked()
                        && act
                    {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(&release.notes_url));
                    }
                });
            } else if let Some(notice) = self.notice.clone() {
                if self.check_rx.is_some() {
                    ui.spinner();
                }
                ui.label(notice);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✖").on_hover_text("Fechar").clicked() && act {
                        self.notice = None;
                    }
                });
            }
        });
    }

    pub fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_about;
        egui::Window::new("Sobre o Ampero")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.heading(format!("Ampero {VERSION}"));
                ui.label("Editor multiplataforma para a pedaleira Hotone Ampero, com assistente de timbre por IA.");
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "Projeto independente, sem vínculo com a Hotone. Hotone e Ampero são marcas dos seus donos.",
                        )
                        .strong(),
                    )
                    .wrap(),
                );
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        "Fotos e descrições dos efeitos vêm da instalação do Ampero Editor oficial, quando \
                         presente; elas não são distribuídas com este app.",
                    )
                    .wrap(),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.hyperlink_to("Código no GitHub", REPO_URL);
                    ui.label("•");
                    ui.hyperlink_to("Releases", RELEASES_URL);
                });
            });
        self.show_about = open;
    }
}
