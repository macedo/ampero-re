//! Configurações globais do Ampero (resposta de `11 00 02 09 00`, nibbles a partir do byte 13).
//! Por enquanto só a função dos footswitches 1–4, que diz quais CTRL de um preset fazem efeito: o CTRL n só
//! liga e desliga blocos se o footswitch n estiver como CTRL/TAP. Ver PROTOCOLO.md, "Estado global".

/// Tap e Hold de cada footswitch 1–4: pares (tap, hold) a partir deste byte.
const FOOTSWITCH_AT: usize = 0x0F;
/// Valor de "CTRL/TAP" no Tap de um footswitch (último item da lista do editor oficial).
pub const FS_CTRL_TAP: u8 = 20;

/// Funções possíveis no Tap de um footswitch, na ordem da lista do editor oficial (o valor é o índice).
pub const FS_TAP_NAMES: [&str; 21] = [
    "None",
    "Patch 1",
    "Patch 2",
    "Patch 3",
    "Patch -",
    "Patch +",
    "Bank -",
    "Bank +",
    "Tuner",
    "Looper",
    "Drum",
    "FX1 On/Off",
    "FX2 On/Off",
    "AMP On/Off",
    "NR On/Off",
    "CAB On/Off",
    "EQ On/Off",
    "FX3 On/Off",
    "DLY On/Off",
    "RVB On/Off",
    "CTRL/TAP",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalSettings {
    /// Função do Tap dos footswitches 1–4 (índice em [`FS_TAP_NAMES`]).
    pub fs_tap: [u8; 4],
    pub fs_hold: [u8; 4],
}

impl GlobalSettings {
    /// A partir dos dados já tirados dos nibbles. `None` se vierem curtos demais.
    pub fn decode(data: &[u8]) -> Option<GlobalSettings> {
        let fs = data.get(FOOTSWITCH_AT..FOOTSWITCH_AT + 8)?;
        Some(GlobalSettings {
            fs_tap: std::array::from_fn(|i| fs[2 * i]),
            fs_hold: std::array::from_fn(|i| fs[2 * i + 1]),
        })
    }

    /// Máscara dos CTRL que fazem efeito (bit n = CTRL n+1): os footswitches com Tap = CTRL/TAP.
    pub fn active_ctrls(&self) -> u8 {
        (0..4)
            .filter(|&i| self.fs_tap[i] == FS_CTRL_TAP)
            .fold(0, |m, i| m | 1 << i)
    }

    /// Nome da função do Tap do footswitch `n` (1–4).
    pub fn fs_tap_name(&self, n: usize) -> &'static str {
        let v = self.fs_tap[n.clamp(1, 4) - 1] as usize;
        FS_TAP_NAMES.get(v).copied().unwrap_or("?")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_owners_footswitches() {
        // estado global lido do aparelho do dono em 2026-10-16: FS1–3 = Patch 1–3, FS4 = CTRL/TAP
        let data: Vec<u8> = [
            0x05, 0x01, 0x00, 0x5d, 0x63, 0x00, 0x05, 0x02, 0x00, 0x14, 0x00, 0x00, 0x00, 0x00,
            0x65, 0x01, 0x00, 0x02, 0x00, 0x03, 0x08, 0x14, 0x14, 0x06, 0x07, 0x09, 0x00, 0x01,
            0x32, 0x06, 0x00, 0x00,
        ]
        .to_vec();
        let g = GlobalSettings::decode(&data).unwrap();
        assert_eq!(g.fs_tap, [1, 2, 3, 20]);
        assert_eq!(g.fs_hold, [0, 0, 8, 20]); // FS3 segura = Tuner, FS4 segura = Tap Tempo
        assert_eq!(g.active_ctrls(), 0b1000); // só o CTRL 4 faz efeito
        assert_eq!(g.fs_tap_name(3), "Patch 3");
        assert_eq!(g.fs_tap_name(4), "CTRL/TAP");
        assert!(GlobalSettings::decode(&data[..10]).is_none());
    }
}
