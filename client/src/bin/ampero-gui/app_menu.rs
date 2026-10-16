//! Menu ⚙ do app (tema, sobre, versão, verificar atualizações) e a faixa que avisa de versão nova,
//! com "Atualizar e reiniciar" no Windows.

use eframe::egui::{self, RichText};
use std::sync::mpsc::Receiver;

use crate::mcp_server::{McpServer, UiCall};
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
    mcp: Option<McpServer>,
    /// Erro ao ligar o servidor MCP (porta ocupada…); não tenta de novo até mudar a opção.
    mcp_error: Option<String>,
    show_mcp: bool,
    /// Resultado do botão "Configurar o Claude Desktop".
    mcp_install_status: Option<String>,
    /// Requisições vistas e quando chegou a última (o indicador pisca logo depois).
    mcp_seen_requests: usize,
    mcp_activity: Option<std::time::Instant>,
}

/// Por quanto tempo o indicador pisca depois de uma requisição.
const MCP_BLINK: std::time::Duration = std::time::Duration::from_millis(1500);

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
            mcp: None,
            mcp_error: None,
            show_mcp: false,
            mcp_install_status: None,
            mcp_seen_requests: 0,
            mcp_activity: None,
        }
    }

    /// Liga ou desliga o servidor MCP conforme a configuração e devolve as chamadas de ferramenta que
    /// chegaram (o app executa, porque tem o preset atual e a conexão com a pedaleira).
    pub fn mcp_calls(&mut self, ctx: &egui::Context) -> Vec<UiCall> {
        let want = self.settings.mcp_enabled;
        match (&self.mcp, want) {
            (None, true) if self.mcp_error.is_none() => {
                match McpServer::start(ctx, self.settings.mcp_port) {
                    Ok(s) => self.mcp = Some(s),
                    Err(e) => self.mcp_error = Some(e),
                }
            }
            (Some(_), false) => self.mcp = None,
            _ => {}
        }
        let Some(server) = &self.mcp else {
            self.mcp_seen_requests = 0;
            return Vec::new();
        };
        let count = server.request_count();
        if count != self.mcp_seen_requests {
            self.mcp_seen_requests = count;
            self.mcp_activity = Some(std::time::Instant::now());
        }
        server.take_calls()
    }

    fn open_mcp_window(&mut self) {
        self.show_mcp = true;
        self.mcp_install_status = None;
    }

    /// Indicador do servidor MCP na barra superior, sempre visível: verde ligado, vermelho desligado,
    /// laranja com erro. Pisca enquanto chegam requisições. Clicar abre a janela "Conectar ao Claude".
    pub fn mcp_indicator(&mut self, ui: &mut egui::Ui, act: bool) {
        let (mut color, hover) = match (&self.mcp, &self.mcp_error) {
            (Some(s), _) => (
                egui::Color32::from_rgb(80, 200, 120),
                format!(
                    "Servidor MCP ligado em 127.0.0.1:{}: o Claude Desktop/Claude Code pode montar timbres.\n\
                     Pisca quando está sendo usado. Clique para ver ou mudar a conexão.",
                    s.port
                ),
            ),
            (None, Some(e)) => (
                egui::Color32::from_rgb(240, 160, 60),
                format!("Servidor MCP com erro: {e}\nClique para ver."),
            ),
            (None, None) if self.settings.mcp_enabled => {
                (egui::Color32::GRAY, "Servidor MCP iniciando…".to_string())
            }
            (None, None) => (
                egui::Color32::from_rgb(220, 80, 80),
                "Servidor MCP desligado: o Claude não consegue montar timbres no app.\nClique para ligar."
                    .to_string(),
            ),
        };
        if self.mcp.is_some()
            && let Some(at) = self.mcp_activity
        {
            let age = at.elapsed();
            if age < MCP_BLINK {
                // alterna aceso/apagado a cada 120 ms
                if (age.as_millis() / 120) % 2 == 1 {
                    color = color.gamma_multiply(0.25);
                }
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(40));
            }
        }
        // layout da direita para a esquerda: o texto entra primeiro e a bolinha fica à esquerda dele
        let label = egui::Label::new(RichText::new("MCP").strong().color(color))
            .sense(egui::Sense::click());
        let text = ui.add(label);
        let (rect, dot) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::click());
        ui.painter().circle_filled(rect.center(), 4.5, color);
        let resp = text
            .union(dot)
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(hover);
        if resp.clicked() && act {
            self.open_mcp_window();
        }
    }

    fn set_mcp_enabled(&mut self, on: bool) {
        self.settings.mcp_enabled = on;
        self.mcp_error = None;
        self.settings.save();
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
            if ui
                .button("Conectar ao Claude…")
                .on_hover_text("Servidor MCP: o Claude Desktop ou o Claude Code montam timbres com a sua assinatura")
                .clicked()
                && act
            {
                self.open_mcp_window();
                ui.close();
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

    /// Janela "Conectar ao Claude": estado do servidor MCP e como ligar o Claude Desktop/Claude Code.
    pub fn mcp_window(&mut self, ctx: &egui::Context, act: bool) {
        if !self.show_mcp {
            return;
        }
        let mut open = true;
        let exe = if cfg!(windows) {
            "ampero.exe"
        } else {
            "ampero"
        };
        let cli = std::env::current_exe()
            .ok()
            .and_then(|p| Some(p.parent()?.join(exe)))
            .unwrap_or_else(|| exe.into());
        let port = self.settings.mcp_port;
        egui::Window::new("Conectar ao Claude")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.set_max_width(560.0);
                ui.label(
                    "Com o servidor MCP ligado, o Claude Desktop (ou o Claude Code) monta timbres com as \
                     ferramentas do Ampero usando a sua assinatura, sem chave de API. Ele consulta o catálogo, \
                     lê o preset atual e aplica no buffer de edição; nada é salvo sem você.",
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let (color, text) = match (&self.mcp, &self.mcp_error) {
                        (Some(s), _) => (egui::Color32::from_rgb(80, 200, 120), format!("Ligado em 127.0.0.1:{}", s.port)),
                        (None, Some(e)) => (egui::Color32::from_rgb(220, 80, 80), format!("Erro: {e}")),
                        (None, None) => (egui::Color32::GRAY, "Desligado".to_string()),
                    };
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(rect.center(), 4.5, color);
                    ui.label(RichText::new(text).strong());
                    let label = if self.settings.mcp_enabled { "Desligar" } else { "Ligar" };
                    if ui.button(label).clicked() && act {
                        self.set_mcp_enabled(!self.settings.mcp_enabled);
                    }
                });
                if !cli.is_file() {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 110, 100),
                        format!("Não achei {} ao lado do app; a ponte do Claude Desktop precisa dele.", cli.display()),
                    );
                }

                ui.separator();
                ui.label(RichText::new("Claude Desktop").strong());
                ui.label("1. Ligue o servidor acima (ele fica ligado nas próximas vezes).");
                ui.horizontal(|ui| {
                    ui.label("2.");
                    if ui.button("Configurar o Claude Desktop").clicked() && act {
                        self.mcp_install_status = Some(match ampero::mcp::install_claude_desktop(&cli, port) {
                            Ok(files) => format!(
                                "Pronto: {} (backup .bak-ampero ao lado).",
                                files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>().join(", ")
                            ),
                            Err(e) => format!("Não deu para configurar: {e}"),
                        });
                    }
                    ui.label(RichText::new("adiciona \"ampero\" em mcpServers, sem mexer no resto").weak());
                });
                if let Some(s) = &self.mcp_install_status {
                    ui.label(RichText::new(s).small());
                }
                ui.label("3. Feche o Claude Desktop de vez (ícone na bandeja do sistema, Sair) e abra de novo.");
                ui.label("4. Peça, por exemplo: \"monte no Ampero o timbre do solo de Comfortably Numb\".");
                let snippet = serde_json::to_string_pretty(&serde_json::json!({
                    "mcpServers": {"ampero": ampero::mcp::claude_desktop_entry(&cli, port)}
                }))
                .unwrap_or_default();
                egui::CollapsingHeader::new("Configurar à mão").show(ui, |ui| {
                    ui.label("Em claude_desktop_config.json (no Claude Desktop: Configurações > Desenvolvedor > Editar config):");
                    let mut text = snippet.clone();
                    ui.add(egui::TextEdit::multiline(&mut text).code_editor().desired_width(f32::INFINITY));
                    if ui.small_button("Copiar").clicked() {
                        ui.ctx().copy_text(snippet.clone());
                    }
                });

                ui.separator();
                ui.label(RichText::new("Claude Code").strong());
                let cmd = format!("claude mcp add ampero -- \"{}\" mcp", cli.display());
                ui.horizontal(|ui| {
                    ui.code(&cmd);
                    if ui.small_button("Copiar").clicked() {
                        ui.ctx().copy_text(cmd.clone());
                    }
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Só programas desta máquina com o token (em mcp-token, na pasta de configuração) conseguem \
                         usar o servidor; páginas da web são recusadas.",
                    )
                    .small()
                    .weak(),
                );
            });
        self.show_mcp = open;
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
                ui.label("Editor multiplataforma para a pedaleira Hotone Ampero, que monta timbres com o Claude via MCP.");
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
