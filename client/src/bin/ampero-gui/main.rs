// Em release no Windows a GUI não abre uma janela de console junto.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Editor gráfico multiplataforma para o Hotone Ampero (egui/eframe).

mod app_menu;
mod mcp_server;
mod self_update;
mod settings;
mod theme;
mod widgets;
mod worker;

use ampero::params::{self, Kind};
use ampero::preset::{MODULES, Preset};
use eframe::egui::{self, Color32, RichText};
use widgets::{accent, themed};
use worker::{Cmd, Evt, Worker};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 860.0])
            .with_maximized(true) // duas colunas (presets e editor) com a cadeia inteira pedem espaço
            .with_title("Ampero")
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))
                    .expect("ícone PNG válido"),
            ),
        ..Default::default()
    };
    eframe::run_native("Ampero", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

struct App {
    worker: Worker,
    port: String,
    connected: bool,
    status: String,
    progress: Option<f32>,
    presets: Vec<Preset>,
    user_irs: Vec<Option<String>>,
    bank: u8,
    search: String,
    current: Option<Preset>,
    selected_block: usize,
    confirm_save: bool,
    /// Configurações globais (função dos footswitches), lidas na sincronização.
    global: Option<ampero::global::GlobalSettings>,
    /// Janela dos alvos do CTRL aberta (pelo indicador do cabeçalho).
    show_ctrl: bool,
    /// Renomeando o preset no cabeçalho: texto em edição, erro de validação e pedido de foco.
    name_edit: Option<(u16, String)>,
    name_edit_error: Option<String>,
    name_edit_focus: bool,
    /// Campos "clique para digitar" do cabeçalho.
    tempo_edit: widgets::NumberEdit,
    volume_edit: widgets::NumberEdit,
    /// O tempo foi mudado pelo app (conta como "não salvo"; o tap tempo da pedaleira não conta).
    tempo_edited: bool,
    /// Último evento recebido da pedaleira e quando chegou.
    activity: Option<(String, std::time::Instant)>,
    /// Blocos cujo liga/desliga foi feito na pedaleira (CTRL, footswitch): uso ao vivo, não conta como "não salvo".
    pedal_switched: [bool; 9],
    /// Fotos e descrições do editor oficial, se instalado.
    resources: Option<ampero::resources::Resources>,
    textures: std::collections::HashMap<std::path::PathBuf, Option<egui::TextureHandle>>,
    /// O egui pode rodar `ui()` duas vezes no mesmo frame (multi-pass) com o mesmo input;
    /// efeitos colaterais só podem acontecer na primeira passada.
    first_pass: bool,
    /// Menu ⚙ (tema, sobre, versão) e aviso de atualização.
    menu: app_menu::AppMenu,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> App {
        theme::apply(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let worker = Worker::spawn(move || ctx.request_repaint());
        let app = App {
            worker,
            port: "Ampero".into(),
            connected: false,
            status: "Conectando…".into(),
            progress: None,
            presets: vec![],
            user_irs: vec![],
            bank: 1,
            search: String::new(),
            current: None,
            selected_block: 2,
            confirm_save: false,
            show_ctrl: false,
            global: None,
            name_edit: None,
            name_edit_error: None,
            name_edit_focus: false,
            tempo_edit: Default::default(),
            volume_edit: Default::default(),
            tempo_edited: false,
            activity: None,
            pedal_switched: [false; 9],
            resources: ampero::resources::Resources::locate(),
            textures: Default::default(),
            first_pass: true,
            menu: app_menu::AppMenu::new(&cc.egui_ctx),
        };
        app.worker.send(Cmd::Connect(app.port.clone()));
        app.worker.send(Cmd::Sync);
        app
    }

    /// Aplica um timbre (vindo do MCP) no buffer de edição: troca algoritmos, liga/desliga blocos e
    /// ajusta parâmetros; no fim relê o buffer para a tela mostrar o estado real. Nada é salvo.
    fn apply_proposal(&mut self, blocks: Vec<ampero::timbre::ResolvedBlock>) {
        let Some(cur) = self.current.as_ref() else {
            self.status = "Conecte a pedaleira para aplicar a proposta".into();
            return;
        };
        let mut cmds = Vec::new();
        for b in &blocks {
            let Some(now) = cur.blocks.get(b.block) else {
                continue;
            };
            if now.algorithm != b.algorithm {
                cmds.push(Cmd::SetAlgorithm {
                    block: b.block,
                    alg: b.algorithm,
                });
            }
            if now.on != b.on {
                cmds.push(Cmd::Switch {
                    block: b.block,
                    on: b.on,
                });
            }
            for &(param, value) in &b.params {
                cmds.push(Cmd::SetParam {
                    block: b.block,
                    alg: b.algorithm,
                    param,
                    value,
                });
            }
        }
        for b in &blocks {
            self.pedal_switched[b.block] = false; // mudanças vindas da GUI contam como edição
        }
        if let Some(first) = blocks.first() {
            self.selected_block = first.block;
        }
        for cmd in cmds {
            self.send(cmd);
        }
        self.send(Cmd::ReadCurrent);
        self.status = format!(
            "Proposta aplicada em {} bloco(s) — salve para manter",
            blocks.len()
        );
    }

    /// Executa uma ferramenta pedida por um cliente MCP (Claude Desktop/Claude Code). Devolve (texto, é erro).
    fn run_mcp_tool(&mut self, name: &str, args: &serde_json::Value) -> (String, bool) {
        use ampero::timbre::{ToolContext, ToolOutcome, run_tool};
        let ctx = ToolContext {
            preset: self.current.as_ref(),
            resources: self.resources.as_ref(),
            unsaved: self.is_unsaved(),
            active_ctrls: self.active_ctrls(),
        };
        match run_tool(name, args, &ctx) {
            ToolOutcome::Text(t) => (t, false),
            ToolOutcome::Error(e) => (e, true),
            ToolOutcome::Apply(_, _, _) if self.current.is_none() => (
                "A pedaleira não está conectada no app; nada foi aplicado.".into(),
                true,
            ),
            ToolOutcome::Apply(proposal, blocks, ctrl) => {
                let n = blocks.len();
                // alvos dos CTRL antes dos blocos: o ReadCurrent no fim de apply_proposal relê tudo
                for &(k, mask) in &ctrl {
                    if let Some(cur) = self.current.as_mut() {
                        cur.ctrl_targets[k] = mask;
                    }
                    self.send(Cmd::SetCtrlTargets { block: k, mask });
                }
                self.apply_proposal(blocks);
                self.status = format!(
                    "Claude aplicou “{}” no buffer — salve para manter",
                    proposal.title
                );
                let ctrl_note = if ctrl.is_empty() {
                    String::new()
                } else {
                    let names: Vec<&str> = proposal.ctrl.keys().map(String::as_str).collect();
                    format!(" CTRL atualizados: {}.", names.join(", "))
                };
                (
                    format!(
                        "Aplicado \"{}\" no buffer de edição ({n} bloco(s)).{ctrl_note} Não foi salvo: o usuário \
                         ouve e salva pela pedaleira ou pelo app se gostar.",
                        proposal.title
                    ),
                    false,
                )
            }
            ToolOutcome::Save { label, name } => {
                // run_tool já conferiu: preset carregado, banco de usuário e rótulo confirmado
                let Some(cur) = self.current.as_mut() else {
                    return (
                        "A pedaleira não está conectada no app; nada foi salvo.".into(),
                        true,
                    );
                };
                cur.name = name.clone();
                let (bank, slot) = (cur.bank, cur.slot);
                self.send(Cmd::Save {
                    bank,
                    slot,
                    name: name.clone(),
                });
                self.status = format!("Claude salvou {label} “{name}”");
                (format!("Salvo em {label} \"{name}\"."), false)
            }
        }
    }

    /// Máscara dos CTRL que fazem efeito (bit n = CTRL n+1): footswitches com Tap = CTRL/TAP no global.
    /// Sem o global (não lido), todos.
    fn active_ctrls(&self) -> u8 {
        self.global.map_or(0x0F, |g| g.active_ctrls())
    }

    /// O buffer de edição carregado difere do preset gravado.
    fn is_unsaved(&self) -> bool {
        self.current
            .as_ref()
            .is_some_and(|cur| self.differs_from_saved(cur))
    }

    /// `cur` difere do preset gravado: som, nome editado no app ou tempo mudado pelo app. Liga/desliga e
    /// tempo vindos da pedaleira (CTRL, footswitch, tap tempo) são uso ao vivo e não contam.
    fn differs_from_saved(&self, cur: &Preset) -> bool {
        self.presets
            .iter()
            .find(|p| p.index == cur.index)
            .is_some_and(|s| {
                !same_sound(s, cur, &self.pedal_switched)
                    || s.name != cur.name
                    || (self.tempo_edited && s.tempo != cur.tempo)
                    || s.ctrl_targets != cur.ctrl_targets
            })
    }

    fn send(&self, cmd: Cmd) {
        if self.first_pass {
            self.worker.send(cmd);
        }
    }

    fn handle_events(&mut self) {
        while let Ok(evt) = self.worker.rx.try_recv() {
            match evt {
                Evt::Connected(port) => {
                    self.connected = true;
                    self.status = format!("Conectado a \"{port}\"");
                }
                Evt::Disconnected(err) => {
                    self.connected = false;
                    self.progress = None;
                    self.status = err;
                }
                Evt::Progress { done, total } => {
                    self.progress = Some(done as f32 / total as f32);
                    self.status = format!("Lendo presets… {done}/{total}");
                }
                Evt::Synced {
                    presets,
                    irs,
                    global,
                    secs,
                } => {
                    self.global = global;
                    self.status = format!("{} presets sincronizados em {secs:.2} s", presets.len());
                    self.presets = presets;
                    self.user_irs = irs;
                    self.progress = None;
                }
                Evt::Current(p) => {
                    if self.current.as_ref().map(|c| c.index) != Some(p.index) {
                        self.pedal_switched = [false; 9];
                        self.tempo_edited = false;
                    }
                    // GUI aberta com o CTRL já acionado: as diferenças de liga/desliga vêm do CTRL.
                    if let Some(saved) = self.presets.iter().find(|s| s.index == p.index)
                        && saved.ctrl_state != p.ctrl_state
                    {
                        for (k, (b, s)) in p.blocks.iter().zip(&saved.blocks).enumerate() {
                            if b.on != s.on {
                                self.pedal_switched[k] = true;
                            }
                        }
                    }
                    self.current = Some(p);
                }
                Evt::HardwareParam(block, param, value) => {
                    if let Some(p) = self.current.as_mut().and_then(|c| c.blocks.get_mut(block))
                        && let Some(v) = p.params.get_mut(param as usize)
                    {
                        *v = value; // "não salvo" sai da comparação com o preset gravado
                    }
                }
                Evt::HardwareSwitch(block, on) => {
                    if let Some(flag) = self.pedal_switched.get_mut(block) {
                        *flag = true;
                    }
                    if let Some(b) = self.current.as_mut().and_then(|c| c.blocks.get_mut(block)) {
                        b.on = on;
                    }
                }
                Evt::Activity(text) => self.activity = Some((text, std::time::Instant::now())),
                Evt::CtrlState(v) => {
                    if let Some(c) = self.current.as_mut() {
                        c.ctrl_state = v;
                    }
                }
                Evt::Tempo(bpm) => {
                    if let Some(c) = self.current.as_mut() {
                        c.tempo = bpm;
                    }
                }
                Evt::Saved(bank, slot) => {
                    let index = bank as u16 * 128 + slot as u16;
                    if let Some(cur) = &self.current
                        && let Some(entry) = self.presets.iter_mut().find(|p| p.index == index)
                    {
                        *entry = cur.clone();
                    }
                    self.pedal_switched = [false; 9];
                    self.tempo_edited = false;
                    self.status =
                        format!("Salvo em banco {bank} {}", ampero::preset::slot_label(slot));
                }
                Evt::Error(e) => self.status = format!("Erro: {e}"),
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        // Barra enxuta: marca, conexão (os controles de manutenção ficam no menu dela), status discreto e,
        // à direita, MCP e ⚙. O que se usa o tempo todo (salvar, volume, tempo) está no cabeçalho do preset.
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("AMPERO")
                    .size(theme::LABEL)
                    .strong()
                    .extra_letter_spacing(1.5),
            );
            ui.add_space(theme::S3);
            let (dot, label) = if self.connected {
                (theme::OK, "Ampero conectado")
            } else {
                (theme::ERROR, "Desconectado")
            };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.5, dot);
            ui.menu_button(label, |ui| {
                ui.set_min_width(240.0);
                ui.horizontal(|ui| {
                    ui.label("Porta MIDI");
                    ui.add(egui::TextEdit::singleline(&mut self.port).desired_width(130.0))
                        .on_hover_text("Trecho do nome da porta MIDI da pedaleira");
                });
                let reconnect = if self.connected {
                    "Reconectar"
                } else {
                    "Conectar"
                };
                if ui.button(reconnect).clicked() {
                    self.send(Cmd::Connect(self.port.clone()));
                    ui.close();
                }
                ui.separator();
                ui.add_enabled_ui(self.connected && self.progress.is_none(), |ui| {
                    if ui
                        .button("Sincronizar presets")
                        .on_hover_text("Relê os nomes e todos os presets da pedaleira")
                        .clicked()
                    {
                        self.send(Cmd::Sync);
                        ui.close();
                    }
                    if ui
                        .button("Reler o preset atual")
                        .on_hover_text("Relê o buffer de edição da pedaleira")
                        .clicked()
                    {
                        self.send(Cmd::ReadCurrent);
                        ui.close();
                    }
                });
            })
            .response
            .on_hover_text("Conexão com a pedaleira");
            if let Some(f) = self.progress {
                ui.add(egui::ProgressBar::new(f).desired_width(160.0));
            }
            ui.add_space(theme::S2);
            ui.label(RichText::new(&self.status).size(theme::META).weak());
            if let Some((text, at)) = &self.activity {
                let age = at.elapsed().as_secs_f32();
                let color = if age < 1.5 {
                    accent(ui)
                } else {
                    ui.visuals().weak_text_color()
                };
                ui.add_space(theme::S2);
                ui.label(
                    RichText::new(format!("Pedaleira: {text}"))
                        .size(theme::META)
                        .color(color),
                );
                if age < 1.5 {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(200));
                }
            }
            let act = self.first_pass;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.menu.menu_button(ui, act);
                self.menu.mcp_indicator(ui, act); // da direita para a esquerda: fica antes do ⚙
            });
        });
    }

    fn preset_list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.bank, 1, "User Patches");
            ui.selectable_value(&mut self.bank, 0, "Factory");
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Buscar…")
                .desired_width(f32::INFINITY),
        );
        ui.separator();
        let needle = self.search.to_lowercase();
        let current = self.current.as_ref().map(|c| c.index);
        let mut clicked = None;
        // A lista ocupa a largura do painel (barra de rolagem na borda dele) e corta nomes que não cabem,
        // em vez de passar por baixo da barra e invadir a área central quando a fonte fica maior.
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                    for p in self.presets.iter().filter(|p| p.bank == self.bank) {
                        if !needle.is_empty()
                            && !p.name.to_lowercase().contains(&needle)
                            && !p.label.to_lowercase().contains(&needle)
                        {
                            continue;
                        }
                        let full = format!("{}  {}", p.label, p.name);
                        let resp = ui.selectable_label(
                            current == Some(p.index),
                            RichText::new(&full).monospace(),
                        );
                        if resp.clicked() {
                            clicked = Some((p.bank, p.slot));
                        }
                        // nome cortado: o completo aparece ao passar o mouse
                        resp.on_hover_text(full);
                    }
                });
            });
        if let Some((bank, slot)) = clicked {
            self.send(Cmd::Select { bank, slot });
        }
    }

    /// Cadeia de sinal numa faixa horizontal (como no editor oficial), com rolagem lateral se não couber.
    /// Cada card: tipo do bloco + ON/OFF em cima, miniatura do efeito no meio, nome embaixo.
    fn chain(&mut self, ui: &mut egui::Ui, cur: &mut Preset) {
        const CARD_W: f32 = 92.0;
        const THUMB_H: f32 = 64.0;
        // A roda do mouse (vertical) rola a faixa na horizontal quando o cursor está sobre ela —
        // o egui só faz isso com Shift. Usa o retângulo/limite medidos no frame anterior e força o
        // offset antes de desenhar. Só na primeira passada para não aplicar o delta duas vezes.
        let memory = ui.id().with("chain-scroll-memory");
        let mut area = egui::ScrollArea::horizontal()
            .id_salt("chain")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded);
        let previous: Option<(egui::Id, egui::Rect, f32)> = ui.data(|d| d.get_temp(memory));
        if let Some((area_id, rect, overflow)) = previous
            && self.first_pass
            && overflow > 0.0
            && ui.rect_contains_pointer(rect)
        {
            let (wheel, shift) = ui.input(|i| (i.smooth_scroll_delta.y, i.modifiers.shift));
            if wheel != 0.0 && !shift {
                let now =
                    egui::scroll_area::State::load(ui.ctx(), area_id).map_or(0.0, |s| s.offset.x);
                area = area.horizontal_scroll_offset((now - wheel).clamp(0.0, overflow));
            }
        }
        // arrastar um card reordena a cadeia, então o arraste não pode rolar a faixa
        let area = area.scroll_source(egui::scroll_area::ScrollSource {
            drag: egui::scroll_area::DragScroll::Never,
            ..Default::default()
        });
        let mut card_rects: Vec<egui::Rect> = Vec::new();
        // Centraliza a faixa quando ela cabe: usa a largura medida no frame anterior e, se ela mudou,
        // pede uma passada extra ao egui (como `widgets::centered_row`).
        let width_id = memory.with("strip-width");
        let height_id = memory.with("card-height");
        let card_h: f32 = ui.data(|d| d.get_temp(height_id)).unwrap_or(20.0);
        let last_width: f32 = ui.data(|d| d.get_temp(width_id)).unwrap_or(0.0);
        let pad = ((ui.available_width() - last_width) / 2.0).max(0.0);
        let out = area.show(ui, |ui| {
            ui.horizontal(|ui| {
                // espaço pequeno: a seta já separa um card do outro
                let card_spacing = ui.spacing().item_spacing.x;
                ui.spacing_mut().item_spacing.x = 3.0;
                ui.add_space(pad);
                let start = ui.cursor().left();
                flow_arrow(ui, card_h); // entrada
                for (pos, name) in cur.chain.clone().into_iter().enumerate() {
                    if pos > 0 {
                        flow_arrow(ui, card_h);
                    }
                    let k = MODULES.iter().position(|m| *m == name).unwrap_or(0);
                    // só os CTRL que fazem efeito (footswitch como CTRL/TAP no global)
                    let ctrl_mask = cur.ctrl_targets[k] & self.active_ctrls();
                    let block = &mut cur.blocks[k];
                    let thumb = match (&self.resources, block.algorithm_name.as_deref()) {
                        (Some(res), Some(n)) => res.image_path(block.module, n),
                        _ => None,
                    }
                    .and_then(|p| self.texture(ui.ctx(), &p));

                    // Card neutro: ligado/desligado se lê pela pílula ON e pela miniatura (cheia ou
                    // esmaecida); a seleção, pelo contorno claro e um fundo um tom acima. O dourado fica só
                    // para "ligado" (ver theme.rs).
                    let selected = self.selected_block == k;
                    let fill = match (selected, ui.visuals().dark_mode) {
                        (true, true) => Color32::from_gray(48),
                        (false, true) => Color32::from_gray(32),
                        (true, false) => Color32::WHITE,
                        (false, false) => Color32::from_gray(238),
                    };
                    let stroke = if selected {
                        egui::Stroke::new(2.0, theme::selected(ui))
                    } else {
                        egui::Stroke::new(
                            1.0,
                            themed(ui, Color32::from_gray(62), Color32::from_gray(200)),
                        )
                    };
                    // o card inteiro é arrastável (a posição na cadeia vai como carga) e clicável: o clique
                    // no ON/OFF liga/desliga, no resto do card seleciona o bloco (ver `drag_card`)
                    let card = drag_card(ui, egui::Id::new(("chain-card", k)), pos, |ui| {
                        ui.spacing_mut().item_spacing.x = card_spacing; // dentro do card, o normal
                        egui::Frame::new()
                            .fill(fill)
                            .stroke(stroke)
                            .corner_radius(8.0)
                            .inner_margin(theme::S2 - 1.0)
                            .show(ui, |ui| {
                                ui.set_width(CARD_W);
                                ui.vertical(|ui| {
                                    // topo: tipo do bloco + ON/OFF (devolve onde o botão ficou)
                                    let toggle_rect = ui
                                        .horizontal(|ui| {
                                            ui.add_sized(
                                                [CARD_W - 42.0, 18.0],
                                                egui::Label::new(
                                                    RichText::new(block.module).strong(),
                                                ),
                                            );
                                            // pílula ON acesa (dourada) ou OFF apagada
                                            let toggle = if block.on {
                                                egui::Button::new(
                                                    RichText::new("ON")
                                                        .size(theme::META)
                                                        .strong()
                                                        .color(Color32::from_rgb(40, 28, 0)),
                                                )
                                                .fill(widgets::GOLD)
                                            } else {
                                                egui::Button::new(
                                                    RichText::new("OFF")
                                                        .size(theme::META)
                                                        .strong()
                                                        .color(ui.visuals().weak_text_color()),
                                                )
                                                .fill(Color32::TRANSPARENT)
                                            };
                                            ui.add_sized([34.0, 18.0], toggle.corner_radius(9.0))
                                                .rect
                                        })
                                        .inner;
                                    // meio: miniatura
                                    let picture = match &thumb {
                                        Some(tex) => {
                                            let size = tex.size_vec2();
                                            let scale = (THUMB_H / size.y).min(CARD_W / size.x);
                                            let img = egui::Image::new((tex.id(), size * scale));
                                            let img = if block.on {
                                                img
                                            } else {
                                                img.tint(themed(
                                                    ui,
                                                    Color32::from_gray(85),
                                                    Color32::from_gray(150),
                                                ))
                                            };
                                            egui::Button::image(img).frame(false)
                                        }
                                        None => egui::Button::new(
                                            RichText::new(block.module).size(20.0).weak(),
                                        )
                                        .frame(false),
                                    };
                                    let thumb_rect = ui.add_sized([CARD_W, THUMB_H], picture).rect;
                                    // selo dos CTRL que ligam/desligam este bloco (só visual; edita-se
                                    // na janela do CTRL, pelo indicador do cabeçalho)
                                    ctrl_badge(ui, thumb_rect, ctrl_mask);
                                    // embaixo: nome do efeito
                                    let label = block.algorithm_name.as_deref().unwrap_or("?");
                                    let name = RichText::new(label).size(theme::META);
                                    let name = if block.on { name } else { name.weak() };
                                    let select = egui::Button::new(name).frame(false).truncate();
                                    ui.add_sized([CARD_W, 16.0], select);
                                    toggle_rect
                                })
                                .inner
                            })
                            .inner
                    });
                    if card.response.clicked() && self.first_pass {
                        let on_toggle = card
                            .response
                            .interact_pointer_pos()
                            .is_some_and(|p| card.inner.contains(p));
                        if on_toggle {
                            block.on = !block.on;
                            self.pedal_switched[k] = false; // alternado pela GUI: é edição
                            self.send(Cmd::Switch {
                                block: k,
                                on: block.on,
                            });
                        } else {
                            self.selected_block = k;
                        }
                    }
                    card_rects.push(card.response.rect);
                }
                flow_arrow(ui, card_h); // saída
                if let Some(h) = card_rects.first().map(|r| r.height())
                    && (h - card_h).abs() > 0.5
                {
                    ui.data_mut(|d| d.insert_temp(height_id, h));
                    ui.ctx().request_discard("altura das setas da cadeia");
                }
                let width = ui.cursor().left() - start;
                if (width - last_width).abs() > 0.5 {
                    ui.data_mut(|d| d.insert_temp(width_id, width));
                    ui.ctx().request_discard("centralizar cadeia");
                }
                self.chain_drop(ui, cur, &card_rects);
            });
        });

        let overflow = out.content_size.x - out.inner_rect.width();
        // só a faixa dos cards (o inner_rect da ScrollArea pode ir até o fim do painel, e aí a roda
        // sobre os knobs também rolaria os pedais)
        let strip = egui::Rect::from_min_size(
            out.inner_rect.min,
            egui::vec2(
                out.inner_rect.width(),
                out.content_size.y.min(out.inner_rect.height()),
            ),
        );
        ui.data_mut(|d| d.insert_temp(memory, (out.id, strip, overflow)));
    }

    /// Enquanto um card da cadeia é arrastado, marca onde ele vai entrar; ao soltar, reordena a cadeia e
    /// manda a ordem nova para o buffer de edição (como o "Manage Signal Chain" do editor oficial).
    fn chain_drop(&mut self, ui: &mut egui::Ui, cur: &mut Preset, rects: &[egui::Rect]) {
        let ctx = ui.ctx().clone();
        if rects.is_empty() || !egui::DragAndDrop::has_payload_of_type::<usize>(&ctx) {
            return;
        }
        let Some(pointer) = ctx.pointer_interact_pos() else {
            return;
        };
        // posição de inserção: quantos cards têm o centro à esquerda do ponteiro
        let insert = rects.iter().filter(|r| r.center().x < pointer.x).count();
        // a linha fica no meio do vão entre os cards (onde está a seta), ou logo fora das pontas
        let x = match insert {
            0 => rects[0].left() - 4.0,
            i if i == rects.len() => rects[i - 1].right() + 4.0,
            i => (rects[i - 1].right() + rects[i].left()) / 2.0,
        };
        let (top, bottom) = (rects[0].top(), rects[0].bottom());
        ui.painter().line_segment(
            [egui::pos2(x, top), egui::pos2(x, bottom)],
            egui::Stroke::new(3.0, widgets::GOLD),
        );
        if !ui.input(|i| i.pointer.primary_released()) {
            return;
        }
        let Some(from) = egui::DragAndDrop::take_payload::<usize>(&ctx).map(|p| *p) else {
            return;
        };
        let to = if insert > from { insert - 1 } else { insert };
        if from == to || from >= cur.chain.len() {
            return;
        }
        let module = cur.chain.remove(from);
        cur.chain.insert(to, module);
        let order: Vec<usize> = cur
            .chain
            .iter()
            .filter_map(|m| MODULES.iter().position(|x| x == m))
            .collect();
        self.send(Cmd::SetChain(order));
        self.status = format!("{module} movido para a posição {} da cadeia", to + 1);
    }

    fn block_editor(&mut self, ui: &mut egui::Ui, cur: &mut Preset) {
        let k = self.selected_block;
        let module = MODULES[k];
        let block = &mut cur.blocks[k];

        // cabeçalho do bloco: tipo + algoritmo, com ‹ › para passar pelos algoritmos sem abrir a lista
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(module)
                    .size(theme::SECTION)
                    .strong()
                    .color(ui.visuals().weak_text_color()),
            );
            ui.add_space(theme::S1);
            let algorithms = params::algorithms_of(module);
            let pos = algorithms.iter().position(|(c, _)| *c == block.algorithm);
            let mut alg = block.algorithm;
            let step = |delta: isize| {
                let n = algorithms.len() as isize;
                pos.map(|p| algorithms[((p as isize + delta).rem_euclid(n)) as usize].0)
            };
            if ui
                .button(RichText::new("‹").size(theme::SECTION))
                .on_hover_text("Algoritmo anterior")
                .clicked()
                && let Some(a) = step(-1)
            {
                alg = a;
            }
            egui::ComboBox::from_id_salt("algorithm")
                .width(240.0)
                .height(420.0)
                .selected_text(
                    RichText::new(
                        block
                            .algorithm_name
                            .clone()
                            .unwrap_or_else(|| format!("#{alg}")),
                    )
                    .size(theme::LABEL)
                    .strong(),
                )
                .show_ui(ui, |ui| {
                    for (code, name) in &algorithms {
                        ui.selectable_value(&mut alg, *code, *name);
                    }
                });
            if ui
                .button(RichText::new("›").size(theme::SECTION))
                .on_hover_text("Próximo algoritmo")
                .clicked()
                && let Some(a) = step(1)
            {
                alg = a;
            }
            if let Some(p) = pos {
                ui.label(
                    RichText::new(format!("{} de {}", p + 1, algorithms.len()))
                        .size(theme::META)
                        .weak(),
                );
            }
            if alg != block.algorithm {
                self.send(Cmd::SetAlgorithm { block: k, alg });
            }
        });
        ui.add_space(theme::S2);

        // valores salvos do mesmo bloco (para marcar o que mudou), se o algoritmo for o mesmo
        let saved: Option<Vec<u16>> = self
            .presets
            .iter()
            .find(|p| p.index == cur.index)
            .map(|p| &p.blocks[k])
            .filter(|b| b.algorithm == block.algorithm)
            .map(|b| b.params.clone());
        let name = block.algorithm_name.clone();
        ui.horizontal_top(|ui| {
            let left_h = self.effect_info(ui, module, name.as_deref());
            ui.add_space(theme::S2);
            ui.separator();
            ui.add_space(theme::S4);
            ui.vertical(|ui| self.param_grid(ui, k, module, block, saved.as_deref(), left_h));
        });
    }

    /// Foto + nome + descrição do efeito, como no editor oficial (lidos da instalação dele).
    fn effect_info(&mut self, ui: &mut egui::Ui, module: &str, name: Option<&str>) -> f32 {
        const WIDTH: f32 = 290.0;
        const IMAGE_MAX_H: f32 = 200.0;
        let (image, description) = match (&self.resources, name) {
            (Some(res), Some(n)) => (
                res.image_path(module, n),
                res.description(module, n).map(str::to_owned),
            ),
            _ => (None, None),
        };
        let texture = image.and_then(|p| self.texture(ui.ctx(), &p));
        ui.vertical(|ui| {
            ui.set_width(WIDTH);
            if let Some(tex) = texture {
                let size = tex.size_vec2();
                let scale = (IMAGE_MAX_H / size.y).min(WIDTH / size.x).min(1.5);
                ui.vertical_centered(|ui| ui.image((tex.id(), size * scale)));
            }
            if let Some(n) = name {
                ui.add_space(theme::S1);
                ui.label(RichText::new(n).strong().size(theme::LABEL));
            }
            match (&self.resources, description) {
                (_, Some(text)) => {
                    // só a primeira frase à mostra; o texto todo (com os controles) fica recolhível
                    let first = first_sentence(&text);
                    ui.label(RichText::new(first).weak());
                    ui.add_space(theme::S1);
                    egui::CollapsingHeader::new(RichText::new("Sobre o efeito").size(theme::META))
                        .id_salt("effect-about")
                        .default_open(false)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("description")
                                .max_height(300.0)
                                .show(ui, |ui| description_view(ui, &text));
                        });
                }
                (Some(_), None) => {
                    ui.label(
                        RichText::new("Sem descrição para este efeito.")
                            .small()
                            .weak(),
                    );
                }
                (None, None) => {
                    ui.label(
                        RichText::new(
                            "Fotos e descrições vêm da instalação do Ampero Editor oficial. \
                             Instale-o ou defina AMPERO_RESOURCES com a pasta Resource/Ampero.",
                        )
                        .small()
                        .weak(),
                    );
                }
            }
        })
        .response
        .rect
        .height()
    }

    /// Carrega (uma vez) o PNG como textura do egui.
    fn texture(
        &mut self,
        ctx: &egui::Context,
        path: &std::path::Path,
    ) -> Option<egui::TextureHandle> {
        self.textures
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                let img = image::open(path).ok()?.to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                Some(ctx.load_texture(path.to_string_lossy(), color, egui::TextureOptions::LINEAR))
            })
            .clone()
    }

    /// Painel híbrido numa "placa" centralizada: knobs para contínuos (maiores quando são poucos),
    /// faders para bandas de EQ, botões segmentados para opções curtas e on/off (dropdown só para
    /// listas longas), campos numéricos para posições. `min_h` = altura da coluna da foto/descrição.
    fn param_grid(
        &mut self,
        ui: &mut egui::Ui,
        k: usize,
        module: &str,
        block: &mut ampero::preset::Block,
        saved: Option<&[u16]>,
        min_h: f32,
    ) {
        let Some(meta) = params::algorithm(module, block.algorithm) else {
            ui.label("Sem metadados para este algoritmo (IR do usuário?). Valores brutos:");
            ui.label(format!("{:?}", block.params));
            return;
        };
        let alg = block.algorithm;
        let act = self.first_pass;
        let saved_of = |i: usize| saved.and_then(|s| s.get(i).copied());
        let mut changes: Vec<(usize, u16)> = Vec::new();
        let params_of = |kinds: &[Kind]| {
            meta.params
                .iter()
                .filter(|p| kinds.contains(&p.kind) && p.index < block.params.len())
                .collect::<Vec<_>>()
        };
        let knobs = params_of(&[Kind::Knob]);
        let faders = params_of(&[Kind::Slider]);
        let options = params_of(&[Kind::Combox, Kind::Switch, Kind::Texteditor]);

        const MARGIN: f32 = 16.0;
        let plate_w = ui.available_width();
        let inner_h = (min_h - 2.0 * MARGIN).max(220.0);
        egui::Frame::new()
            .fill(ui.visuals().extreme_bg_color)
            .stroke(egui::Stroke::new(
                0.5,
                ui.visuals().widgets.inactive.bg_stroke.color,
            ))
            .corner_radius(10.0)
            .inner_margin(MARGIN)
            .show(ui, |ui| {
                ui.set_width(plate_w - 2.0 * MARGIN);
                ui.set_min_height(inner_h);
                // centralização vertical: altura do conteúdo medida no frame anterior
                let h_id = ui.id().with("plate-content-h");
                let last_h: f32 = ui.data(|d| d.get_temp(h_id)).unwrap_or(0.0);
                ui.add_space(((inner_h - last_h) / 2.0).max(0.0));

                let content = ui.scope(|ui| {
                    if !knobs.is_empty() {
                        let scale = widgets::knob_scale(knobs.len());
                        let spacing = ui.spacing().item_spacing.x;
                        let knob_w = 74.0 * scale + spacing;
                        let per_row =
                            ((ui.available_width() + spacing) / knob_w).floor().max(1.0) as usize;
                        for (row, chunk) in knobs.chunks(per_row).enumerate() {
                            widgets::centered_row(ui, ui.id().with(("knob-row", row)), |ui| {
                                for pm in chunk {
                                    let value = &mut block.params[pm.index];
                                    if widgets::knob(ui, value, pm, saved_of(pm.index), act, scale)
                                        .changed()
                                    {
                                        changes.push((pm.index, *value));
                                    }
                                }
                            });
                        }
                    }

                    if !faders.is_empty() {
                        ui.add_space(6.0);
                        let mut values: Vec<u16> =
                            faders.iter().map(|pm| block.params[pm.index]).collect();
                        let saved: Vec<Option<u16>> =
                            faders.iter().map(|pm| saved_of(pm.index)).collect();
                        let moved = widgets::centered_row(ui, ui.id().with("faders"), |ui| {
                            widgets::fader_bank(ui, &faders, &mut values, &saved, act)
                        });
                        for i in moved {
                            block.params[faders[i].index] = values[i];
                            changes.push((faders[i].index, values[i]));
                        }
                    }

                    if !options.is_empty() {
                        ui.add_space(14.0);
                        widgets::centered_row(ui, ui.id().with("options"), |ui| {
                            egui::Grid::new("options")
                                .num_columns(2)
                                .spacing([14.0, 8.0])
                                .show(ui, |ui| {
                                    for pm in &options {
                                        let value = &mut block.params[pm.index];
                                        let marker =
                                            if saved_of(pm.index).is_some_and(|s| s != *value) {
                                                " •"
                                            } else {
                                                ""
                                            };
                                        ui.label(
                                            RichText::new(format!("{}{marker}", pm.name))
                                                .color(ui.visuals().weak_text_color()),
                                        );
                                        let before = *value;
                                        match pm.kind {
                                            Kind::Switch => {
                                                widgets::segmented(ui, value, &["Off", "On"], act);
                                            }
                                            Kind::Combox if pm.options.len() <= 6 => {
                                                let opts: Vec<&str> =
                                                    pm.options.iter().map(String::as_str).collect();
                                                widgets::segmented(ui, value, &opts, act);
                                            }
                                            Kind::Combox => {
                                                let text = pm
                                                    .options
                                                    .get(*value as usize)
                                                    .cloned()
                                                    .unwrap_or_else(|| value.to_string());
                                                egui::ComboBox::from_id_salt(("combo", pm.index))
                                                    .selected_text(text)
                                                    .show_ui(ui, |ui| {
                                                        for (i, opt) in
                                                            pm.options.iter().enumerate()
                                                        {
                                                            ui.selectable_value(
                                                                value, i as u16, opt,
                                                            );
                                                        }
                                                    });
                                            }
                                            _ => {
                                                ui.add(
                                                    egui::DragValue::new(value).range(pm.range()),
                                                );
                                            }
                                        }
                                        if *value != before {
                                            changes.push((pm.index, *value));
                                        }
                                        ui.end_row();
                                    }
                                });
                        });
                    }
                });

                let h = content.response.rect.height();
                if (h - last_h).abs() > 0.5 {
                    ui.data_mut(|d| d.insert_temp(h_id, h));
                    ui.ctx().request_discard("centralizar placa");
                }
            });

        for (param, value) in changes {
            self.send(Cmd::SetParam {
                block: k,
                alg,
                param: param as u8,
                value,
            });
        }
    }

    /// Nome do preset no cabeçalho: clicar abre um campo para renomear (Enter confirma, Esc cancela). O nome
    /// novo deixa o preset "não salvo" e vai para a pedaleira junto com o Salvar, como no editor oficial.
    fn name_editor(&mut self, ui: &mut egui::Ui, cur: &mut Preset) {
        // a edição pertence ao preset em que começou: se ele trocou, descarta
        if self
            .name_edit
            .as_ref()
            .is_some_and(|(index, _)| *index != cur.index)
        {
            self.name_edit = None;
            self.name_edit_error = None;
        }
        let Some((_, buf)) = self.name_edit.as_mut() else {
            let text = RichText::new(&cur.name).size(theme::TITLE).strong();
            if cur.bank == 0 {
                ui.label(text)
                    .on_hover_text("Presets de fábrica não podem ser renomeados");
                return;
            }
            let resp = ui
                .add(egui::Label::new(text).sense(egui::Sense::click()))
                .on_hover_cursor(egui::CursorIcon::Text)
                .on_hover_text("Clique para renomear");
            if resp.clicked() {
                self.name_edit = Some((cur.index, cur.name.clone()));
                self.name_edit_error = None;
                self.name_edit_focus = true;
            }
            return;
        };
        let resp = ui.add(
            egui::TextEdit::singleline(buf)
                .char_limit(16)
                .font(egui::FontId::proportional(theme::TITLE - 2.0))
                .desired_width(230.0)
                .hint_text("nome do preset"),
        );
        if std::mem::take(&mut self.name_edit_focus) {
            resp.request_focus();
        }
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if escape {
            self.name_edit = None;
        } else if resp.lost_focus() {
            match ampero::timbre::validate_preset_name(buf) {
                Ok(name) => {
                    cur.name = name;
                    self.name_edit = None;
                }
                Err(e) => {
                    self.name_edit_error = Some(e);
                    self.name_edit_focus = true;
                }
            }
        }
        if let Some(e) = &self.name_edit_error {
            ui.colored_label(Color32::from_rgb(230, 110, 100), e);
        } else if self.name_edit.is_some() {
            ui.label(RichText::new("Enter confirma, Esc cancela").small().weak());
        }
    }

    /// Tempo do preset: clicar abre um campo (40–250 BPM; Enter confirma, Esc cancela) e o valor vai na hora
    /// para o buffer de edição.
    fn tempo_editor(&mut self, ui: &mut egui::Ui, cur: &mut Preset) {
        use ampero::protocol::{TEMPO_MAX, TEMPO_MIN};
        let hover = format!("Clique para mudar o tempo ({TEMPO_MIN}–{TEMPO_MAX} BPM)");
        let field = widgets::NumberField {
            range: TEMPO_MIN..=TEMPO_MAX,
            suffix: " BPM",
            hover: &hover,
            width: 64.0,
        };
        if let Some(bpm) =
            widgets::number_edit(ui, &mut self.tempo_edit, cur.index, cur.tempo, &field)
        {
            cur.tempo = bpm;
            self.tempo_edited = true;
            self.send(Cmd::SetTempo(bpm));
        }
    }

    /// Janela "CTRL do preset": para cada CTRL 1–4, a função (só leitura) e os blocos que ele liga e desliga
    /// (marcar/desmarcar vai na hora para o buffer de edição e deixa o preset "não salvo").
    fn ctrl_window(&mut self, ctx: &egui::Context, cur: &mut Preset) {
        if !self.show_ctrl {
            return;
        }
        let mut open = true;
        let mut changes: Vec<(usize, u8)> = Vec::new();
        let active = self.active_ctrls();
        let global = self.global;
        egui::Window::new(format!("CTRL do {} \"{}\"", cur.label, cur.name))
            .id(egui::Id::new("ctrl-window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("Marque os blocos que cada CTRL liga e desliga neste preset.")
                        .weak(),
                );
                if global.is_none() {
                    ui.label(
                        RichText::new(
                            "Não deu para ler as configurações globais da pedaleira; mostrando todos os CTRL.",
                        )
                        .small()
                        .weak(),
                    );
                }
                // em ordem numérica; os CTRL sem efeito (footswitch que não está como CTRL/TAP no global)
                // vêm recolhidos e esmaecidos, com os alvos que o preset guarda
                for ctrl in 1..=4u8 {
                    let bit = 1u8 << (ctrl - 1);
                    if active & bit != 0 {
                        Self::ctrl_section(ui, cur, ctrl, &mut changes);
                        continue;
                    }
                    let fs = global.map_or("?", |g| g.fs_tap_name(ctrl as usize));
                    ui.add_space(4.0);
                    egui::CollapsingHeader::new(
                        RichText::new(format!("CTRL {ctrl} · sem efeito")).weak(),
                    )
                    .id_salt(("ctrl-inactive", ctrl))
                    .default_open(false)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!(
                                "O footswitch {ctrl} está como \"{fs}\" nas configurações globais, então o CTRL \
                                 {ctrl} não liga nem desliga nada. Os blocos marcados ficam guardados no preset."
                            ))
                            .small()
                            .weak(),
                        );
                        Self::ctrl_section(ui, cur, ctrl, &mut changes);
                    });
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "As mudanças vão na hora para a pedaleira; salve o preset para manter.",
                    )
                    .small()
                    .weak(),
                );
            });
        for (k, mask) in changes {
            cur.ctrl_targets[k] = mask;
            self.send(Cmd::SetCtrlTargets { block: k, mask });
        }
        if !open {
            self.show_ctrl = false;
        }
    }

    /// Uma seção da janela do CTRL: nome, função (leitura) e as caixas dos 9 blocos. Mudanças vão para `changes`.
    fn ctrl_section(ui: &mut egui::Ui, cur: &Preset, ctrl: u8, changes: &mut Vec<(usize, u8)>) {
        {
            let bit = 1u8 << (ctrl - 1);
            let function = cur.ctrl_functions[ctrl as usize - 1];
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("CTRL {ctrl}"))
                        .strong()
                        .color(CTRL_BLUE),
                );
                let state = if cur.ctrl_state & bit != 0 {
                    " · acionado"
                } else {
                    ""
                };
                ui.label(
                    RichText::new(format!(
                        "função: {}{state}",
                        ampero::preset::ctrl_function_name(function)
                    ))
                    .small()
                    .weak(),
                )
                .on_hover_text("A função se muda no editor oficial (CTRL/EXP)");
            });
            if function != 0 {
                ui.label(
                    RichText::new("Com essa função, os blocos marcados não são alternados.")
                        .small()
                        .weak(),
                );
            }
            // mesma disposição do editor oficial: ordem de armazenamento, 5 + 4 por linha
            egui::Grid::new(("ctrl-grid", ctrl))
                .num_columns(5)
                .show(ui, |ui| {
                    for (k, module) in MODULES.iter().enumerate() {
                        let mut on = cur.ctrl_targets[k] & bit != 0;
                        if ui.checkbox(&mut on, *module).changed() {
                            changes.push((k, cur.ctrl_targets[k] ^ bit));
                        }
                        if k == 4 {
                            ui.end_row();
                        }
                    }
                    ui.end_row();
                });
        }
    }

    fn header(&mut self, ui: &mut egui::Ui, cur: &mut Preset) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(&cur.label)
                    .size(theme::TITLE)
                    .strong()
                    .color(ui.visuals().weak_text_color()),
            );
            self.name_editor(ui, cur);
            let dirty = self.differs_from_saved(cur);
            if dirty {
                ui.label(RichText::new("não salvo").color(theme::warn(ui)))
                    .on_hover_text("O buffer da pedaleira difere do preset gravado");
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_save = self.connected && cur.bank == 1;
                // botão principal (dourado) quando há o que salvar; discreto quando não há
                let save = if dirty {
                    egui::Button::new(
                        RichText::new("Salvar")
                            .strong()
                            .color(Color32::from_rgb(40, 28, 0)),
                    )
                    .fill(widgets::GOLD)
                } else {
                    egui::Button::new("Salvar")
                };
                if ui
                    .add_enabled(can_save, save.min_size(egui::vec2(72.0, 26.0)))
                    .on_disabled_hover_text("Só presets do banco de usuário podem ser salvos")
                    .clicked()
                {
                    self.confirm_save = true;
                }
                ui.add_space(theme::S2);
                ui.label(
                    RichText::new(if cur.volume_pre { "vol. pré" } else { "vol. pós" })
                        .size(theme::META)
                        .weak(),
                )
                .on_hover_text(if cur.volume_pre {
                    "O volume do patch fica antes dos efeitos"
                } else {
                    "O volume do patch fica depois dos efeitos"
                });
                ui.separator();
                // da direita para a esquerda: o campo de tempo entra antes do rótulo "Tempo"
                self.tempo_editor(ui, cur);
                ui.label(RichText::new("Tempo").weak());
                ui.separator();
                // slider sem o número embutido: dentro do layout da direita para a esquerda o egui punha o
                // campo numérico por cima da trilha. O número vai à direita, como texto clicável só de
                // digitar (um DragValue também mudava o valor ao arrastar sobre ele).
                let max = ampero::protocol::VOLUME_MAX;
                let mut volume = cur.volume.min(max);
                let field = widgets::NumberField {
                    range: 0..=max,
                    suffix: "",
                    hover: "Clique para digitar o volume (0–99)",
                    width: 24.0,
                };
                let typed =
                    widgets::number_edit(ui, &mut self.volume_edit, cur.index, volume, &field);
                if let Some(v) = typed {
                    volume = v;
                }
                // distância maior que a margem de toque do egui (interact_radius, 5 px): sem ela, o controle
                // ao lado (desenhado por cima) capturava o clique no número
                ui.add_space(ui.style().interaction.interact_radius * 2.0);
                // teste: knob compacto no lugar do slider (o slider deslocava sob o mouse e pulava)
                let act = self.first_pass;
                let knob = widgets::mini_knob(ui, &mut volume, 0..=max, 22.0, act).on_hover_text(
                    "Volume do patch (0–99): arraste para cima/direita ou role a roda; com Shift, ajuste fino",
                );
                if (typed.is_some() || knob.changed()) && volume != cur.volume {
                    cur.volume = volume;
                    self.send(Cmd::SetVolume(volume));
                }
                ui.label(RichText::new("Volume").weak());
                ui.separator();
                // layout da direita para a esquerda: isto aparece à esquerda do volume/tempo
                // qual CTRL está acionado (0x1AB, bit n = CTRL n+1); clicar abre a janela dos alvos
                let (text, color) = match ctrl_list(cur.ctrl_state & 0x0F) {
                    Some(on) => (format!("{on} ON"), theme::CTRL),
                    None => ("CTRL off".to_string(), ui.visuals().weak_text_color()),
                };
                let resp = ui
                    .add(
                        egui::Label::new(RichText::new(text).strong().color(color))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Clique para escolher os blocos que cada CTRL liga e desliga");
                if resp.clicked() {
                    self.show_ctrl = true;
                }
            });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.first_pass = ui.ctx().current_pass_index() == 0;
        self.handle_events();
        let ctx = ui.ctx().clone();

        self.menu.poll(&ctx);
        // `send` só despacha comandos na primeira passada: as chamadas MCP são atendidas nela
        if self.first_pass {
            for call in self.menu.mcp_calls(&ctx) {
                let reply = self.run_mcp_tool(&call.name, &call.args);
                let _ = call.reply.send(reply);
            }
        }
        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        if self.menu.wants_banner() {
            let act = self.first_pass;
            egui::Panel::top("update-banner").show(ui, |ui| self.menu.banner(ui, act));
        }
        self.menu.about_window(&ctx);
        self.menu.mcp_window(&ctx, self.first_pass);
        egui::Panel::left("presets")
            .default_size(260.0)
            .min_size(180.0)
            .show(ui, |ui| self.preset_list(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            let Some(mut cur) = self.current.take() else {
                ui.centered_and_justified(|ui| {
                    ui.label("Conecte o Ampero e sincronize para começar.")
                });
                return;
            };
            self.header(ui, &mut cur);
            ui.separator();
            self.chain(ui, &mut cur);
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| self.block_editor(ui, &mut cur));
            self.ctrl_window(&ctx, &mut cur);

            if self.confirm_save {
                egui::Window::new("Salvar preset")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .show(&ctx, |ui| {
                        ui.label(format!(
                            "Gravar o estado atual em {} \"{}\" (banco {})?",
                            cur.label, cur.name, cur.bank
                        ));
                        ui.horizontal(|ui| {
                            if ui.button("Salvar").clicked() {
                                self.send(Cmd::Save {
                                    bank: cur.bank,
                                    slot: cur.slot,
                                    name: cur.name.clone(),
                                });
                                self.confirm_save = false;
                            }
                            if ui.button("Cancelar").clicked() {
                                self.confirm_save = false;
                            }
                        });
                    });
            }
            self.current = Some(cur);
        });
    }
}

/// Azul dos CTRL (selo dos cards, janela e indicador): distinto do dourado do ON e legível nos dois temas.
const CTRL_BLUE: Color32 = theme::CTRL;

/// Texto dos CTRL de uma máscara (bit n = CTRL n+1): "CTRL 4", "CTRL 3·4".
fn ctrl_list(mask: u8) -> Option<String> {
    let ctrls: Vec<String> = (1..=4u8)
        .filter(|c| mask & (1 << (c - 1)) != 0)
        .map(|c| c.to_string())
        .collect();
    (!ctrls.is_empty()).then(|| format!("CTRL {}", ctrls.join("·")))
}

/// Selo no canto da miniatura do card com os CTRL que ligam/desligam o bloco.
fn ctrl_badge(ui: &egui::Ui, thumb: egui::Rect, mask: u8) {
    let Some(text) = ctrl_list(mask) else { return };
    let font = egui::FontId::proportional(10.0);
    let galley = ui.painter().layout_no_wrap(text, font, Color32::WHITE);
    let size = galley.size() + egui::vec2(8.0, 2.0);
    let rect = egui::Rect::from_min_size(thumb.right_top() - egui::vec2(size.x, 0.0), size);
    ui.painter().rect_filled(rect, 4.0, CTRL_BLUE);
    ui.painter()
        .galley(rect.min + egui::vec2(4.0, 1.0), galley, Color32::WHITE);
}

/// Card arrastável **e clicável**. Igual ao `Ui::dnd_drag_source` do egui, só que com `Sense::click_and_drag`:
/// o do egui sente só arraste, e aí o egui começa o arraste já no clique (o card "decola" e nenhum clique
/// chega, nem nos botões de dentro). Aqui o arraste só começa depois de o mouse andar alguns pixels, e um
/// clique parado volta como `clicked()` na resposta — quem chama decide o que ele faz pela posição.
fn drag_card<R>(
    ui: &mut egui::Ui,
    id: egui::Id,
    payload: usize,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    if ui.ctx().is_being_dragged(id) {
        egui::DragAndDrop::set_payload(ui.ctx(), payload);
        // desenha o card numa camada própria e o leva até o ponteiro
        let layer = egui::LayerId::new(egui::Order::Tooltip, id);
        let out = ui.scope_builder(egui::UiBuilder::new().layer_id(layer), add_contents);
        if let Some(pointer) = ui.ctx().pointer_interact_pos() {
            let delta = pointer - out.response.rect.center();
            ui.ctx()
                .transform_layer_shapes(layer, egui::emath::TSTransform::from_translation(delta));
        }
        out
    } else {
        let out = ui.scope(add_contents);
        let response = ui
            .interact(out.response.rect, id, egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        egui::InnerResponse::new(out.inner, response)
    }
}

/// Seta do fluxo do sinal entre os cards da cadeia (e nas pontas: entrada e saída), como no editor oficial.
/// `height` = altura dos cards (a do frame anterior), para a seta ficar no meio deles mesmo antes do
/// primeiro card da linha.
fn flow_arrow(ui: &mut egui::Ui, height: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, height), egui::Sense::hover());
    let c = rect.center();
    let color = ui.visuals().weak_text_color();
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(c.x - 3.5, c.y - 5.5),
            egui::pos2(c.x + 4.0, c.y),
            egui::pos2(c.x - 3.5, c.y + 5.5),
        ],
        color,
        egui::Stroke::NONE,
    ));
}

/// Compara o que afeta o som (blocos, cadeia, volume), ignorando metadados e o liga/desliga
/// dos blocos em `ignore_on` (alternados pela pedaleira).
fn same_sound(a: &Preset, b: &Preset, ignore_on: &[bool; 9]) -> bool {
    a.volume == b.volume
        && a.chain == b.chain
        && a.blocks
            .iter()
            .zip(&b.blocks)
            .enumerate()
            .all(|(k, (x, y))| {
                (ignore_on[k] || x.on == y.on) && x.algorithm == y.algorithm && x.params == y.params
            })
}

/// Primeira frase da descrição (até o primeiro ponto ou quebra de linha), para o resumo à mostra.
fn first_sentence(text: &str) -> &str {
    let line = text.trim_start().lines().next().unwrap_or("");
    match line.find(". ") {
        Some(i) => &line[..=i],
        None => line,
    }
}

/// Descrição do efeito: parágrafos normais; na seção "Controls:" o nome de cada controle vem em destaque,
/// com um pequeno espaço entre os itens; a nota de marcas registradas (`*…`) fica menor e esmaecida.
fn description_view(ui: &mut egui::Ui, text: &str) {
    const SIZE: f32 = 13.5;
    let gold = accent(ui);
    let body = ui.visuals().text_color();
    let mut in_controls = false;
    let mut pending_gap = false;
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            pending_gap = true;
            continue;
        }
        if pending_gap {
            ui.add_space(6.0);
            pending_gap = false;
        }
        if line.to_lowercase().starts_with("controls") && line.ends_with(':') {
            in_controls = true;
            ui.label(RichText::new(line).size(SIZE).strong());
            ui.add_space(2.0);
        } else if line.starts_with('*') {
            in_controls = false;
            ui.label(RichText::new(line).size(SIZE - 2.5).weak());
        } else if let Some((name, desc)) = line
            .split_once(':')
            .filter(|(n, _)| in_controls && n.len() <= 40)
        {
            let mut job = egui::text::LayoutJob::default();
            let fmt = |color| egui::TextFormat {
                font_id: egui::FontId::proportional(SIZE),
                color,
                ..Default::default()
            };
            job.append(name.trim(), 0.0, fmt(gold));
            job.append(&format!(": {}", desc.trim()), 0.0, fmt(body));
            ui.label(job);
            ui.add_space(3.0);
        } else {
            ui.label(RichText::new(line).size(SIZE));
        }
    }
}
