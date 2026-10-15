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

/// Mudança de parâmetro (o aparelho envia o mesmo formato de [`set_param`] quando algo muda no hardware).
/// Retorna `(slot, algoritmo, parâmetro, valor)`.
pub fn parse_param_change(msg: &[u8]) -> Option<(u8, u16, u8, u16)> {
    if !is_ampero_sysex(msg)
        || msg.len() != 19
        || msg[8] != OP_WRITE
        || msg[9] != 0x10
        || msg[11] != 0x00
        || msg[12] != 0x02
    {
        return None;
    }
    Some((
        msg[10],
        join14(msg[13], msg[14]),
        msg[15],
        join14(msg[16], msg[17]),
    ))
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
        // pedal de expressão no Classic PS (capturado do aparelho)
        assert_eq!(
            parse_param_change(&hex(
                "F0 21 25 7F 4D 50 2D 64 12 10 01 00 02 01 22 01 00 62 F7"
            )),
            Some((1, 162, 1, 98))
        );
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
