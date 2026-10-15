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
    dirty: bool,
    confirm_save: bool,
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
            dirty: false,
            confirm_save: false,
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
                    self.current = Some(p);
                    self.dirty = false;
                }
                Evt::HardwareParam(block, param, value) => {
                    if let Some(p) = self.current.as_mut().and_then(|c| c.blocks.get_mut(block))
                        && let Some(v) = p.params.get_mut(param as usize)
                    {
                        *v = value;
                        self.dirty = true;
                    }
                }
                Evt::Saved(bank, slot) => {
                    let index = bank as u16 * 128 + slot as u16;
                    if let Some(cur) = &self.current
                        && let Some(entry) = self.presets.iter_mut().find(|p| p.index == index)
                    {
                        *entry = cur.clone();
                    }
                    self.dirty = false;
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

    fn chain(&mut self, ui: &mut egui::Ui, cur: &mut Preset) {
        const TILE: f32 = 112.0;
        // largura total do tile: conteúdo + margens (2×6) + borda + espaçamento entre itens
        let outer = TILE + 12.0 + 3.0 + ui.spacing().item_spacing.x;
        let per_row = (ui.available_width() / outer).floor().max(1.0) as usize;
        for row in cur.chain.clone().chunks(per_row) {
            ui.horizontal(|ui| {
                for &name in row {
                    let k = MODULES.iter().position(|m| *m == name).unwrap_or(0);
                    let block = &mut cur.blocks[k];
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
                            ui.set_width(TILE);
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.add_sized(
                                        [TILE - 46.0, 18.0],
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
                                    if ui.add_sized([38.0, 18.0], toggle).clicked()
                                        && self.first_pass
                                    {
                                        block.on = !block.on;
                                        self.send(Cmd::Switch {
                                            block: k,
                                            on: block.on,
                                        });
                                        self.dirty = true;
                                    }
                                });
                                let label = block.algorithm_name.as_deref().unwrap_or("?");
                                let select = egui::Button::new(RichText::new(label).small())
                                    .frame(false)
                                    .truncate();
                                if ui.add_sized([TILE, 16.0], select).clicked() {
                                    self.selected_block = k;
                                }
                            });
                        });
                }
            });
        }
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
                self.dirty = true;
            }
        });
        ui.add_space(8.0);

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
                        self.dirty = true;
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
            if self.dirty || saved.is_some_and(|s| !same_sound(s, cur)) {
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

/// Compara o que afeta o som (blocos, cadeia, volume), ignorando metadados.
fn same_sound(a: &Preset, b: &Preset) -> bool {
    a.volume == b.volume
        && a.chain == b.chain
        && a.blocks
            .iter()
            .zip(&b.blocks)
            .all(|(x, y)| x.on == y.on && x.algorithm == y.algorithm && x.params == y.params)
}
