//! Metadados de algoritmos/parâmetros (`assets/params.json`, gerado do algorithm.xml por tools/gen_params.ts).

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Knob,
    Slider,
    Combox,
    Switch,
    Texteditor,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ParamMeta {
    pub index: usize,
    pub name: String,
    pub kind: Kind,
    pub min: f64,
    pub max: f64,
    pub display_min: f64,
    pub display_max: f64,
    pub default: f64,
    pub options: Vec<String>,
}

impl ParamMeta {
    /// Valor bruto (o que vai no protocolo) → valor exibido na UI.
    pub fn display(&self, raw: u16) -> f64 {
        if self.max <= self.min {
            return raw as f64;
        }
        self.display_min
            + (raw as f64 - self.min) * (self.display_max - self.display_min)
                / (self.max - self.min)
    }

    pub fn range(&self) -> std::ops::RangeInclusive<u16> {
        (self.min.max(0.0) as u16)..=(self.max.max(0.0) as u16)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlgoMeta {
    pub name: String,
    pub params: Vec<ParamMeta>,
}

fn table() -> &'static HashMap<String, AlgoMeta> {
    static TABLE: OnceLock<HashMap<String, AlgoMeta>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t: HashMap<String, AlgoMeta> =
            serde_json::from_str(include_str!("../assets/params.json"))
                .expect("assets/params.json inválido");
        for algo in t.values_mut() {
            algo.params.dedup_by_key(|p| p.index);
        }
        t
    })
}

pub fn algorithm(module: &str, code: u16) -> Option<&'static AlgoMeta> {
    table().get(&format!("{module}:{code}"))
}

/// Algoritmos de um módulo, ordenados por código: `(código, nome)`.
pub fn algorithms_of(module: &str) -> Vec<(u16, &'static str)> {
    let prefix = format!("{module}:");
    let mut v: Vec<_> = table()
        .iter()
        .filter_map(|(k, a)| Some((k.strip_prefix(&prefix)?.parse().ok()?, a.name.as_str())))
        .collect();
    v.sort();
    v.dedup_by_key(|(c, _)| *c);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_matches_known_algorithms() {
        let ps = algorithm("FX1", 162).unwrap();
        assert_eq!(ps.name, "Classic PS");
        assert_eq!(ps.params[0].options[3], "+2 Oct");
        let rack = algorithm("DLY", 14).unwrap();
        assert_eq!(rack.params[2].name, "Time");
        assert_eq!(rack.params[2].display(636), 656.0); // ms
        let eq = algorithm("EQ", 0).unwrap();
        assert_eq!(eq.params[0].display(50), 0.0); // dB
        assert!(
            algorithms_of("AMP")
                .iter()
                .any(|&(c, n)| c == 65 && n == "Marshell 800")
        );
    }
}
