//! Conexão MIDI multiplataforma (midir: WinMM / CoreMIDI / ALSA) e operações request/response.

use crate::preset::Preset;
use crate::protocol::{self as p, CHUNK_DATA_START};
use anyhow::{Context, Result, anyhow, bail};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_millis(300);
const RETRIES: usize = 3;

/// Índices de preset lidos na sync: banco 0 (0..=98) e banco 1 (128..=226).
pub fn all_preset_indices() -> impl Iterator<Item = u16> {
    (0..=98u16).chain(128..=226)
}

pub fn list_ports() -> Result<(Vec<String>, Vec<String>)> {
    let input = MidiInput::new("ampero-list")?;
    let output = MidiOutput::new("ampero-list")?;
    let ins = input
        .ports()
        .iter()
        .filter_map(|p| input.port_name(p).ok())
        .collect();
    let outs = output
        .ports()
        .iter()
        .filter_map(|p| output.port_name(p).ok())
        .collect();
    Ok((ins, outs))
}

pub struct Device {
    out: MidiOutputConnection,
    rx: Receiver<Vec<u8>>,
    _in: MidiInputConnection<()>,
    /// Mensagens recebidas que não eram resposta de nenhum pedido (notificações do aparelho).
    pub unsolicited: usize,
    /// Notificações que chegaram durante um pedido; entregues depois por `drain_incoming`.
    pending: Vec<Vec<u8>>,
    pub round_trips: usize,
}

impl Device {
    /// Abre a primeira porta de entrada e de saída cujo nome contém `pattern` (ex.: "Ampero").
    pub fn open(pattern: &str) -> Result<Device> {
        let mut input = MidiInput::new("ampero-client")?;
        input.ignore(Ignore::None); // precisamos de SysEx
        let output = MidiOutput::new("ampero-client")?;

        let pat = pattern.to_lowercase();
        let in_port = input
            .ports()
            .into_iter()
            .find(|port| {
                input
                    .port_name(port)
                    .is_ok_and(|n| n.to_lowercase().contains(&pat))
            })
            .ok_or_else(|| anyhow!("nenhuma porta MIDI de entrada contendo \"{pattern}\""))?;
        let out_port = output
            .ports()
            .into_iter()
            .find(|port| {
                output
                    .port_name(port)
                    .is_ok_and(|n| n.to_lowercase().contains(&pat))
            })
            .ok_or_else(|| anyhow!("nenhuma porta MIDI de saída contendo \"{pattern}\""))?;

        let (tx, rx) = channel();
        let conn_in = input
            .connect(
                &in_port,
                "ampero-in",
                move |_, msg, _| drop(tx.send(msg.to_vec())),
                (),
            )
            .map_err(|e| anyhow!("abrindo entrada MIDI (o editor oficial está aberto?): {e}"))?;
        let conn_out = output
            .connect(&out_port, "ampero-out")
            .map_err(|e| anyhow!("abrindo saída MIDI (o editor oficial está aberto?): {e}"))?;

        Ok(Device {
            out: conn_out,
            rx,
            _in: conn_in,
            unsolicited: 0,
            pending: Vec::new(),
            round_trips: 0,
        })
    }

    /// Mensagens que chegaram sem pedido (mudanças feitas no hardware, pedal de expressão…).
    pub fn drain_incoming(&mut self) -> Vec<Vec<u8>> {
        let mut msgs = std::mem::take(&mut self.pending);
        msgs.extend(self.rx.try_iter());
        msgs
    }

    /// Espera a próxima mensagem do aparelho (qualquer uma), até `timeout`.
    pub fn recv(&mut self, timeout: Duration) -> Option<Vec<u8>> {
        if !self.pending.is_empty() {
            return Some(self.pending.remove(0));
        }
        self.rx.recv_timeout(timeout).ok()
    }

    pub fn send(&mut self, msg: &[u8]) -> Result<()> {
        self.out
            .send(msg)
            .map_err(|e| anyhow!("enviando MIDI: {e}"))
    }

    /// Envia `msg` e devolve a primeira resposta aceita por `accept`, com timeout e retransmissão.
    pub fn request(&mut self, msg: &[u8], accept: impl Fn(&[u8]) -> bool) -> Result<Vec<u8>> {
        for _ in 0..RETRIES {
            self.send(msg)?;
            self.round_trips += 1;
            let deadline = Instant::now() + TIMEOUT;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                match self.rx.recv_timeout(left) {
                    Ok(resp) if accept(&resp) => return Ok(resp),
                    Ok(other) => {
                        self.unsolicited += 1;
                        // guarda mudanças feitas no hardware para a UI (com limite, caso ninguém leia)
                        if p::parse_notification(&other).is_some() && self.pending.len() < 4096 {
                            self.pending.push(other);
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => bail!("conexão MIDI de entrada fechada"),
                }
            }
        }
        bail!("sem resposta do aparelho para {}", hex(msg))
    }

    fn request_cmd(&mut self, msg: &[u8], cmd: [u8; 3]) -> Result<Vec<u8>> {
        let resp = self.request(msg, |r| p::command(r) == Some(cmd))?;
        if !p::frame_checksum_ok(&resp) {
            bail!("checksum inválido na resposta {}", hex(&resp));
        }
        Ok(resp)
    }

    pub fn handshake(&mut self, stage: u8) -> Result<Vec<u8>> {
        self.request_cmd(&p::req_handshake(stage), [0x00, 0x00, 0x05])
    }

    pub fn global_state(&mut self) -> Result<Vec<u8>> {
        self.request_cmd(&p::req_global_state(), [0x00, 0x02, 0x09])
    }

    /// Dados do estado global já tirados dos nibbles (a partir do byte 13 da resposta).
    pub fn global_data(&mut self) -> Result<Vec<u8>> {
        let resp = self.global_state()?;
        Ok(p::from_nibbles(&resp[13..resp.len() - 3]))
    }

    /// Configurações globais decodificadas (função dos footswitches).
    pub fn global_settings(&mut self) -> Result<crate::global::GlobalSettings> {
        let data = self.global_data()?;
        crate::global::GlobalSettings::decode(&data).context("estado global curto demais")
    }

    /// Nome da lista `kind` (0x05 capturas, 0x07 IRs). `None` para slot vazio.
    pub fn name(&mut self, kind: u8, index: u8) -> Result<Option<String>> {
        let resp = self.request(&p::req_name(kind, index), |r| {
            p::command(r) == Some([0x00, 0x02, kind]) && r.get(13) == Some(&index)
        })?;
        if !p::frame_checksum_ok(&resp) {
            bail!("checksum inválido no nome {kind:02X}/{index}");
        }
        let bytes = p::from_nibbles(&resp[14..resp.len() - 3]);
        let name: Vec<u8> = bytes
            .iter()
            .take(16)
            .copied()
            .take_while(|&b| b != 0 && b != 0xFF)
            .collect();
        Ok((!name.is_empty()).then(|| String::from_utf8_lossy(&name).trim_end().to_string()))
    }

    /// Lê um preset salvo (comandos 08 → 0C, 0D → 0E…, 0F → 0E…/10).
    pub fn read_preset(&mut self, index: u16) -> Result<Preset> {
        let [hi, lo] = p::split14(index);
        let addr_ok = move |r: &[u8]| r.get(13) == Some(&hi) && r.get(14) == Some(&lo);
        self.request(&p::req_preset_open(index), |r| {
            p::command(r) == Some([0x00, 0x02, 0x0C]) && addr_ok(r)
        })?;
        let first = self.request(&p::req_preset_first_chunk(index), |r| {
            p::command(r) == Some([0x00, 0x02, 0x0E]) && addr_ok(r)
        })?;
        let raw = self.collect_chunks(
            first,
            0x0E,
            0x10,
            |chunk| p::req_preset_ack_chunk(index, chunk),
            addr_ok,
        )?;
        Preset::decode(index, &raw).with_context(|| format!("preset {index}"))
    }

    /// Lê o buffer de edição (preset atual, incluindo alterações não salvas).
    pub fn read_edit_buffer(&mut self) -> Result<Preset> {
        let any = |_: &[u8]| true;
        self.request(
            &p::frame(p::OP_READ, &[0x00, 0x02, 0x0A, 0x00, 0x00, 0x00]),
            |r| p::command(r) == Some([0x00, 0x02, 0x12]),
        )?;
        let first = self.request(
            &p::frame(p::OP_WRITE, &[0x00, 0x02, 0x13, 0x00, 0x01, 0x00, 0x01]),
            |r| p::command(r) == Some([0x00, 0x02, 0x14]),
        )?;
        let raw = self.collect_chunks(
            first,
            0x14,
            0x16,
            |chunk| {
                p::frame(
                    p::OP_WRITE,
                    &[0x00, 0x02, 0x15, 0x00, 0x01, 0x00, 0x00, chunk, 0x01],
                )
            },
            any,
        )?;
        let index = p::join14(raw[0], raw[1]);
        Preset::decode(index, &raw)
    }

    fn collect_chunks(
        &mut self,
        first: Vec<u8>,
        data_cmd: u8,
        end_cmd: u8,
        ack: impl Fn(u8) -> Vec<u8>,
        addr_ok: impl Fn(&[u8]) -> bool,
    ) -> Result<Vec<u8>> {
        let mut nibs = Vec::with_capacity(1024);
        let mut frame = first;
        loop {
            let chunk = frame[16];
            nibs.extend_from_slice(&frame[CHUNK_DATA_START..frame.len() - 1]);
            frame = self.request(&ack(chunk), |r| {
                matches!(p::command(r), Some([0x00, 0x02, c]) if c == data_cmd || c == end_cmd)
                    && addr_ok(r)
            })?;
            if frame[11] == end_cmd {
                break;
            }
        }
        p::decode_chunked_payload(&nibs).ok_or_else(|| anyhow!("checksum do preset inválido"))
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}
