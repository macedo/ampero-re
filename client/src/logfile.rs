//! Leitura offline dos logs gerados pelo proxy `winmm.dll` (`proxy/`).
//! Formato da linha: `<ms> <OUT|IN > <handle> <len>: <hex...>`

use crate::preset::Preset;
use crate::protocol::{self as p, CHUNK_DATA_START};
use anyhow::Result;

/// Mensagens recebidas do aparelho (`IN`), na ordem do log.
pub fn incoming_frames(log: &str) -> Vec<Vec<u8>> {
    log.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            f.next()?;
            if f.next()? != "IN" {
                return None;
            }
            f.next()?;
            f.next()?;
            f.map(|h| u8::from_str_radix(h, 16).ok()).collect()
        })
        .collect()
}

/// Remonta os presets lidos na sync (chunks `0E` até o fim `10`).
pub fn presets_from_log(log: &str) -> Result<Vec<Preset>> {
    let mut presets = Vec::new();
    let mut nibs = Vec::new();
    for frame in incoming_frames(log) {
        match p::command(&frame) {
            Some([0x00, 0x02, 0x0E]) => {
                nibs.extend_from_slice(&frame[CHUNK_DATA_START..frame.len() - 1])
            }
            Some([0x00, 0x02, 0x10]) => {
                let index = p::join14(frame[13], frame[14]);
                match p::decode_chunked_payload(&nibs) {
                    Some(raw) => presets.push(Preset::decode(index, &raw)?),
                    None => eprintln!("checksum inválido no preset {index}"),
                }
                nibs.clear();
            }
            _ => {}
        }
    }
    Ok(presets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_saved_p29_1_from_real_capture() {
        let presets = presets_from_log(include_str!("../tests/fixtures/sync_excerpt.log")).unwrap();
        let pr = presets
            .iter()
            .find(|p| p.index == 212)
            .expect("P29-1 no trecho");
        assert_eq!(
            (pr.bank, pr.label.as_str(), pr.name.as_str()),
            (1, "P29-1", "K NIU DS")
        );
        assert_eq!((pr.volume, pr.tempo, pr.volume_pre), (45, 120, false));
        let fx1 = &pr.blocks[0];
        assert_eq!(
            (fx1.on, fx1.algorithm, fx1.algorithm_name.as_deref()),
            (true, 3, Some("Squeezer"))
        );
        assert_eq!(&fx1.params[..7], &[50; 7]);
        let amp = &pr.blocks[2];
        assert_eq!(amp.algorithm_name.as_deref(), Some("Marshell 800"));
        assert_eq!(&amp.params[..6], &[19, 50, 80, 76, 59, 56]);
        assert_eq!(pr.blocks[7].params[2], 814); // Delay Time em ms (u16)
        assert_eq!(
            pr.chain,
            ["FX1", "FX2", "AMP", "CAB", "NR", "EQ", "FX3", "DLY", "RVB"]
        );
    }
}
