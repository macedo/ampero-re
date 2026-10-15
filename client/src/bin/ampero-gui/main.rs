//! Editor gráfico multiplataforma para o Hotone Ampero (egui/eframe).

mod worker;

use ampero::params::{self, Kind};
use ampero::preset::{MODULES, Preset};
use eframe::egui::{self, Color32, RichText};
use worker::{Cmd, Evt, Worker};

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 720.0])
            .with_title("Ampero"),
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
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> App {
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
            activity: None,
            pedal_switched: [false; 9],
            resources: ampero::resources::Resources::locate(),
            textures: Default::default(),
            first_pass: true,
        };
        app.worker.send(Cmd::Connect(app.port.clone()));
        app.worker.send(Cmd::Sync);
        app
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
                Evt::Synced { presets, irs, secs } => {
                    self.status = format!("{} presets sincronizados em {secs:.2} s", presets.len());
                    self.presets = presets;
                    self.user_irs = irs;
                    self.progress = None;
                }
                Evt::Current(p) => {
                    if self.current.as_ref().map(|c| c.index) != Some(p.index) {
                        self.pedal_switched = [false; 9];
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
                    self.status =
                        format!("Salvo em banco {bank} {}", ampero::preset::slot_label(slot));
                }
                Evt::Error(e) => self.status = format!("Erro: {e}"),
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("AMPERO");
            ui.separator();
            ui.label("Porta:");
            ui.add(egui::TextEdit::singleline(&mut self.port).desired_width(110.0));
            if ui
                .button(if self.connected {
                    "Reconectar"
                } else {
                    "Conectar"
                })
                .clicked()
            {
                self.send(Cmd::Connect(self.port.clone()));
            }
            ui.add_enabled_ui(self.connected && self.progress.is_none(), |ui| {
                if ui.button("⟳ Sincronizar").clicked() {
                    self.send(Cmd::Sync);
                }
                if ui.button("Ler atual").clicked() {
                    self.send(Cmd::ReadCurrent);
                }
            });
            ui.separator();
            let dot = if self.connected {
                Color32::from_rgb(80, 200, 120)
            } else {
                Color32::from_rgb(220, 80, 80)
            };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.5, dot);
            if let Some(f) = self.progress {
                ui.add(egui::ProgressBar::new(f).desired_width(160.0));
            }
            ui.label(&self.status);
            if let Some((text, at)) = &self.activity {
                ui.separator();
                let age = at.elapsed().as_secs_f32();
                let color = if age < 1.5 {
                    Color32::from_rgb(240, 190, 70)
                } else {
                    Color32::GRAY
                };
                ui.label(RichText::new(format!("Pedaleira: {text}")).color(color));
                if age < 1.5 {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(200));
                }
            }
        });
    }

    fn preset_list(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.bank, 1, "User Patches");
            ui.selectable_value(&mut self.bank, 0, "Factory");
        });
        ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Buscar…"));
        ui.separator();
        let needle = self.search.to_lowercase();
        let current = self.current.as_ref().map(|c| c.index);
        let mut clicked = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for p in self.presets.iter().filter(|p| p.bank == self.bank) {
                if !needle.is_empty()
                    && !p.name.to_lowercase().contains(&needle)
                    && !p.label.to_lowercase().contains(&needle)
                {
                    continue;
                }
                let text = RichText::new(format!("{}  {}", p.label, p.name)).monospace();
                if ui
                    .selectable_label(current == Some(p.index), text)
                    .clicked()
                {
                    clicked = Some((p.bank, p.slot));
                }
            }
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
        egui::ScrollArea::horizontal()
            .id_salt("chain")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for name in cur.chain.clone() {
                        let k = MODULES.iter().position(|m| *m == name).unwrap_or(0);
                        let block = &mut cur.blocks[k];
                        let thumb = match (&self.resources, block.algorithm_name.as_deref()) {
                            (Some(res), Some(n)) => res.image_path(block.module, n),
                            _ => None,
                        }
                        .and_then(|p| self.texture(ui.ctx(), &p));

                        let fill = if block.on {
                            Color32::from_rgb(70, 55, 20)
                        } else {
                            Color32::from_gray(35)
                        };
                        let stroke = if self.selected_block == k {
                            Color32::from_rgb(230, 180, 60)
                        } else {
                            Color32::from_gray(70)
                        };
                        egui::Frame::new()
                            .fill(fill)
                            .stroke(egui::Stroke::new(1.5, stroke))
                            .corner_radius(6.0)
                            .inner_margin(6.0)
                            .show(ui, |ui| {
                                ui.set_width(CARD_W);
                                ui.vertical(|ui| {
                                    // topo: tipo do bloco + ON/OFF
                                    ui.horizontal(|ui| {
                                        ui.add_sized(
                                            [CARD_W - 42.0, 18.0],
                                            egui::Label::new(RichText::new(block.module).strong()),
                                        );
                                        let (text, color) = if block.on {
                                            ("ON", Color32::from_rgb(240, 190, 70))
                                        } else {
                                            ("OFF", Color32::from_gray(140))
                                        };
                                        let toggle = egui::Button::new(
                                            RichText::new(text).small().strong().color(color),
                                        );
                                        if ui.add_sized([34.0, 18.0], toggle).clicked()
                                            && self.first_pass
                                        {
                                            block.on = !block.on;
                                            self.pedal_switched[k] = false; // alternado pela GUI: é edição
                                            self.send(Cmd::Switch {
                                                block: k,
                                                on: block.on,
                                            });
                                        }
                                    });
                                    // meio: miniatura (clicar seleciona o bloco)
                                    let picture = match &thumb {
                                        Some(tex) => {
                                            let size = tex.size_vec2();
                                            let scale = (THUMB_H / size.y).min(CARD_W / size.x);
                                            let img = egui::Image::new((tex.id(), size * scale));
                                            let img = if block.on {
                                                img
                                            } else {
                                                img.tint(Color32::from_gray(110))
                                            };
                                            egui::Button::image(img).frame(false)
                                        }
                                        None => egui::Button::new(
                                            RichText::new(block.module).size(20.0).weak(),
                                        )
                                        .frame(false),
                                    };
                                    let clicked_picture =
                                        ui.add_sized([CARD_W, THUMB_H], picture).clicked();
                                    // embaixo: nome do efeito
                                    let label = block.algorithm_name.as_deref().unwrap_or("?");
                                    let select = egui::Button::new(RichText::new(label).small())
                                        .frame(false)
                                        .truncate();
                                    let clicked_name =
                                        ui.add_sized([CARD_W, 16.0], select).clicked();
                                    if clicked_picture || clicked_name {
                                        self.selected_block = k;
                                    }
                                });
                            });
                    }
                });
            });
    }

    fn block_editor(&mut self, ui: &mut egui::Ui, cur: &mut Preset) {
        let k = self.selected_block;
        let module = MODULES[k];
        let block = &mut cur.blocks[k];

        ui.horizontal(|ui| {
            ui.label(RichText::new(module).heading());
            let mut alg = block.algorithm;
            egui::ComboBox::from_id_salt("algorithm")
                .width(220.0)
                .selected_text(
                    block
                        .algorithm_name
                        .clone()
                        .unwrap_or_else(|| format!("#{alg}")),
                )
                .show_ui(ui, |ui| {
                    for (code, name) in params::algorithms_of(module) {
                        ui.selectable_value(&mut alg, code, name);
                    }
                });
            if alg != block.algorithm {
                self.send(Cmd::SetAlgorithm { block: k, alg });
            }
        });
        ui.add_space(8.0);

        let name = block.algorithm_name.clone();
        ui.horizontal_top(|ui| {
            self.effect_info(ui, module, name.as_deref());
            ui.separator();
            ui.vertical(|ui| self.param_grid(ui, k, module, block));
        });
    }

    /// Foto + nome + descrição do efeito, como no editor oficial (lidos da instalação dele).
    fn effect_info(&mut self, ui: &mut egui::Ui, module: &str, name: Option<&str>) {
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
                ui.label(RichText::new(n).strong().size(16.0));
            }
            match (&self.resources, description) {
                (_, Some(text)) => {
                    egui::ScrollArea::vertical()
                        .id_salt("description")
                        .max_height(300.0)
                        .show(ui, |ui| description_view(ui, &text));
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
        });
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

    fn param_grid(
        &mut self,
        ui: &mut egui::Ui,
        k: usize,
        module: &str,
        block: &mut ampero::preset::Block,
    ) {
        let Some(meta) = params::algorithm(module, block.algorithm) else {
            ui.label("Sem metadados para este algoritmo (IR do usuário?). Valores brutos:");
            ui.label(format!("{:?}", block.params));
            return;
        };
        let alg = block.algorithm;
        egui::Grid::new("params")
            .num_columns(2)
            .spacing([16.0, 10.0])
            .show(ui, |ui| {
                for pm in &meta.params {
                    let Some(value) = block.params.get_mut(pm.index) else {
                        continue;
                    };
                    ui.label(&pm.name);
                    let before = *value;
                    match pm.kind {
                        Kind::Combox => {
                            let text = pm
                                .options
                                .get(*value as usize)
                                .cloned()
                                .unwrap_or_else(|| value.to_string());
                            egui::ComboBox::from_id_salt(("combo", pm.index))
                                .selected_text(text)
                                .show_ui(ui, |ui| {
                                    for (i, opt) in pm.options.iter().enumerate() {
                                        ui.selectable_value(value, i as u16, opt);
                                    }
                                });
                        }
                        Kind::Switch => {
                            let mut on = *value != 0;
                            if ui.checkbox(&mut on, "").changed() && self.first_pass {
                                *value = on as u16;
                            }
                        }
                        _ => {
                            let pm2 = pm.clone();
                            ui.add(
                                egui::Slider::new(value, pm.range())
                                    .custom_formatter(move |n, _| {
                                        format!("{:.0}", pm2.display(n as u16))
                                    })
                                    .clamping(egui::SliderClamping::Always),
                            );
                        }
                    }
                    if *value != before {
                        self.send(Cmd::SetParam {
                            block: k,
                            alg,
                            param: pm.index as u8,
                            value: *value,
                        });
                    }
                    ui.end_row();
                }
            });
    }

    fn header(&mut self, ui: &mut egui::Ui, cur: &Preset) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{}  {}", cur.label, cur.name))
                    .size(22.0)
                    .strong(),
            );
            let saved = self.presets.iter().find(|p| p.index == cur.index);
            if saved.is_some_and(|s| !same_sound(s, cur, &self.pedal_switched)) {
                ui.label(RichText::new("não salvo").color(Color32::from_rgb(230, 180, 60)));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let can_save = self.connected && cur.bank == 1;
                if ui
                    .add_enabled(can_save, egui::Button::new("💾 Salvar"))
                    .on_disabled_hover_text("só no banco de usuário")
                    .clicked()
                {
                    self.confirm_save = true;
                }
                ui.label(format!(
                    "Volume {}   Tempo {} BPM   {}",
                    cur.volume,
                    cur.tempo,
                    if cur.volume_pre { "Pre" } else { "Post" }
                ));
                // layout da direita para a esquerda: isto aparece à esquerda do volume/tempo
                let (text, color) = if cur.ctrl_state != 0 {
                    ("CTRL ON", Color32::from_rgb(240, 190, 70))
                } else {
                    ("CTRL off", Color32::from_gray(120))
                };
                ui.label(RichText::new(text).strong().color(color))
                    .on_hover_text(format!("estado dos CTRL: 0x{:02X}", cur.ctrl_state));
            });
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.first_pass = ui.ctx().current_pass_index() == 0;
        self.handle_events();
        let ctx = ui.ctx().clone();

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::left("presets")
            .default_size(260.0)
            .show(ui, |ui| self.preset_list(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            let Some(mut cur) = self.current.take() else {
                ui.centered_and_justified(|ui| {
                    ui.label("Conecte o Ampero e sincronize para começar.")
                });
                return;
            };
            self.header(ui, &cur);
            ui.separator();
            self.chain(ui, &mut cur);
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| self.block_editor(ui, &mut cur));

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

/// Descrição do efeito: parágrafos normais; na seção "Controls:" o nome de cada controle vem em destaque,
/// com um pequeno espaço entre os itens; a nota de marcas registradas (`*…`) fica menor e esmaecida.
fn description_view(ui: &mut egui::Ui, text: &str) {
    const SIZE: f32 = 13.5;
    let gold = Color32::from_rgb(240, 190, 70);
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
