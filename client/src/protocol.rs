//! Formato dos frames SysEx do Ampero (ver PROTOCOLO.md).
//!
//! ```text
//! F0 21 25 7F 4D 50 2D 64 | OP | G1 G2 | CMD | ARGS... | DATA... | [CK_hi CK_lo] | F7
//! ```

pub const HEADER: [u8; 8] = [0xF0, 0x21, 0x25, 0x7F, 0x4D, 0x50, 0x2D, 0x64];
pub const OP_READ: u8 = 0x11;
pub const OP_WRITE: u8 = 0x12;

/// Canal MIDI usado para CC/PC (canal 6, 0-based 5).
pub const MIDI_CHANNEL: u8 = 5;

/// Índice do primeiro byte de dados nos chunks de preset (`0E`/`14`).
pub const CHUNK_DATA_START: usize = 17;

pub fn frame(op: u8, body: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(HEADER.len() + 2 + body.len());
    f.extend_from_slice(&HEADER);
    f.push(op);
    f.extend_from_slice(body);
    f.push(0xF7);
    f
}

pub fn is_ampero_sysex(msg: &[u8]) -> bool {
    msg.len() > HEADER.len() + 1 && msg.starts_with(&HEADER) && msg.last() == Some(&0xF7)
}

/// `[G1, G2, CMD]` de uma resposta (`msg[9..12]`).
pub fn command(msg: &[u8]) -> Option<[u8; 3]> {
    if !is_ampero_sysex(msg) || msg.len() < 13 {
        return None;
    }
    Some([msg[9], msg[10], msg[11]])
}

/// Divide cada byte em dois nibbles (alto primeiro).
pub fn to_nibbles(data: &[u8]) -> Vec<u8> {
    data.iter().flat_map(|b| [b >> 4, b & 0x0F]).collect()
}

/// Junta pares de nibbles em bytes. Ignora um nibble final ímpar.
pub fn from_nibbles(nibs: &[u8]) -> Vec<u8> {
    let (pairs, _) = nibs.as_chunks::<2>();
    pairs
        .iter()
        .map(|[hi, lo]| (hi << 4) | (lo & 0x0F))
        .collect()
}

/// Valor de 14 bits em dois bytes de 7 bits (`hi lo`).
pub fn split14(v: u16) -> [u8; 2] {
    [((v >> 7) & 0x7F) as u8, (v & 0x7F) as u8]
}

pub fn join14(hi: u8, lo: u8) -> u16 {
    ((hi as u16) << 7) | lo as u16
}

/// Checksum dos frames simples: soma de 8 bits de `msg[13..len-3]`,
/// guardada em dois nibbles logo antes do `F7` (rotina 0x58C9F0 do editor oficial).
pub fn frame_checksum_ok(msg: &[u8]) -> bool {
    let n = msg.len();
    if n < 17 {
        return true; // frames curtos não têm checksum
    }
    let sum = msg[13..n - 3].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    sum == (msg[n - 3] << 4) | msg[n - 2]
}

/// Checksum do preset remontado: soma de 8 bits de `buf[..n-2]` contra os dois últimos nibbles.
/// Retorna os bytes decodificados (sem o checksum) quando confere.
pub fn decode_chunked_payload(nibs: &[u8]) -> Option<Vec<u8>> {
    let n = nibs.len();
    if n < 2 || !n.is_multiple_of(2) {
        return None;
    }
    let sum = nibs[..n - 2].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    if sum != (nibs[n - 2] << 4) | nibs[n - 1] {
        return None;
    }
    Some(from_nibbles(&nibs[..n - 2]))
}

// ---- pedidos conhecidos ----

pub fn req_handshake(stage: u8) -> Vec<u8> {
    frame(OP_READ, &[0x00, 0x00, 0x05, stage])
}

pub fn req_global_state() -> Vec<u8> {
    frame(OP_READ, &[0x00, 0x02, 0x09, 0x00])
}

/// Lista de nomes: `kind` 0x05 (capturas/clones, 11 entradas) ou 0x07 (IRs do usuário, 20 entradas).
pub fn req_name(kind: u8, index: u8) -> Vec<u8> {
    frame(OP_READ, &[0x00, 0x02, kind, 0x00, index])
}

pub fn req_preset_open(index: u16) -> Vec<u8> {
    let [hi, lo] = split14(index);
    frame(OP_READ, &[0x00, 0x02, 0x08, 0x00, hi, lo])
}

pub fn req_preset_first_chunk(index: u16) -> Vec<u8> {
    let [hi, lo] = split14(index);
    frame(OP_WRITE, &[0x00, 0x02, 0x0D, 0x00, hi, lo, 0x01])
}

pub fn req_preset_ack_chunk(index: u16, chunk: u8) -> Vec<u8> {
    let [hi, lo] = split14(index);
    frame(
        OP_WRITE,
        &[0x00, 0x02, 0x0F, 0x00, hi, lo, 0x00, chunk, 0x01],
    )
}

// ---- edição ao vivo ----

/// Altera um parâmetro no buffer de edição. `slot` = índice de armazenamento do bloco + 1.
pub fn set_param(slot: u8, alg: u16, param: u8, value: u16) -> Vec<u8> {
    let [ah, al] = split14(alg);
    let [vh, vl] = split14(value);
    frame(OP_WRITE, &[0x10, slot, 0x00, 0x02, ah, al, param, vh, vl])
}

/// Mensagens que o aparelho envia sozinho quando algo muda no hardware (footswitch, CTRL, knobs, pedal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notification {
    /// Mesmo formato de [`set_param`]: `(slot, algoritmo, parâmetro, valor)`.
    Param {
        slot: u8,
        alg: u16,
        param: u8,
        value: u16,
    },
    /// `12 10 SS 00 00 VV` — bloco ligado (VV ≥ 0x40) ou desligado (CTRL, footswitch).
    Switch { slot: u8, on: bool },
    /// `12 10 SS 00 01 AH AL` — algoritmo trocado no aparelho.
    Algorithm { slot: u8, alg: u16 },
    /// `12 00 02 06 05 00 00 TH TL` — tempo atual em BPM (chega junto com cada mudança).
    Tempo(u16),
    /// `12 00 02 06 04 …` — preset trocado no aparelho.
    PresetChanged,
    /// `12 00 02 06 23 VV` — estado dos CTRL (byte 0x1AB do preset) após apertar um footswitch.
    CtrlState(u8),
    /// Outra notificação `12 00 02 06 …` ainda não decodificada (ex.: `24` dos footswitches).
    Other,
}

pub fn parse_notification(msg: &[u8]) -> Option<Notification> {
    if !is_ampero_sysex(msg) || msg.len() < 14 || msg[8] != OP_WRITE {
        return None;
    }
    let n = msg.len();
    match (msg[9], msg[11], msg[12]) {
        (0x10, 0x00, 0x02) if n == 19 => Some(Notification::Param {
            slot: msg[10],
            alg: join14(msg[13], msg[14]),
            param: msg[15],
            value: join14(msg[16], msg[17]),
        }),
        (0x10, 0x00, 0x00) if n == 15 => Some(Notification::Switch {
            slot: msg[10],
            on: msg[13] >= 0x40,
        }),
        (0x10, 0x00, 0x01) if n == 16 => Some(Notification::Algorithm {
            slot: msg[10],
            alg: join14(msg[13], msg[14]),
        }),
        (0x00, 0x06, 0x05) if msg[10] == 0x02 && n == 18 => {
            Some(Notification::Tempo(join14(msg[15], msg[16])))
        }
        (0x00, 0x06, 0x04) if msg[10] == 0x02 => Some(Notification::PresetChanged),
        (0x00, 0x06, 0x23) if msg[10] == 0x02 && n == 15 => Some(Notification::CtrlState(msg[13])),
        (0x00, 0x06, _) if msg[10] == 0x02 => Some(Notification::Other),
        _ => None,
    }
}

/// Troca o algoritmo de um bloco no buffer de edição.
pub fn set_algorithm(slot: u8, alg: u16) -> Vec<u8> {
    let [ah, al] = split14(alg);
    frame(OP_WRITE, &[0x10, slot, 0x00, 0x01, ah, al])
}

/// Grava o buffer de edição no banco/slot (o editor oficial envia isto e, em seguida, [`preset_name`]).
pub fn save_preset(bank: u8, slot: u8) -> Vec<u8> {
    frame(
        OP_WRITE,
        &[0x00, 0x02, 0x06, 0x07, bank & 0x7F, slot & 0x7F],
    )
}

/// Nome do preset: até 16 caracteres ASCII num campo de 17 bytes (padding/terminador `00`).
pub fn preset_name(bank: u8, slot: u8, name: &str) -> Vec<u8> {
    let mut body = vec![0x10, 0x00, 0x00, 0x00, bank & 0x7F, slot & 0x7F];
    let ascii = name.bytes().filter(|b| (0x20..0x7F).contains(b)).take(16);
    let mut field = [0u8; 17];
    ascii.enumerate().for_each(|(i, b)| field[i] = b);
    body.extend_from_slice(&field);
    frame(OP_WRITE, &body)
}

/// Seleção de preset: CC0 (banco) + Program Change no canal 6.
pub fn select_preset(bank: u8, slot: u8) -> [Vec<u8>; 2] {
    [
        vec![0xB0 | MIDI_CHANNEL, 0x00, bank & 0x7F],
        vec![0xC0 | MIDI_CHANNEL, slot & 0x7F],
    ]
}

/// Liga/desliga o bloco `storage_index` (0 = FX1 … 8 = RVB) via CC 48+índice (capturado do editor oficial).
pub fn block_switch(storage_index: usize, on: bool) -> Vec<u8> {
    vec![
        0xB0 | MIDI_CHANNEL,
        48 + storage_index as u8,
        if on { 0x7F } else { 0x00 },
    ]
}

/// Ordem da cadeia do buffer de edição. `order` = índices de armazenamento (FX1=0 … RVB=8) na ordem em
/// que o sinal passa; a mensagem leva a **posição de cada bloco**, na ordem de armazenamento (capturado do
/// "Manage Signal Chain" do editor oficial). `None` se `order` não for uma permutação dos 9 blocos.
pub fn chain_order(order: &[usize]) -> Option<Vec<u8>> {
    let mut pos = [u8::MAX; 9];
    for (i, &block) in order.iter().enumerate() {
        if i >= 9 || block >= 9 || pos[block] != u8::MAX {
            return None;
        }
        pos[block] = i as u8;
    }
    if pos.contains(&u8::MAX) {
        return None;
    }
    let mut body = vec![0x00, 0x02, 0x06, 0x0D];
    body.extend_from_slice(&pos);
    Some(frame(OP_WRITE, &body))
}

pub const VOLUME_MAX: u16 = 99;
pub const TEMPO_MIN: u16 = 40;
pub const TEMPO_MAX: u16 = 250;

/// Patch Volume do buffer de edição: CC 7 (0–99), capturado do editor oficial.
pub fn patch_volume(volume: u16) -> Vec<u8> {
    vec![0xB0 | MIDI_CHANNEL, 0x07, volume.min(VOLUME_MAX) as u8]
}

/// Patch Tempo do buffer de edição (40–250 BPM): CC 73 com os 7 bits altos e CC 74 com os baixos,
/// capturado do editor oficial (150 BPM = `49 01` + `4A 16`).
pub fn patch_tempo(bpm: u16) -> [Vec<u8>; 2] {
    let bpm = bpm.clamp(TEMPO_MIN, TEMPO_MAX);
    [
        vec![0xB0 | MIDI_CHANNEL, 73, (bpm >> 7) as u8],
        vec![0xB0 | MIDI_CHANNEL, 74, (bpm & 0x7F) as u8],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|h| u8::from_str_radix(h, 16).unwrap())
            .collect()
    }

    #[test]
    fn builds_frames_exactly_like_the_official_editor() {
        assert_eq!(
            req_handshake(0),
            hex("F0 21 25 7F 4D 50 2D 64 11 00 00 05 00 F7")
        );
        assert_eq!(
            req_name(0x05, 3),
            hex("F0 21 25 7F 4D 50 2D 64 11 00 02 05 00 03 F7")
        );
        assert_eq!(
            req_preset_open(212),
            hex("F0 21 25 7F 4D 50 2D 64 11 00 02 08 00 01 54 F7")
        );
        assert_eq!(
            req_preset_first_chunk(1),
            hex("F0 21 25 7F 4D 50 2D 64 12 00 02 0D 00 00 01 01 F7")
        );
        assert_eq!(
            req_preset_ack_chunk(1, 0),
            hex("F0 21 25 7F 4D 50 2D 64 12 00 02 0F 00 00 01 00 00 01 F7")
        );
        assert_eq!(
            set_param(1, 0, 0, 39),
            hex("F0 21 25 7F 4D 50 2D 64 12 10 01 00 02 00 00 00 00 27 F7")
        );
        assert_eq!(
            set_param(3, 65, 5, 56),
            hex("F0 21 25 7F 4D 50 2D 64 12 10 03 00 02 00 41 05 00 38 F7")
        );
        assert_eq!(
            set_algorithm(1, 3),
            hex("F0 21 25 7F 4D 50 2D 64 12 10 01 00 01 00 03 F7")
        );
        assert_eq!(select_preset(1, 84), [hex("B5 00 01"), hex("C5 54")]);
        assert_eq!(block_switch(4, false), hex("B5 34 00")); // CAB
        assert_eq!(
            save_preset(1, 84),
            hex("F0 21 25 7F 4D 50 2D 64 12 00 02 06 07 01 54 F7")
        );
        assert_eq!(
            preset_name(1, 84, "K NIU DS"),
            hex(
                "F0 21 25 7F 4D 50 2D 64 12 10 00 00 00 01 54 4B 20 4E 49 55 20 44 53 00 00 00 00 00 00 00 00 00 F7"
            )
        );
    }

    #[test]
    fn parses_hardware_notifications_captured_from_the_pedal() {
        use Notification::*;
        let n = |s: &str| parse_notification(&hex(s));
        // pedal de expressão no Classic PS
        assert_eq!(
            n("F0 21 25 7F 4D 50 2D 64 12 10 01 00 02 01 22 01 00 62 F7"),
            Some(Param {
                slot: 1,
                alg: 162,
                param: 1,
                value: 98
            })
        );
        // CTRL alternando o DLY
        assert_eq!(
            n("F0 21 25 7F 4D 50 2D 64 12 10 08 00 00 7F F7"),
            Some(Switch { slot: 8, on: true })
        );
        assert_eq!(
            n("F0 21 25 7F 4D 50 2D 64 12 10 08 00 00 00 F7"),
            Some(Switch { slot: 8, on: false })
        );
        assert_eq!(
            n("F0 21 25 7F 4D 50 2D 64 12 00 02 06 05 00 00 01 16 F7"),
            Some(Tempo(150))
        );
        assert_eq!(
            n("F0 21 25 7F 4D 50 2D 64 12 00 02 06 04 01 02 F7"),
            Some(PresetChanged)
        );
        assert_eq!(
            n("F0 21 25 7F 4D 50 2D 64 12 00 02 06 23 08 F7"),
            Some(CtrlState(0x08))
        );
        assert_eq!(
            n("F0 21 25 7F 4D 50 2D 64 12 00 02 06 24 00 01 F7"),
            Some(Other)
        );
        // respostas de pedidos e MIDI curto não são notificações
        assert_eq!(n("F0 21 25 7F 4D 50 2D 64 12 00 00 05 00 01 00 F7"), None);
        assert_eq!(parse_notification(&hex("B6 08 00")), None);
    }

    #[test]
    fn chain_order_matches_the_official_editor() {
        let header = [
            0xF0, 0x21, 0x25, 0x7F, 0x4D, 0x50, 0x2D, 0x64, 0x12, 0x00, 0x02, 0x06, 0x0D,
        ];
        let with = |tail: [u8; 9]| [&header[..], &tail[..], &[0xF7]].concat();
        // capturado em 2026-10-16 no P29-1: FX3 movido para antes do FX2, e depois a ordem original
        // (FX1 FX2 AMP CAB NR EQ FX3 DLY RVB = armazenamento 0 1 2 4 3 5 6 7 8)
        assert_eq!(
            chain_order(&[0, 6, 1, 2, 4, 3, 5, 7, 8]).unwrap(),
            with([0, 2, 3, 5, 4, 6, 1, 7, 8])
        );
        assert_eq!(
            chain_order(&[0, 1, 2, 4, 3, 5, 6, 7, 8]).unwrap(),
            with([0, 1, 2, 4, 3, 5, 6, 7, 8])
        );
        assert_eq!(chain_order(&[0, 1, 2]), None); // incompleta
        assert_eq!(chain_order(&[0, 0, 2, 4, 3, 5, 6, 7, 8]), None); // repetida
        assert_eq!(chain_order(&[0, 1, 2, 4, 3, 5, 6, 7, 9]), None); // fora da faixa
    }

    #[test]
    fn patch_volume_and_tempo_match_the_official_editor() {
        // capturado em 2026-10-16 no P29-1: volume 48 / 45 / 99 (máx.), tempo 125 / 150 / 120 / 250 / 40
        assert_eq!(patch_volume(48), [0xB5, 0x07, 0x30]);
        assert_eq!(patch_volume(45), [0xB5, 0x07, 0x2D]);
        assert_eq!(patch_volume(500), [0xB5, 0x07, 0x63]);
        assert_eq!(
            patch_tempo(125),
            [vec![0xB5, 0x49, 0x00], vec![0xB5, 0x4A, 0x7D]]
        );
        assert_eq!(
            patch_tempo(150),
            [vec![0xB5, 0x49, 0x01], vec![0xB5, 0x4A, 0x16]]
        );
        assert_eq!(
            patch_tempo(120),
            [vec![0xB5, 0x49, 0x00], vec![0xB5, 0x4A, 0x78]]
        );
        assert_eq!(
            patch_tempo(999),
            [vec![0xB5, 0x49, 0x01], vec![0xB5, 0x4A, 0x7A]]
        );
        assert_eq!(
            patch_tempo(1),
            [vec![0xB5, 0x49, 0x00], vec![0xB5, 0x4A, 0x28]]
        );
    }

    #[test]
    fn validates_name_frame_checksum() {
        let empty = hex(
            "F0 21 25 7F 4D 50 2D 64 12 00 02 05 00 01 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 0F 00 00 0C 01 F7",
        );
        assert!(frame_checksum_ok(&empty));
        let mut bad = empty.clone();
        bad[20] = 0x0E;
        assert!(!frame_checksum_ok(&bad));
    }

    #[test]
    fn nibbles_roundtrip() {
        let data = b"K NIU DS\x00\xFF";
        assert_eq!(from_nibbles(&to_nibbles(data)), data);
        assert_eq!(to_nibbles(&[0x4A]), vec![0x04, 0x0A]);
    }

    #[test]
    fn fourteen_bit_values() {
        assert_eq!(split14(162), [0x01, 0x22]);
        assert_eq!(join14(0x01, 0x54), 212);
    }
}
