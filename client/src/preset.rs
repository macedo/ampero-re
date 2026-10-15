//! Estrutura dos 459 bytes de um preset (ver PROTOCOLO.md, "Estrutura do preset").

use serde::Serialize;

pub const PRESET_LEN: usize = 459;
pub const MODULES: [&str; 9] = ["FX1", "FX2", "AMP", "NR", "CAB", "EQ", "FX3", "DLY", "RVB"];

const BLOCKS_AT: usize = 0x05F;
const BLOCK_LEN: usize = 33;
const PARAMS_PER_BLOCK: usize = 15;
const EXP_AT: usize = 0x027;
const EXP_ENTRIES: usize = 8;
const TEMPO_AT: usize = 0x18A;
const QUICK_KNOBS_AT: usize = 0x18C;
const CHAIN_AT: usize = 0x19A;
const VOL_POSITION_AT: usize = 0x1AC;

/// Primeiro código de CAB que aponta para um slot de IR do usuário (20 slots).
pub const USER_IR_BASE: u16 = 144;

#[derive(Debug, Clone, Serialize)]
pub struct Block {
    pub module: &'static str,
    pub on: bool,
    pub algorithm: u16,
    pub algorithm_name: Option<String>,
    pub params: Vec<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Target {
    pub module: Option<&'static str>,
    pub algorithm: u16,
    pub param: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExpAssign {
    pub target: Target,
    pub min: u8,
    pub max: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct Preset {
    pub index: u16,
    pub bank: u8,
    pub slot: u8,
    pub label: String,
    pub name: String,
    pub volume: u16,
    pub tempo: u16,
    pub volume_pre: bool,
    pub blocks: Vec<Block>,
    pub chain: Vec<&'static str>,
    pub quick_knobs: Vec<Target>,
    pub exp: Vec<ExpAssign>,
    #[serde(skip)]
    pub raw: Vec<u8>,
}

pub fn algorithm_name(module: &str, code: u16) -> Option<String> {
    if module == "CAB" && (USER_IR_BASE..USER_IR_BASE + 20).contains(&code) {
        return Some(format!("User IR {}", code - USER_IR_BASE + 1));
    }
    crate::params::algorithm(module, code).map(|a| a.name.clone())
}

/// Rótulo da UI ("P29-1") a partir do slot dentro do banco.
pub fn slot_label(slot: u8) -> String {
    format!("P{:02}-{}", slot / 3 + 1, slot % 3 + 1)
}

/// Converte "P29-1" em slot (0..98).
pub fn parse_label(s: &str) -> Option<u8> {
    let s = s.trim().trim_start_matches(['P', 'p']);
    let (n, sub) = s.split_once('-')?;
    let (n, sub): (u8, u8) = (n.parse().ok()?, sub.parse().ok()?);
    if !(1..=33).contains(&n) || !(1..=3).contains(&sub) {
        return None;
    }
    Some((n - 1) * 3 + (sub - 1))
}

fn module_at(storage_index_plus_one: u8) -> Option<&'static str> {
    MODULES
        .get((storage_index_plus_one as usize).checked_sub(1)?)
        .copied()
}

impl Preset {
    pub fn decode(index: u16, raw: &[u8]) -> anyhow::Result<Preset> {
        anyhow::ensure!(
            raw.len() == PRESET_LEN,
            "preset com {} bytes (esperado {PRESET_LEN})",
            raw.len()
        );
        let u16be = |o: usize| u16::from_be_bytes([raw[o], raw[o + 1]]);
        let code = |o: usize| crate::protocol::join14(raw[o], raw[o + 1]);

        let blocks = MODULES
            .iter()
            .enumerate()
            .map(|(k, &module)| {
                let o = BLOCKS_AT + BLOCK_LEN * k;
                let algorithm = code(o + 1);
                Block {
                    module,
                    on: raw[o] != 0,
                    algorithm,
                    algorithm_name: algorithm_name(module, algorithm),
                    params: (0..PARAMS_PER_BLOCK)
                        .map(|p| u16be(o + 3 + 2 * p))
                        .collect(),
                }
            })
            .collect();

        let quick_knobs = (0..3)
            .map(|q| {
                let o = QUICK_KNOBS_AT + 4 * q;
                // módulo = índice de armazenamento + 3 (FX1 = 3); valores menores são alvos de nível de patch
                Target {
                    module: raw[o].checked_sub(2).and_then(module_at),
                    algorithm: code(o + 1),
                    param: raw[o + 3],
                }
            })
            .collect();

        let exp = (0..EXP_ENTRIES)
            .map(|r| EXP_AT + 7 * r)
            .filter(|&o| raw[o] != 0)
            .map(|o| ExpAssign {
                target: Target {
                    module: module_at(raw[o]),
                    algorithm: code(o + 1),
                    param: raw[o + 3],
                },
                min: raw[o + 4],
                max: raw[o + 5],
            })
            .collect();

        let name_bytes: Vec<u8> = raw[4..20].iter().copied().take_while(|&b| b != 0).collect();

        Ok(Preset {
            index,
            bank: raw[0],
            slot: raw[1],
            label: slot_label(raw[1]),
            name: String::from_utf8_lossy(&name_bytes).trim_end().to_string(),
            volume: u16be(2),
            tempo: u16be(TEMPO_AT),
            volume_pre: raw[VOL_POSITION_AT] != 0,
            blocks,
            chain: (0..9)
                .filter_map(|k| MODULES.get(raw[CHAIN_AT + 2 * k] as usize).copied())
                .collect(),
            quick_knobs,
            exp,
            raw: raw.to_vec(),
        })
    }

    pub fn summary(&self) -> String {
        let mut s = format!(
            "[{:3}] banco {} {} \"{}\"  vol={} tempo={} {}\n",
            self.index,
            self.bank,
            self.label,
            self.name,
            self.volume,
            self.tempo,
            if self.volume_pre { "Pre" } else { "Post" }
        );
        for b in &self.blocks {
            let used = b.params.iter().rposition(|&v| v != 0).map_or(0, |i| i + 1);
            s += &format!(
                "    {:<3} {} {:<22} [{:3}] {:?}\n",
                b.module,
                if b.on { "ON " } else { "off" },
                b.algorithm_name.as_deref().unwrap_or("?"),
                b.algorithm,
                &b.params[..used]
            );
        }
        s += &format!("    cadeia: {}\n", self.chain.join(" > "));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(parse_label("P29-1"), Some(84));
        assert_eq!(slot_label(84), "P29-1");
        assert_eq!(parse_label("P34-1"), None);
        assert_eq!(algorithm_name("AMP", 65).as_deref(), Some("Marshell 800"));
        assert_eq!(algorithm_name("CAB", 152).as_deref(), Some("User IR 9"));
    }
}
