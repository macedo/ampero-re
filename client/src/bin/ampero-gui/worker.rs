//! Thread dona da conexão MIDI. A UI manda `Cmd`s e recebe `Evt`s; nada de I/O na thread da UI.

use ampero::device::{Device, all_preset_indices};
use ampero::preset::Preset;
use ampero::protocol as p;
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
                if let Some((slot, _alg, param, value)) = p::parse_param_change(&msg)
                    && (1..=9).contains(&slot)
                {
                    emit(Evt::HardwareParam(slot as usize - 1, param, value));
                }
            }
        }
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
        Cmd::Save { bank, slot, name } => {
            d.send(&p::save_preset(bank, slot))?;
            d.send(&p::preset_name(bank, slot, &name))?;
            thread::sleep(Duration::from_millis(150));
            emit(Evt::Saved(bank, slot));
        }
    }
    Ok(())
}
