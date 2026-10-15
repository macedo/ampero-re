//! Painel "Assistente de timbre": conversa com um agente de IA que propõe timbres usando o que o Ampero tem.
//! A conversa roda numa thread própria (as respostas podem levar dezenas de segundos).

use ampero::assistant::{
    self, Config, Conversation, HttpTransport, Proposal, Provider, ResolvedBlock, ToolContext, Turn,
};
use ampero::preset::Preset;
use ampero::resources::Resources;
use eframe::egui::{self, Color32, RichText};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use crate::widgets::GOLD;

enum Cmd {
    Send {
        text: String,
        preset: Option<Preset>,
    },
    Reset(Config),
}

enum Evt {
    Activity(String),
    Reply(Turn),
    Error(String),
}

enum Item {
    User(String),
    Assistant(String),
    Proposal { proposal: Proposal, applied: bool },
    Error(String),
}

/// O que o painel pede para o app principal fazer.
pub enum Action {
    /// Aplicar estes blocos no buffer de edição.
    Apply(Vec<ResolvedBlock>),
}

pub struct AssistantPanel {
    tx: Sender<Cmd>,
    rx: Receiver<Evt>,
    config: Config,
    items: Vec<Item>,
    input: String,
    busy: bool,
    activity: String,
    settings: Option<Config>,
    settings_status: String,
    /// Painel recolhido (atualizado pela GUI a cada frame).
    collapsed: bool,
    /// Chegou resposta enquanto o painel estava recolhido.
    unread: bool,
}

const SUGGESTIONS: [&str; 4] = [
    "Monte o timbre do solo de Comfortably Numb (Pink Floyd)",
    "Quero a base de Back in Black (AC/DC)",
    "Um limpo cristalino estilo John Mayer, com um pouco de chorus",
    "Ajuste o preset atual para ficar mais encorpado nas bases",
];

fn worker(rx: Receiver<Cmd>, tx: Sender<Evt>, config: Config, repaint: impl Fn()) {
    let transport = HttpTransport::default();
    let resources = Resources::locate();
    let mut conv = Conversation::new(config);
    for cmd in rx {
        match cmd {
            Cmd::Reset(cfg) => conv = Conversation::new(cfg),
            Cmd::Send { text, preset } => {
                let ctx = ToolContext {
                    preset: preset.as_ref(),
                    resources: resources.as_ref(),
                };
                let mut activity = |a: &str| {
                    let _ = tx.send(Evt::Activity(a.to_string()));
                    repaint();
                };
                let evt = match conv.send(&text, &ctx, &transport, &mut activity) {
                    Ok(turn) => Evt::Reply(turn),
                    Err(e) => Evt::Error(format!("{e:#}")),
                };
                let _ = tx.send(evt);
                repaint();
            }
        }
    }
}

impl AssistantPanel {
    pub fn new(ctx: &egui::Context) -> AssistantPanel {
        let config = Config::load();
        let (cmd_tx, cmd_rx) = channel();
        let (evt_tx, evt_rx) = channel();
        let ctx = ctx.clone();
        let cfg = config.clone();
        thread::spawn(move || worker(cmd_rx, evt_tx, cfg, move || ctx.request_repaint()));
        AssistantPanel {
            tx: cmd_tx,
            rx: evt_rx,
            config,
            items: Vec::new(),
            input: String::new(),
            busy: false,
            activity: String::new(),
            settings: None,
            settings_status: String::new(),
            collapsed: false,
            unread: false,
        }
    }

    fn poll(&mut self) {
        while let Ok(evt) = self.rx.try_recv() {
            if !matches!(evt, Evt::Activity(_)) && self.collapsed {
                self.unread = true; // avisa na faixa recolhida que chegou resposta
            }
            match evt {
                Evt::Activity(a) => self.activity = a,
                Evt::Reply(turn) => {
                    self.busy = false;
                    if !turn.text.is_empty() {
                        self.items.push(Item::Assistant(turn.text));
                    }
                    for proposal in turn.proposals {
                        self.items.push(Item::Proposal {
                            proposal,
                            applied: false,
                        });
                    }
                }
                Evt::Error(e) => {
                    self.busy = false;
                    self.items.push(Item::Error(e));
                }
            }
        }
    }

    fn send(&mut self, text: String, preset: Option<&Preset>) {
        let text = text.trim().to_string();
        if text.is_empty() || self.busy {
            return;
        }
        self.items.push(Item::User(text.clone()));
        self.busy = true;
        self.activity = "pensando…".into();
        let _ = self.tx.send(Cmd::Send {
            text,
            preset: preset.cloned(),
        });
    }

    /// Faixa fina do painel recolhido. Retorna true quando o usuário pede para expandir.
    pub fn collapsed_ui(&mut self, ui: &mut egui::Ui, act: bool) -> bool {
        self.collapsed = true;
        self.poll();
        let mut expand = false;
        ui.vertical_centered(|ui| {
            ui.add_space(4.0);
            let button =
                egui::Button::new(RichText::new("«").size(18.0)).min_size(egui::vec2(28.0, 28.0));
            let resp = ui.add(button).on_hover_text("Abrir o assistente de timbre");
            if resp.clicked() && act {
                expand = true;
            }
            ui.label(RichText::new("IA").small().strong());
            if self.busy {
                ui.spinner();
            }
            if self.unread {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.0, GOLD);
                ui.label(RichText::new("nova\nresposta").small().color(GOLD));
            }
        });
        expand
    }

    /// Painel expandido. Retorna as ações para o app e se o usuário pediu para recolher.
    /// `act` = false na passada descartada do egui (efeitos colaterais só na primeira).
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        preset: Option<&Preset>,
        act: bool,
    ) -> (Vec<Action>, bool) {
        self.collapsed = false;
        self.unread = false;
        self.poll();
        let mut actions = Vec::new();
        let mut collapse = false;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Assistente de timbre").strong().size(16.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // na ponta direita: na esquerda ele ficaria sobre a alça de redimensionar do painel
                let button = egui::Button::new(RichText::new("»").size(16.0));
                let resp = ui.add(button).on_hover_text("Recolher o assistente");
                if resp.clicked() && act {
                    collapse = true;
                }
                if ui.button("Configurar").clicked() && act {
                    self.settings = Some(self.config.clone());
                    self.settings_status.clear();
                }
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Nova conversa"))
                    .clicked()
                    && act
                {
                    self.items.clear();
                    let _ = self.tx.send(Cmd::Reset(self.config.clone()));
                }
            });
        });
        ui.label(
            RichText::new(format!(
                "{} · {}",
                self.config.provider.label(),
                self.config.endpoint().model
            ))
            .small()
            .weak(),
        );
        ui.separator();

        let input_h = 92.0;
        egui::ScrollArea::vertical()
            .id_salt("assistant-chat")
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .max_height((ui.available_height() - input_h).max(80.0))
            .show(ui, |ui| {
                if self.items.is_empty() {
                    ui.label(
                        RichText::new(
                            "Peça um timbre de uma música, banda ou estilo. O assistente monta usando só o que o \
                             Ampero tem e mostra uma proposta para você aplicar.",
                        )
                        .weak(),
                    );
                    ui.add_space(6.0);
                    for s in SUGGESTIONS {
                        if ui.add(egui::Button::new(RichText::new(s).small()).wrap()).clicked() && act {
                            self.send(s.to_string(), preset);
                        }
                    }
                }
                for item in &mut self.items {
                    match item {
                        Item::User(t) => bubble(ui, t, true),
                        Item::Assistant(t) => bubble(ui, t, false),
                        Item::Error(e) => {
                            ui.colored_label(Color32::from_rgb(230, 110, 100), format!("Erro: {e}"));
                        }
                        Item::Proposal { proposal, applied } => {
                            if let Some(blocks) = proposal_card(ui, proposal, *applied)
                                && act {
                                    *applied = true;
                                    actions.push(Action::Apply(blocks));
                                }
                        }
                    }
                    ui.add_space(6.0);
                }
                if self.busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(&self.activity).weak());
                    });
                }
            });

        ui.separator();
        let edit = egui::TextEdit::multiline(&mut self.input)
            .hint_text(
                "Ex.: timbre da base de Enter Sandman (Enter envia, Shift+Enter quebra linha)",
            )
            .desired_rows(3)
            .desired_width(f32::INFINITY);
        let resp = ui.add_enabled(!self.busy, edit);
        let enter =
            resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
        ui.horizontal(|ui| {
            let clicked = ui
                .add_enabled(
                    !self.busy && !self.input.trim().is_empty(),
                    egui::Button::new("Enviar"),
                )
                .clicked();
            if (clicked || enter) && act {
                let text = std::mem::take(&mut self.input);
                self.send(text, preset);
            }
            if preset.is_none() {
                ui.label(
                    RichText::new("pedaleira desconectada: o agente não vê o preset atual")
                        .small()
                        .weak(),
                );
            }
        });

        self.settings_window(ui.ctx(), act);
        (actions, collapse)
    }

    fn settings_window(&mut self, ctx: &egui::Context, act: bool) {
        let Some(mut draft) = self.settings.take() else {
            return;
        };
        let mut keep_open = true;
        let mut save = false;
        egui::Window::new("Configurar assistente")
            .collapsible(false)
            .default_width(560.0)
            .open(&mut keep_open)
            .show(ctx, |ui| {
                egui::Grid::new("assistant-settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Provedor");
                    egui::ComboBox::from_id_salt("provider")
                        .width(320.0)
                        .selected_text(draft.provider.label())
                        .show_ui(ui, |ui| {
                            for p in Provider::ALL {
                                ui.selectable_value(&mut draft.provider, p, p.label());
                            }
                        });
                    ui.end_row();
                    let provider = draft.provider;
                    let ep = draft.endpoint_mut();
                    ui.label("URL da API");
                    ui.add(egui::TextEdit::singleline(&mut ep.base_url).desired_width(320.0));
                    ui.end_row();
                    ui.label("Modelo");
                    ui.add(egui::TextEdit::singleline(&mut ep.model).desired_width(320.0));
                    ui.end_row();
                    ui.label("Chave da API");
                    let hint = provider.key_env().map_or("opcional".to_string(), |v| format!("vazio = usar {v}"));
                    ui.add(egui::TextEdit::singleline(&mut ep.api_key).password(true).hint_text(hint).desired_width(320.0));
                    ui.end_row();
                    if provider == Provider::Claude {
                        ui.label("Esforço");
                        egui::ComboBox::from_id_salt("effort").selected_text(&draft.claude_effort).show_ui(ui, |ui| {
                            for e in ["low", "medium", "high", "xhigh", "max"] {
                                ui.selectable_value(&mut draft.claude_effort, e.to_string(), e);
                            }
                        });
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut draft.claude_server_fallback, "Fallback automático se o modelo recusar")
                            .on_hover_text("Se um classificador de segurança recusar o pedido, a Anthropic refaz a resposta em outro modelo.");
                        ui.end_row();
                    }
                });
                ui.label(
                    RichText::new(match draft.provider {
                        Provider::Claude => "Crie a chave em console.anthropic.com (cobrança por uso).",
                        Provider::ChatGpt => "Use uma chave da API da OpenAI (platform.openai.com), não a assinatura do ChatGPT Plus.",
                        Provider::Compatible => "Qualquer servidor no formato chat/completions: Ollama (http://localhost:11434/v1), LM Studio, OpenRouter…",
                    })
                    .small()
                    .weak(),
                );
                ui.label(
                    RichText::new("A chave fica salva em texto puro no seu computador; deixe vazia para usar a variável de ambiente.")
                        .small()
                        .weak(),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Prompt de sistema").strong());
                    if ui.small_button("Restaurar padrão").clicked() {
                        draft.system_prompt = assistant::DEFAULT_SYSTEM_PROMPT.to_string();
                    }
                });
                egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut draft.system_prompt).desired_rows(12).desired_width(f32::INFINITY));
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Salvar").clicked() {
                        save = true;
                    }
                    if !self.settings_status.is_empty() {
                        ui.label(RichText::new(&self.settings_status).small());
                    }
                });
                if let Some(path) = Config::path() {
                    ui.label(RichText::new(format!("Arquivo: {}", path.display())).small().weak());
                }
            });

        if save && act {
            match draft.save() {
                Ok(()) => {
                    // trocar de provedor/modelo/prompt começa uma conversa nova
                    self.config = draft.clone();
                    self.items.clear();
                    let _ = self.tx.send(Cmd::Reset(draft.clone()));
                    self.settings_status = "Salvo.".into();
                    keep_open = false;
                }
                Err(e) => self.settings_status = format!("Erro ao salvar: {e:#}"),
            }
        }
        if keep_open {
            self.settings = Some(draft);
        }
    }
}

fn bubble(ui: &mut egui::Ui, text: &str, user: bool) {
    let fill = if user {
        ui.visuals().widgets.inactive.bg_fill
    } else {
        ui.visuals().extreme_bg_color
    };
    egui::Frame::new()
        .fill(fill)
        .corner_radius(8.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if user {
                ui.label(RichText::new(text).strong());
            } else {
                markdown_lite(ui, text);
            }
        });
}

/// Renderização simples do texto do agente: títulos, listas e negrito sem os marcadores de markdown.
fn markdown_lite(ui: &mut egui::Ui, text: &str) {
    for line in text.lines() {
        let t = line.trim_end();
        if t.trim().is_empty() {
            ui.add_space(4.0);
            continue;
        }
        let heading = t.trim_start().starts_with('#');
        let clean = t
            .trim_start_matches('#')
            .trim_start()
            .replace("**", "")
            .replace('`', "");
        let clean = match clean
            .strip_prefix("- ")
            .or_else(|| clean.strip_prefix("* "))
        {
            Some(rest) => format!("•  {rest}"),
            None => clean.clone(),
        };
        if heading {
            ui.label(RichText::new(clean).strong());
        } else {
            ui.label(clean);
        }
    }
}

/// Card da proposta; devolve os blocos resolvidos quando o usuário clica em Aplicar.
fn proposal_card(ui: &mut egui::Ui, p: &Proposal, applied: bool) -> Option<Vec<ResolvedBlock>> {
    let mut apply = None;
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, GOLD))
        .corner_radius(8.0)
        .inner_margin(10.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(
                RichText::new(format!("Proposta: {}", p.title))
                    .strong()
                    .color(GOLD),
            );
            if !p.explanation.is_empty() {
                ui.label(RichText::new(&p.explanation).small());
            }
            ui.add_space(4.0);
            for b in &p.blocks {
                let values: Vec<String> = b
                    .params
                    .iter()
                    .map(|(k, v)| match v {
                        serde_json::Value::String(s) => format!("{k} {s}"),
                        serde_json::Value::Bool(on) => {
                            format!("{k} {}", if *on { "on" } else { "off" })
                        }
                        other => format!("{k} {other}"),
                    })
                    .collect();
                ui.label(
                    RichText::new(format!(
                        "{} {} {}{}",
                        b.module,
                        if b.on { "[on]" } else { "[off]" },
                        b.algorithm,
                        if values.is_empty() {
                            String::new()
                        } else {
                            format!(" — {}", values.join(" · "))
                        }
                    ))
                    .small(),
                );
            }
            ui.add_space(6.0);
            match assistant::resolve(p) {
                Err(errs) => {
                    ui.colored_label(Color32::from_rgb(230, 110, 100), errs.join("; "));
                }
                Ok(blocks) => {
                    ui.horizontal(|ui| {
                        if applied {
                            ui.label(
                                RichText::new("Aplicada no buffer de edição (salve para manter)")
                                    .small()
                                    .color(GOLD),
                            );
                        } else if ui
                            .button(RichText::new("Aplicar no buffer").strong())
                            .clicked()
                        {
                            apply = Some(blocks);
                        }
                    });
                }
            }
        });
    apply
}
