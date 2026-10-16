//! Thread dona da conexão MIDI. A UI manda `Cmd`s e recebe `Evt`s; nada de I/O na thread da UI.

use ampero::device::{Device, all_preset_indices};
use ampero::preset::{MODULES, Preset};
use ampero::protocol::{self as p, Notification};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

pub enum Cmd {
    Connect(String),
    Sync,
    ReadCurrent,
    Select {
        bank: u8,
        slot: u8,
    },
    SetParam {
        block: usize,
        alg: u16,
        param: u8,
        value: u16,
    },
    SetAlgorithm {
        block: usize,
        alg: u16,
    },
    Switch {
        block: usize,
        on: bool,
    },
    /// Patch Volume (0–99) do buffer de edição.
    SetVolume(u16),
    /// Patch Tempo (40–250 BPM) do buffer de edição.
    SetTempo(u16),
    Save {
        bank: u8,
        slot: u8,
        name: String,
    },
}

pub enum Evt {
    Connected(String),
    Disconnected(String),
    Progress {
        done: usize,
        total: usize,
    },
    Synced {
        presets: Vec<Preset>,
        irs: Vec<Option<String>>,
        secs: f64,
    },
    Current(Preset),
    /// Mudança vinda do hardware: (bloco, parâmetro, valor)
    HardwareParam(usize, u8, u16),
    /// Bloco ligado/desligado no hardware (CTRL, footswitch): (bloco, ligado)
    HardwareSwitch(usize, bool),
    /// Tempo do preset atual (BPM)
    Tempo(u16),
    /// Estado dos CTRL (byte 0x1AB do preset)
    CtrlState(u8),
    /// Descrição de algo que chegou da pedaleira (para a barra de status)
    Activity(String),
    Saved(u8, u8),
    Error(String),
}

pub struct Worker {
    pub tx: Sender<Cmd>,
    pub rx: Receiver<Evt>,
}

impl Worker {
    pub fn spawn(repaint: impl Fn() + Send + 'static) -> Worker {
        let (cmd_tx, cmd_rx) = channel();
        let (evt_tx, evt_rx) = channel();
        thread::spawn(move || run(cmd_rx, evt_tx, repaint));
        Worker {
            tx: cmd_tx,
            rx: evt_rx,
        }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

fn run(rx: Receiver<Cmd>, tx: Sender<Evt>, repaint: impl Fn()) {
    let mut dev: Option<Device> = None;
    // Releitura do buffer agendada após notificações que não sabemos aplicar sozinhos (debounce).
    let mut refresh_at: Option<Instant> = None;
    let emit = |e: Evt| {
        let _ = tx.send(e);
        repaint();
    };
    loop {
        let first = match rx.recv_timeout(Duration::from_millis(30)) {
            Ok(c) => Some(c),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // Junta comandos enfileirados; de SetParam repetidos (arrastar slider) só o último valor importa.
        let mut batch: Vec<Cmd> = first.into_iter().chain(rx.try_iter()).collect();
        coalesce(&mut batch);

        for cmd in batch {
            if let Cmd::Connect(port) = &cmd {
                dev = None; // libera a porta antes de reabrir
                match Device::open(port) {
                    Ok(d) => {
                        dev = Some(d);
                        emit(Evt::Connected(port.clone()));
                    }
                    Err(e) => emit(Evt::Disconnected(format!("{e:#}"))),
                }
                continue;
            }
            let Some(d) = dev.as_mut() else {
                emit(Evt::Error("não conectado".into()));
                continue;
            };
            if let Err(e) = handle(d, cmd, &emit) {
                emit(Evt::Error(format!("{e:#}")));
            }
        }

        if let Some(d) = dev.as_mut() {
            for msg in d.drain_incoming() {
                let notification = p::parse_notification(&msg);
                // Respostas a pedidos de outros programas (o driver entrega a todos) não são eventos da pedaleira.
                if notification.is_some() && !matches!(notification, Some(Notification::Tempo(_))) {
                    emit(Evt::Activity(describe(notification, &msg)));
                }
                match notification {
                    Some(Notification::Param {
                        slot: s @ 1..=9,
                        param,
                        value,
                        ..
                    }) => emit(Evt::HardwareParam(s as usize - 1, param, value)),
                    Some(Notification::Switch {
                        slot: s @ 1..=9,
                        on,
                    }) => emit(Evt::HardwareSwitch(s as usize - 1, on)),
                    Some(Notification::Tempo(bpm)) => emit(Evt::Tempo(bpm)),
                    Some(Notification::CtrlState(v)) => emit(Evt::CtrlState(v)),
                    Some(_) => refresh_at = Some(Instant::now() + Duration::from_millis(150)),
                    None => {}
                }
            }
            if refresh_at.is_some_and(|t| Instant::now() >= t) {
                refresh_at = None;
                match d.read_edit_buffer() {
                    Ok(p) => emit(Evt::Current(p)),
                    Err(e) => emit(Evt::Error(format!("{e:#}"))),
                }
            }
        }
    }
}

/// Texto curto para a barra de status descrevendo o que chegou da pedaleira.
fn describe(n: Option<Notification>, msg: &[u8]) -> String {
    let module = |slot: u8| {
        MODULES
            .get((slot as usize).wrapping_sub(1))
            .copied()
            .unwrap_or("?")
    };
    match n {
        Some(Notification::Param {
            slot, param, value, ..
        }) => {
            format!("{} parâmetro {param} = {value}", module(slot))
        }
        Some(Notification::Switch { slot, on }) => {
            format!(
                "{} {}",
                module(slot),
                if on { "ligado" } else { "desligado" }
            )
        }
        Some(Notification::Algorithm { slot, alg }) => format!("{} algoritmo {alg}", module(slot)),
        Some(Notification::Tempo(bpm)) => format!("tempo {bpm} BPM"),
        Some(Notification::PresetChanged) => "preset trocado".into(),
        Some(Notification::CtrlState(v)) => {
            format!("CTRL {}", if v != 0 { "ativo" } else { "inativo" })
        }
        Some(Notification::Other) | None => format!("mensagem {}", ampero::device::hex(msg)),
    }
}

fn coalesce(batch: &mut Vec<Cmd>) {
    let mut keep = vec![true; batch.len()];
    for i in 0..batch.len() {
        if let Cmd::SetParam { block, param, .. } = batch[i] {
            let superseded = batch[i + 1..]
                .iter()
                .any(|c| matches!(c, Cmd::SetParam { block: b, param: q, .. } if *b == block && *q == param));
            keep[i] = !superseded;
        }
    }
    let mut it = keep.into_iter();
    batch.retain(|_| it.next().unwrap());
}

fn handle(d: &mut Device, cmd: Cmd, emit: &impl Fn(Evt)) -> anyhow::Result<()> {
    match cmd {
        Cmd::Connect(_) => unreachable!(),
        Cmd::Sync => {
            let t0 = Instant::now();
            d.handshake(0)?;
            d.global_state()?;
            let irs = (0..20)
                .map(|i| d.name(0x07, i))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let indices: Vec<u16> = all_preset_indices().collect();
            let mut presets = Vec::with_capacity(indices.len());
            for (n, idx) in indices.iter().enumerate() {
                presets.push(d.read_preset(*idx)?);
                if n % 6 == 0 {
                    emit(Evt::Progress {
                        done: n + 1,
                        total: indices.len(),
                    });
                }
            }
            emit(Evt::Synced {
                presets,
                irs,
                secs: t0.elapsed().as_secs_f64(),
            });
            emit(Evt::Current(d.read_edit_buffer()?));
        }
        Cmd::ReadCurrent => emit(Evt::Current(d.read_edit_buffer()?)),
        Cmd::Select { bank, slot } => {
            for msg in p::select_preset(bank, slot) {
                d.send(&msg)?;
            }
            thread::sleep(Duration::from_millis(120)); // o aparelho carrega o preset antes de responder
            emit(Evt::Current(d.read_edit_buffer()?));
        }
        Cmd::SetParam {
            block,
            alg,
            param,
            value,
        } => d.send(&p::set_param(block as u8 + 1, alg, param, value))?,
        Cmd::SetAlgorithm { block, alg } => {
            d.send(&p::set_algorithm(block as u8 + 1, alg))?;
            thread::sleep(Duration::from_millis(80));
            emit(Evt::Current(d.read_edit_buffer()?)); // parâmetros voltam aos padrões do algoritmo
        }
        Cmd::Switch { block, on } => d.send(&p::block_switch(block, on))?,
        Cmd::SetVolume(v) => d.send(&p::patch_volume(v))?,
        Cmd::SetTempo(bpm) => {
            for msg in p::patch_tempo(bpm) {
                d.send(&msg)?;
            }
        }
        Cmd::Save { bank, slot, name } => {
            d.send(&p::save_preset(bank, slot))?;
            d.send(&p::preset_name(bank, slot, &name))?;
            thread::sleep(Duration::from_millis(150));
            emit(Evt::Saved(bank, slot));
        }
    }
    Ok(())
}
