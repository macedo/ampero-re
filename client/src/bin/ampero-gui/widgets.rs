//! Controles do painel de parâmetros: knob giratório, fader vertical e botões segmentados.

use ampero::params::ParamMeta;
use eframe::egui::{self, Color32, Pos2, Response, Sense, Stroke, Vec2};
use std::f32::consts::PI;

pub const GOLD: Color32 = Color32::from_rgb(240, 190, 70);

/// Dourado para texto: o GOLD no tema escuro, um tom mais escuro (legível sobre branco) no claro.
pub fn accent(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        GOLD
    } else {
        Color32::from_rgb(160, 110, 0)
    }
}

/// Escolhe a cor conforme o tema atual.
pub fn themed(ui: &egui::Ui, dark: Color32, light: Color32) -> Color32 {
    if ui.visuals().dark_mode { dark } else { light }
}

/// Valor exibido com unidade real ("856 ms", "1.2 kHz", "+6 dB", "3.5").
pub fn format_value(pm: &ParamMeta, raw: u16) -> String {
    let v = pm.display(raw);
    let name = pm.name.to_lowercase();
    let signed = pm.display_min < 0.0;
    let num = |v: f64| {
        let decimals = if (pm.display_max - pm.display_min).abs() <= 10.0 {
            1
        } else {
            0
        };
        let s = format!("{v:.decimals$}");
        if signed && v > 0.0 {
            format!("+{s}")
        } else {
            s
        }
    };
    let hz = |v: f64| {
        if v >= 1000.0 {
            format!("{:.1} kHz", v / 1000.0)
        } else {
            format!("{v:.0} Hz")
        }
    };
    if name.contains("cut") || name.starts_with("band") || name.contains("freq") {
        hz(v)
    } else if (name.contains("time") || name.contains("attack") || name.contains("release"))
        && pm.display_max >= 1000.0
    {
        format!("{v:.0} ms")
    } else if name.starts_with("gain") && pm.display_max <= 24.0 && signed {
        format!("{} dB", num(v))
    } else if name.contains("pitch") {
        format!("{} st", num(v))
    } else {
        num(v)
    }
}

/// Converte o que o usuário digitou (na unidade exibida: "856", "856 ms", "1.2k", "1,2 kHz", "+6 dB")
/// para o valor bruto do protocolo, limitado à faixa do parâmetro.
pub fn parse_value(pm: &ParamMeta, text: &str) -> Option<u16> {
    let mut s = text.trim().to_lowercase().replace(',', ".");
    for unit in ["ms", "hz", "db", "st"] {
        if let Some(rest) = s.strip_suffix(unit) {
            s = rest.trim_end().to_string();
        }
    }
    let (num, mult) = match s.strip_suffix('k') {
        Some(rest) => (rest.trim(), 1000.0),
        None => (s.as_str(), 1.0),
    };
    let v: f64 = num.trim_start_matches('+').parse::<f64>().ok()? * mult;
    let raw = if pm.display_max > pm.display_min && pm.max > pm.min {
        pm.min + (v - pm.display_min) * (pm.max - pm.min) / (pm.display_max - pm.display_min)
    } else {
        v
    };
    let range = pm.range();
    Some((raw.round().max(0.0) as u16).clamp(*range.start(), *range.end()))
}

/// Número do valor que vira campo de edição ao ser clicado (Enter ou clicar fora confirma, Esc cancela).
/// O campo flutua sobre o número para não mexer no layout. Retorna true se o valor mudou.
fn inline_value(
    ui: &mut egui::Ui,
    id: egui::Id,
    rect: egui::Rect,
    value: &mut u16,
    pm: &ParamMeta,
    act: bool,
) -> bool {
    let edit_id = id.with("edit");
    let focus_id = id.with("focus");
    let resp = ui.interact(rect, id.with("value"), Sense::click());
    let editing: Option<String> = ui.data(|d| d.get_temp(edit_id));
    let mut changed = false;

    if act && resp.clicked() && editing.is_none() {
        let plain = format_value(pm, *value)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_start_matches('+')
            .to_string();
        ui.data_mut(|d| {
            d.insert_temp(edit_id, plain);
            d.insert_temp(focus_id, true);
        });
    }

    let editing: Option<String> = ui.data(|d| d.get_temp(edit_id));
    if let Some(mut buf) = editing {
        let field = egui::TextEdit::singleline(&mut buf)
            .horizontal_align(egui::Align::Center)
            .font(egui::FontId::proportional(13.0))
            .margin(Vec2::new(2.0, 1.0));
        let field_rect = rect.shrink2(Vec2::new(6.0, 0.0));
        let te = egui::Area::new(edit_id.with("area"))
            .fixed_pos(field_rect.min)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| ui.add_sized(field_rect.size(), field))
            .inner;
        if ui.data(|d| d.get_temp::<bool>(focus_id)).unwrap_or(false) {
            te.request_focus();
            if act {
                ui.data_mut(|d| d.remove::<bool>(focus_id));
            }
        }
        if act {
            if te.lost_focus() {
                ui.data_mut(|d| d.remove::<String>(edit_id));
                let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if let Some(v) = parse_value(pm, &buf).filter(|_| !cancelled)
                    && v != *value
                {
                    *value = v;
                    changed = true;
                }
            } else {
                ui.data_mut(|d| d.insert_temp(edit_id, buf));
            }
        }
    } else {
        let color = if resp.hovered() {
            GOLD
        } else {
            ui.visuals().text_color()
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            format_value(pm, *value),
            egui::FontId::proportional(13.0),
            color,
        );
        resp.on_hover_cursor(egui::CursorIcon::Text);
    }
    changed
}

/// Roda do mouse sobre um controle: devolve (passos, fino). Sem Shift, um passo por frame com rolagem
/// (como sempre foi); com Shift, ajuste fino de **um passo por dente da roda**. O egui transforma a roda
/// com Shift em rolagem horizontal, então os dois eixos contam; o resto de um dente fica guardado em `id`.
fn wheel_steps(ui: &egui::Ui, id: egui::Id) -> (f32, bool) {
    let (delta, shift) = ui.input(|i| {
        (
            i.smooth_scroll_delta.y + i.smooth_scroll_delta.x,
            i.modifiers.shift,
        )
    });
    // pontos por dente da roda (40 no desktop)
    let per_notch = ui
        .ctx()
        .options(|o| o.input_options.line_scroll_speed)
        .max(1.0);
    let key = id.with("wheel-fine");
    if !shift {
        ui.data_mut(|d| d.remove::<f32>(key));
        return (
            if delta.abs() > 0.5 {
                delta.signum()
            } else {
                0.0
            },
            false,
        );
    }
    let acc = ui.data(|d| d.get_temp::<f32>(key)).unwrap_or(0.0) + delta;
    let steps = (acc / per_notch).trunc();
    ui.data_mut(|d| d.insert_temp(key, acc - steps * per_notch));
    (steps, true)
}

/// Knob giratório: arrastar na vertical ajusta (Shift = fino), scroll ajusta (Shift + scroll = 1 por dente),
/// duplo clique volta ao padrão do efeito, clique no número para digitar.
/// `act` = false na passada descartada do egui (multi-pass).
pub fn knob(
    ui: &mut egui::Ui,
    value: &mut u16,
    pm: &ParamMeta,
    saved: Option<u16>,
    act: bool,
    scale: f32,
) -> Response {
    let w: f32 = 74.0 * scale;
    let r: f32 = 24.0 * scale;
    let name_y = 20.0 + 2.0 * r; // relativo ao topo
    let value_y = name_y + 16.0;
    let (rect, mut resp) =
        ui.allocate_exact_size(Vec2::new(w, value_y + 12.0), Sense::click_and_drag());
    let range = pm.range();
    let (lo, hi) = (*range.start() as f32, *range.end() as f32);
    let span = (hi - lo).max(1.0);
    let id = resp.id;

    if act {
        let before = *value;
        if resp.double_clicked() {
            *value = (pm.default as f32).clamp(lo, hi) as u16;
        } else if resp.dragged() {
            let fine = ui.input(|i| i.modifiers.shift);
            // acumulador fracionário para o ajuste fino não se perder no arredondamento
            let mut acc: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(*value as f32);
            if resp.drag_started() {
                acc = *value as f32;
            }
            acc -= resp.drag_delta().y * span / 220.0 * if fine { 0.1 } else { 1.0 };
            acc = acc.clamp(lo, hi);
            ui.data_mut(|d| d.insert_temp(id, acc));
            *value = acc.round() as u16;
        } else if resp.hovered() {
            let (steps, fine) = wheel_steps(ui, id);
            if steps != 0.0 {
                let step = if fine { 1.0 } else { (span / 100.0).max(1.0) };
                *value = (*value as f32 + steps * step).clamp(lo, hi).round() as u16;
            }
        }
        if *value != before {
            resp.mark_changed();
        }
    }

    let painter = ui.painter_at(rect);
    let center = Pos2::new(rect.center().x, rect.top() + 10.0 + r);
    let frac = |v: f32| ((v - lo) / span).clamp(0.0, 1.0);
    let angle = |f: f32| (135.0 + 270.0 * f) * PI / 180.0;
    let at = |a: f32, rad: f32| center + Vec2::new(a.cos(), a.sin()) * rad;
    let arc = |from: f32, to: f32, rad: f32| -> Vec<Pos2> {
        let n = ((to - from).abs() * 24.0).ceil().max(2.0) as usize;
        (0..=n)
            .map(|i| at(angle(from + (to - from) * i as f32 / n as f32), rad))
            .collect()
    };
    let track = ui
        .visuals()
        .widgets
        .inactive
        .bg_stroke
        .color
        .gamma_multiply(2.0);
    painter.add(egui::Shape::line(
        arc(0.0, 1.0, r),
        Stroke::new(4.0 * scale.sqrt(), track),
    ));
    let f = frac(*value as f32);
    if f > 0.0 {
        painter.add(egui::Shape::line(
            arc(0.0, f, r),
            Stroke::new(4.0 * scale.sqrt(), GOLD),
        ));
    }
    let knob_fill = if resp.hovered() || resp.dragged() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    painter.circle(center, r * 0.67, knob_fill, Stroke::new(1.0, track));
    painter.line_segment(
        [center, at(angle(f), r * 0.58)],
        Stroke::new(2.5, ui.visuals().strong_text_color()),
    );
    // marca do valor padrão na borda
    painter.circle_filled(
        at(angle(frac(pm.default as f32)), r + 6.0),
        1.8,
        ui.visuals().weak_text_color(),
    );
    painter.text(
        Pos2::new(rect.center().x, rect.top() + name_y),
        egui::Align2::CENTER_CENTER,
        &pm.name,
        egui::FontId::proportional(12.5),
        ui.visuals().weak_text_color(),
    );

    let value_rect = egui::Rect::from_center_size(
        Pos2::new(rect.center().x, rect.top() + value_y),
        Vec2::new(w - 12.0, 18.0),
    );
    if inline_value(ui, id, value_rect, value, pm, act) {
        resp.mark_changed();
    }
    if saved.is_some_and(|s| s != *value) {
        painter.circle_filled(
            Pos2::new(rect.right() - 4.0, rect.top() + value_y),
            3.0,
            GOLD,
        );
    }
    resp.on_hover_text(format!(
        "{}: {}\npadrão {} · arraste ou role a roda (Shift = fino), duplo clique para o padrão\nclique no número para digitar o valor",
        pm.name,
        format_value(pm, *value),
        format_value(pm, pm.default as u16)
    ))
}

/// Banco de faders verticais (bandas de EQ) com linhas horizontais de referência.
/// Arrasto relativo e em meia velocidade (não pula para onde clicou), Shift = 5× mais fino, scroll ou setas ↑/↓ = 1 em 1,
/// duplo clique = padrão, clique no número para digitar. Retorna os índices (em `metas`) que mudaram.
pub fn fader_bank(
    ui: &mut egui::Ui,
    metas: &[&ParamMeta],
    values: &mut [u16],
    saved: &[Option<u16>],
    act: bool,
) -> Vec<usize> {
    const COL: f32 = 60.0;
    const LABEL_W: f32 = 40.0;
    const TOP: f32 = 10.0;
    const TRACK_H: f32 = 150.0;
    let mut changed = Vec::new();
    let Some(first) = metas.first() else {
        return changed;
    };

    let size = Vec2::new(LABEL_W + COL * metas.len() as f32, TOP + TRACK_H + 46.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let track_top = rect.top() + TOP;
    let track_bot = track_top + TRACK_H;
    let y_of = |f: f32| track_bot - f * TRACK_H;
    let weak = ui.visuals().weak_text_color();
    let line = ui.visuals().widgets.inactive.bg_stroke.color;

    // linhas de referência (0 %, 25 %, 50 %, 75 %, 100 % do curso) com rótulos na unidade do primeiro fader
    let painter = ui.painter_at(rect);
    let range = first.range();
    for f in [0.0, 0.25, 0.5, 0.75, 1.0] {
        let y = y_of(f);
        let strong = f == 0.5 && first.display_min < 0.0;
        let stroke = if strong {
            Stroke::new(1.2, line.gamma_multiply(2.5))
        } else {
            Stroke::new(0.6, line)
        };
        painter.line_segment(
            [
                Pos2::new(rect.left() + LABEL_W - 4.0, y),
                Pos2::new(rect.right(), y),
            ],
            stroke,
        );
        let raw = *range.start() as f32 + f * (*range.end() - *range.start()) as f32;
        painter.text(
            Pos2::new(rect.left() + LABEL_W - 8.0, y),
            egui::Align2::RIGHT_CENTER,
            format_value(first, raw.round() as u16),
            egui::FontId::proportional(11.0),
            weak,
        );
    }

    for (i, pm) in metas.iter().enumerate() {
        let value = &mut values[i];
        let range = pm.range();
        let (lo, hi) = (*range.start() as f32, *range.end() as f32);
        let span = (hi - lo).max(1.0);
        let frac = |v: f32| ((v - lo) / span).clamp(0.0, 1.0);
        let cx = rect.left() + LABEL_W + COL * (i as f32 + 0.5);
        let col_rect = egui::Rect::from_min_max(
            Pos2::new(cx - COL / 2.0 + 4.0, track_top - 8.0),
            Pos2::new(cx + COL / 2.0 - 4.0, track_bot + 8.0),
        );
        let id = ui.id().with(("fader", pm.index));
        let resp = ui.interact(col_rect, id, Sense::click_and_drag());
        let before = *value;

        if act {
            if resp.double_clicked() {
                *value = (pm.default as f32).clamp(lo, hi) as u16;
            } else if resp.dragged() {
                let fine = ui.input(|i| i.modifiers.shift);
                let mut acc: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(*value as f32);
                if resp.drag_started() {
                    acc = *value as f32;
                }
                // meia velocidade: o curso todo pede o dobro do movimento (≈3 px por unidade); Shift = 5× mais fino
                acc -= resp.drag_delta().y * span / TRACK_H * if fine { 0.1 } else { 0.5 };
                acc = acc.clamp(lo, hi);
                ui.data_mut(|d| d.insert_temp(id, acc));
                *value = acc.round() as u16;
            } else if resp.hovered() {
                let (up, down) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::ArrowUp),
                        i.key_pressed(egui::Key::ArrowDown),
                    )
                });
                // roda: 1 por frame; Shift + roda: 1 por dente
                let (wheel, _) = wheel_steps(ui, id);
                let step = if up {
                    1.0
                } else if down {
                    -1.0
                } else {
                    wheel
                };
                *value = (*value as f32 + step).clamp(lo, hi) as u16;
            }
        }

        // trilho, preenchimento (a partir do centro se o parâmetro tem sinal) e alça
        let active = resp.hovered() || resp.dragged();
        let track = ui.visuals().widgets.inactive.bg_fill;
        painter.line_segment(
            [Pos2::new(cx, track_top), Pos2::new(cx, track_bot)],
            Stroke::new(4.0, track),
        );
        let base = if pm.display_min < 0.0 { 0.5 } else { 0.0 };
        let yv = y_of(frac(*value as f32));
        painter.line_segment(
            [Pos2::new(cx, y_of(base)), Pos2::new(cx, yv)],
            Stroke::new(4.0, GOLD),
        );
        let handle = egui::Rect::from_center_size(Pos2::new(cx, yv), Vec2::new(26.0, 12.0));
        let handle_fill = if active {
            GOLD
        } else {
            ui.visuals().widgets.hovered.bg_fill
        };
        painter.rect(
            handle,
            3.0,
            handle_fill,
            Stroke::new(1.0, line.gamma_multiply(2.0)),
            egui::StrokeKind::Middle,
        );
        painter.line_segment(
            [
                Pos2::new(handle.left() + 5.0, yv),
                Pos2::new(handle.right() - 5.0, yv),
            ],
            Stroke::new(1.0, if active { Color32::BLACK } else { weak }),
        );

        let value_rect = egui::Rect::from_center_size(
            Pos2::new(cx, track_bot + 18.0),
            Vec2::new(COL - 6.0, 18.0),
        );
        let typed = inline_value(ui, id, value_rect, value, pm, act);
        if saved[i].is_some_and(|s| s != *value) {
            painter.circle_filled(Pos2::new(cx + COL / 2.0 - 6.0, track_bot + 18.0), 3.0, GOLD);
        }
        painter.text(
            Pos2::new(cx, track_bot + 36.0),
            egui::Align2::CENTER_CENTER,
            &pm.name,
            egui::FontId::proportional(11.5),
            weak,
        );
        resp.on_hover_text(format!(
            "{}: {}\narraste ou role a roda (Shift = fino), ↑/↓ de 1 em 1, duplo clique = padrão ({})",
            pm.name,
            format_value(pm, *value),
            format_value(pm, pm.default as u16)
        ));
        if typed || *value != before {
            changed.push(i);
        }
    }
    changed
}

/// Desenha `add` centralizado na horizontal. Usa a largura medida no frame anterior; se ela mudou,
/// pede uma passada extra ao egui (request_discard) para não aparecer deslocado.
pub fn centered_row<R>(ui: &mut egui::Ui, id: egui::Id, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let width_id = id.with("centered-width");
    let last: f32 = ui.data(|d| d.get_temp(width_id)).unwrap_or(0.0);
    let pad = ((ui.available_width() - last) / 2.0).max(0.0);
    let (inner, width) = ui
        .horizontal(|ui| {
            ui.add_space(pad);
            let r = ui.scope(add);
            (r.inner, r.response.rect.width())
        })
        .inner;
    if (width - last).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(width_id, width));
        ui.ctx().request_discard("centralizar controles");
    }
    inner
}

/// Escala dos knobs conforme a quantidade: poucos ficam grandes, muitos ficam compactos.
pub fn knob_scale(count: usize) -> f32 {
    match count {
        0..=3 => 1.5,
        4..=6 => 1.3,
        _ => 1.05,
    }
}

/// Botões segmentados (opções curtas e on/off). Retorna true se o valor mudou.
pub fn segmented(ui: &mut egui::Ui, value: &mut u16, options: &[&str], act: bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, opt) in options.iter().enumerate() {
            let selected = *value as usize == i;
            let text = egui::RichText::new(*opt).size(13.0);
            let text = if selected {
                text.color(Color32::BLACK)
            } else {
                text
            };
            let button = egui::Button::new(text).fill(if selected {
                GOLD
            } else {
                ui.visuals().widgets.inactive.weak_bg_fill
            });
            if ui.add(button).clicked() && act && !selected {
                *value = i as u16;
                changed = true;
            }
        }
    });
    changed
}

/// Knob compacto (só o disco, sem nome nem valor) para a barra do preset: arrastar para cima ou para a
/// direita aumenta (Shift = ajuste fino), a roda do mouse anda de 1 em 1. `resp.changed()` quando o valor
/// muda. Mesmo visual dos knobs do painel: trilha, arco dourado do valor e ponteiro.
pub fn mini_knob(
    ui: &mut egui::Ui,
    value: &mut u16,
    range: std::ops::RangeInclusive<u16>,
    diameter: f32,
    act: bool,
) -> Response {
    let (rect, mut resp) =
        ui.allocate_exact_size(Vec2::splat(diameter + 4.0), Sense::click_and_drag());
    let (lo, hi) = (*range.start() as f32, *range.end() as f32);
    let span = (hi - lo).max(1.0);
    let id = resp.id;
    let before = *value;
    // só na primeira passada do egui: na passada extra o mesmo arraste seria somado de novo
    if !act {
    } else if resp.dragged() {
        let fine = ui.input(|i| i.modifiers.shift);
        let mut acc: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(*value as f32);
        if resp.drag_started() {
            acc = *value as f32;
        }
        let d = resp.drag_delta();
        acc += (d.x - d.y) * span / 200.0 * if fine { 0.1 } else { 1.0 };
        acc = acc.clamp(lo, hi);
        ui.data_mut(|data| data.insert_temp(id, acc));
        *value = acc.round() as u16;
    } else if resp.hovered() {
        // roda: 1 por frame; Shift + roda: 1 por dente
        let (steps, _) = wheel_steps(ui, id);
        if steps != 0.0 {
            *value = (*value as f32 + steps).clamp(lo, hi) as u16;
        }
    }
    if *value != before {
        resp.mark_changed();
    }

    let painter = ui.painter_at(rect);
    let center = rect.center();
    let r = diameter / 2.0;
    let frac = ((*value as f32 - lo) / span).clamp(0.0, 1.0);
    let angle = |f: f32| (135.0 + 270.0 * f) * PI / 180.0;
    let at = |a: f32, rad: f32| center + Vec2::new(a.cos(), a.sin()) * rad;
    let arc = |to: f32| -> Vec<Pos2> {
        let n = (to * 24.0).ceil().max(2.0) as usize;
        (0..=n)
            .map(|i| at(angle(to * i as f32 / n as f32), r))
            .collect()
    };
    let track = ui
        .visuals()
        .widgets
        .inactive
        .bg_stroke
        .color
        .gamma_multiply(2.0);
    painter.add(egui::Shape::line(arc(1.0), Stroke::new(2.5, track)));
    if frac > 0.0 {
        painter.add(egui::Shape::line(arc(frac), Stroke::new(2.5, GOLD)));
    }
    let fill = if resp.hovered() || resp.dragged() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        ui.visuals().widgets.inactive.bg_fill
    };
    painter.circle(center, r * 0.62, fill, Stroke::new(1.0, track));
    painter.line_segment(
        [center, at(angle(frac), r * 0.55)],
        Stroke::new(2.0, ui.visuals().strong_text_color()),
    );
    resp.on_hover_cursor(egui::CursorIcon::ResizeVertical)
}

/// Estado de um número "clique para digitar" (volume, tempo): o texto em edição pertence a um preset
/// (`owner`), para não cair noutro se o preset trocar com o campo aberto.
#[derive(Default)]
pub struct NumberEdit {
    edit: Option<(u16, String)>,
    error: bool,
    focus: bool,
}

/// O que não muda num campo "clique para digitar": faixa, sufixo exibido, dica e largura fixa.
pub struct NumberField<'a> {
    pub range: std::ops::RangeInclusive<u16>,
    pub suffix: &'a str,
    pub hover: &'a str,
    /// Largura fixa (texto e campo de digitação): dentro do cabeçalho, montado da direita para a
    /// esquerda, um número que ganha ou perde um dígito deslocava o slider ao lado durante o arraste.
    pub width: f32,
}

/// Mostra `value` como texto clicável; o clique abre um campo só de digitar (sem arrastar), Enter
/// confirma, Esc cancela e um valor fora da faixa fica marcado em vermelho. Devolve o valor novo quando
/// confirmado e diferente do atual.
pub fn number_edit(
    ui: &mut egui::Ui,
    state: &mut NumberEdit,
    owner: u16,
    value: u16,
    field: &NumberField,
) -> Option<u16> {
    if state.edit.as_ref().is_some_and(|(o, _)| *o != owner) {
        *state = NumberEdit::default();
    }
    let size = [field.width, 18.0];
    let Some((_, buf)) = state.edit.as_mut() else {
        let text = egui::RichText::new(format!("{value}{}", field.suffix)).strong();
        let resp = ui
            .add_sized(size, egui::Label::new(text).sense(Sense::click()))
            .on_hover_cursor(egui::CursorIcon::Text)
            .on_hover_text(field.hover);
        if resp.clicked() {
            *state = NumberEdit {
                edit: Some((owner, value.to_string())),
                error: false,
                focus: true,
            };
        }
        return None;
    };
    if state.error {
        ui.colored_label(
            Color32::from_rgb(230, 110, 100),
            format!("{} a {}", field.range.start(), field.range.end()),
        );
    }
    let resp = ui.add_sized(
        size,
        egui::TextEdit::singleline(buf)
            .char_limit(3)
            .horizontal_align(egui::Align::Center),
    );
    if std::mem::take(&mut state.focus) {
        resp.request_focus();
    }
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.edit = None;
        return None;
    }
    if !resp.lost_focus() {
        return None;
    }
    match buf.trim().parse::<u16>() {
        Ok(v) if field.range.contains(&v) => {
            state.edit = None;
            (v != value).then_some(v)
        }
        _ => {
            state.error = true;
            state.focus = true;
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(name: &str, dmin: f64, dmax: f64, min: f64, max: f64) -> ParamMeta {
        serde_json::from_value(serde_json::json!({
            "index": 0, "name": name, "kind": "knob", "min": min, "max": max,
            "display_min": dmin, "display_max": dmax, "default": 0, "options": []
        }))
        .unwrap()
    }

    #[test]
    fn formats_values_with_real_units() {
        assert_eq!(
            format_value(&meta("Time", 20.0, 4000.0, 0.0, 3980.0), 836),
            "856 ms"
        );
        assert_eq!(
            format_value(&meta("Low Cut", 19.0, 2000.0, 0.0, 1981.0), 1181),
            "1.2 kHz"
        );
        assert_eq!(
            format_value(&meta("Gain 1", -12.0, 12.0, 0.0, 24.0), 18),
            "+6 dB"
        );
        assert_eq!(format_value(&meta("Q 1", 0.1, 10.0, 0.0, 99.0), 34), "3.5");
        assert_eq!(
            format_value(&meta("Gain", 0.0, 100.0, 0.0, 100.0), 52),
            "52"
        );
        assert_eq!(
            format_value(&meta("Pitch", -24.0, 24.0, 0.0, 48.0), 19),
            "-5 st"
        );
    }

    #[test]
    fn parses_typed_values_in_display_units() {
        let time = meta("Time", 20.0, 4000.0, 0.0, 3980.0);
        assert_eq!(parse_value(&time, "856"), Some(836));
        assert_eq!(parse_value(&time, "856 ms"), Some(836));
        assert_eq!(parse_value(&time, "9999"), Some(3980)); // limitado à faixa
        let cut = meta("Low Cut", 19.0, 2000.0, 0.0, 1981.0);
        assert_eq!(parse_value(&cut, "1.2k"), Some(1181));
        assert_eq!(parse_value(&cut, "1,2 kHz"), Some(1181));
        assert_eq!(
            parse_value(&meta("Gain 1", -12.0, 12.0, 0.0, 24.0), "+6 dB"),
            Some(18)
        );
        assert_eq!(
            parse_value(&meta("Q 1", 0.1, 10.0, 0.0, 99.0), "3.5"),
            Some(34)
        );
        assert_eq!(
            parse_value(&meta("Gain", 0.0, 100.0, 0.0, 100.0), "abc"),
            None
        );
    }
}
