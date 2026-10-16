//! Ferramentas de timbre usadas pelo servidor MCP: consultam só o que o Ampero tem (catálogo de
//! `params.json` + descrições da instalação oficial, quando disponíveis) e validam propostas de timbre
//! antes de qualquer coisa ir para a pedaleira.

use crate::params::{self, AlgoMeta, Kind, ParamMeta};
use crate::preset::{MODULES, Preset};
use crate::resources::Resources;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

// ---------------------------------------------------------------- propostas

/// Timbre pedido pelo cliente MCP (valores na unidade exibida).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Proposal {
    pub title: String,
    #[serde(default)]
    pub explanation: String,
    pub blocks: Vec<ProposedBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposedBlock {
    pub module: String,
    pub algorithm: String,
    #[serde(default = "yes")]
    pub on: bool,
    /// nome do parâmetro → número (unidade exibida), nome da opção (combo) ou booleano (chave)
    #[serde(default)]
    pub params: Map<String, Value>,
}

fn yes() -> bool {
    true
}

/// Bloco da proposta já convertido para o protocolo.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedBlock {
    /// índice de armazenamento (0 = FX1 … 8 = RVB)
    pub block: usize,
    pub algorithm: u16,
    pub on: bool,
    /// (índice do parâmetro, valor bruto)
    pub params: Vec<(u8, u16)>,
}

fn module_index(name: &str) -> Option<usize> {
    MODULES
        .iter()
        .position(|m| m.eq_ignore_ascii_case(name.trim()))
}

fn find_param<'a>(algo: &'a AlgoMeta, name: &str) -> Option<&'a ParamMeta> {
    let norm = |s: &str| s.to_lowercase().split_whitespace().collect::<String>();
    algo.params.iter().find(|p| norm(&p.name) == norm(name))
}

fn raw_value(pm: &ParamMeta, v: &Value) -> Result<u16, String> {
    let fail = |why: &str| Err(format!("{}: {why}", pm.name));
    match pm.kind {
        Kind::Switch => match v {
            Value::Bool(b) => Ok(*b as u16),
            Value::Number(n) => Ok((n.as_f64().unwrap_or(0.0) != 0.0) as u16),
            Value::String(s) => match s.to_lowercase().as_str() {
                "on" | "ligado" | "true" | "1" => Ok(1),
                "off" | "desligado" | "false" | "0" => Ok(0),
                _ => fail("use true/false"),
            },
            _ => fail("use true/false"),
        },
        Kind::Combox => match v {
            Value::String(s) => pm.option_index(s).map_or_else(
                || {
                    fail(&format!(
                        "opção inválida; use uma de: {}",
                        pm.options.join(", ")
                    ))
                },
                Ok,
            ),
            Value::Number(n) => {
                let i = n.as_u64().unwrap_or(u64::MAX);
                if (i as usize) < pm.options.len() {
                    Ok(i as u16)
                } else {
                    fail(&format!(
                        "opção inválida; use uma de: {}",
                        pm.options.join(", ")
                    ))
                }
            }
            _ => fail("use o nome da opção"),
        },
        _ => {
            let n = match v {
                Value::Number(n) => n.as_f64(),
                Value::String(s) => s.trim().parse().ok(),
                _ => None,
            };
            let Some(n) = n else {
                return fail("use um número");
            };
            pm.raw_from_display(n).map_or_else(
                || {
                    fail(&format!(
                        "{n} fora da faixa {}..{}",
                        pm.display_min, pm.display_max
                    ))
                },
                Ok,
            )
        }
    }
}

/// Valida e converte a proposta. Em caso de erro, devolve a lista de problemas (para o agente corrigir).
pub fn resolve(p: &Proposal) -> Result<Vec<ResolvedBlock>, Vec<String>> {
    let mut errors = Vec::new();
    let mut out = Vec::new();
    if p.blocks.is_empty() {
        errors.push("a proposta não tem blocos".to_string());
    }
    for b in &p.blocks {
        let Some(block) = module_index(&b.module) else {
            errors.push(format!(
                "bloco \"{}\" não existe; use um de: {}",
                b.module,
                MODULES.join(", ")
            ));
            continue;
        };
        let module = MODULES[block];
        let Some((code, algo)) = params::find_by_name(module, &b.algorithm) else {
            errors.push(format!(
                "{module}: algoritmo \"{}\" não existe (use listar_algoritmos)",
                b.algorithm
            ));
            continue;
        };
        let mut resolved = Vec::new();
        for (name, v) in &b.params {
            match find_param(algo, name) {
                None => errors.push(format!(
                    "{module}/{}: parâmetro \"{name}\" não existe; os parâmetros são: {}",
                    algo.name,
                    algo.params
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Some(pm) => match raw_value(pm, v) {
                    Ok(raw) => resolved.push((pm.index as u8, raw)),
                    Err(e) => errors.push(format!("{module}/{}: {e}", algo.name)),
                },
            }
        }
        out.push(ResolvedBlock {
            block,
            algorithm: code,
            on: b.on,
            params: resolved,
        });
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

// ---------------------------------------------------------------- ferramentas do agente

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
}

pub fn tool_specs() -> Vec<ToolSpec> {
    let module = json!({"type": "string", "enum": MODULES, "description": "Bloco da cadeia"});
    vec![
        ToolSpec {
            name: "listar_algoritmos",
            description: "Lista os algoritmos disponíveis num bloco, com o equipamento real em que cada um se \
                          baseia. Use antes de escolher um algoritmo para um bloco.",
            schema: json!({"type": "object", "properties": {"module": module}, "required": ["module"]}),
        },
        ToolSpec {
            name: "detalhar_algoritmo",
            description: "Mostra os parâmetros de um algoritmo (faixas, unidades, opções, padrões) e a descrição \
                          completa. Use antes de propor valores para ele.",
            schema: json!({
                "type": "object",
                "properties": {"module": module, "name": {"type": "string", "description": "Nome exato do algoritmo"}},
                "required": ["module", "name"]
            }),
        },
        ToolSpec {
            name: "preset_atual",
            description: "Mostra o preset carregado agora na pedaleira (buffer de edição): blocos, algoritmos, \
                          liga/desliga e valores. Use quando o pedido for ajustar o timbre atual.",
            schema: json!({"type": "object", "properties": {}}),
        },
        ToolSpec {
            name: "aplicar_timbre",
            description: "Aplica um timbre no buffer de edição da pedaleira (não salva). Inclua só os blocos que \
                          devem mudar; blocos omitidos ficam como estão. Valores na unidade exibida (ex.: 450 para \
                          450 ms), opções de combo pelo nome e chaves como true/false. Se houver erros, nada é \
                          aplicado e a ferramenta lista o que corrigir.",
            schema: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string", "description": "Nome curto do timbre (ex.: \"Comfortably Numb - solo\")"},
                    "explanation": {"type": "string", "description": "Resumo das escolhas em 1-3 frases"},
                    "blocks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "module": module,
                                "algorithm": {"type": "string"},
                                "on": {"type": "boolean"},
                                "params": {"type": "object", "description": "nome do parâmetro → valor", "additionalProperties": true}
                            },
                            "required": ["module", "algorithm", "on", "params"]
                        }
                    }
                },
                "required": ["title", "explanation", "blocks"]
            }),
        },
    ]
}

/// O que as ferramentas podem consultar.
pub struct ToolContext<'a> {
    pub preset: Option<&'a Preset>,
    pub resources: Option<&'a Resources>,
}

pub enum ToolOutcome {
    Text(String),
    /// `aplicar_timbre` válido: quem chamou aplica os blocos no buffer de edição.
    Apply(Proposal, Vec<ResolvedBlock>),
    Error(String),
}

fn first_sentence(text: &str, max: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let end = flat.find(". ").map_or(flat.len(), |i| i + 1);
    let mut s: String = flat[..end].chars().take(max).collect();
    if s.len() < flat[..end].len() {
        s.push('…');
    }
    s
}

fn describe_param(pm: &ParamMeta) -> String {
    let default_raw = pm.default.max(0.0) as u16;
    match pm.kind {
        Kind::Switch => format!(
            "- {} (chave on/off), padrão {}",
            pm.name,
            if pm.default != 0.0 { "on" } else { "off" }
        ),
        Kind::Combox => format!(
            "- {} (opções: {}), padrão {}",
            pm.name,
            pm.options.join(", "),
            pm.options
                .get(default_raw as usize)
                .map_or("?", String::as_str)
        ),
        _ => format!(
            "- {}: {}..{}, padrão {}",
            pm.name,
            trim_num(pm.display_min),
            trim_num(pm.display_max),
            trim_num(pm.display(default_raw))
        ),
    }
}

fn trim_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-6 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

fn shown_value(pm: &ParamMeta, raw: u16) -> String {
    match pm.kind {
        Kind::Switch => if raw != 0 { "on" } else { "off" }.into(),
        Kind::Combox => pm
            .options
            .get(raw as usize)
            .cloned()
            .unwrap_or_else(|| raw.to_string()),
        _ => trim_num(pm.display(raw)),
    }
}

pub fn run_tool(name: &str, input: &Value, ctx: &ToolContext) -> ToolOutcome {
    let module = || -> Result<&'static str, String> {
        let m = input
            .get("module")
            .and_then(Value::as_str)
            .unwrap_or_default();
        module_index(m).map(|i| MODULES[i]).ok_or_else(|| {
            format!(
                "bloco \"{m}\" não existe; use um de: {}",
                MODULES.join(", ")
            )
        })
    };
    match name {
        "listar_algoritmos" => match module() {
            Err(e) => ToolOutcome::Error(e),
            Ok(m) => {
                let lines: Vec<String> = params::algorithms_of(m)
                    .into_iter()
                    .map(
                        |(_, n)| match ctx.resources.and_then(|r| r.description(m, n)) {
                            Some(d) => format!("- {n}: {}", first_sentence(d, 140)),
                            None => format!("- {n}"),
                        },
                    )
                    .collect();
                ToolOutcome::Text(format!("Algoritmos do bloco {m}:\n{}", lines.join("\n")))
            }
        },
        "detalhar_algoritmo" => {
            let m = match module() {
                Ok(m) => m,
                Err(e) => return ToolOutcome::Error(e),
            };
            let n = input
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            match params::find_by_name(m, n) {
                None => ToolOutcome::Error(format!(
                    "{m}: algoritmo \"{n}\" não existe (use listar_algoritmos)"
                )),
                Some((_, algo)) => {
                    let mut s = format!("{m} / {}\nParâmetros:\n", algo.name);
                    for pm in &algo.params {
                        s += &describe_param(pm);
                        s.push('\n');
                    }
                    if let Some(d) = ctx.resources.and_then(|r| r.description(m, &algo.name)) {
                        s += "\nDescrição:\n";
                        s += d;
                    }
                    ToolOutcome::Text(s)
                }
            }
        }
        "preset_atual" => match ctx.preset {
            None => ToolOutcome::Text(
                "Nenhum preset carregado (a pedaleira não está conectada).".into(),
            ),
            Some(p) => {
                let mut s = format!(
                    "{} \"{}\" — volume {}, tempo {} BPM\nOrdem da cadeia: {}\n",
                    p.label,
                    p.name,
                    p.volume,
                    p.tempo,
                    p.chain.join(" > ")
                );
                for b in &p.blocks {
                    let meta = params::algorithm(b.module, b.algorithm);
                    let name = b.algorithm_name.as_deref().unwrap_or("?");
                    let values = meta.map_or_else(String::new, |a| {
                        a.params
                            .iter()
                            .filter_map(|pm| {
                                Some(format!(
                                    "{}={}",
                                    pm.name,
                                    shown_value(pm, *b.params.get(pm.index)?)
                                ))
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    });
                    s += &format!(
                        "- {} [{}] {name}: {values}\n",
                        b.module,
                        if b.on { "on" } else { "off" }
                    );
                }
                ToolOutcome::Text(s)
            }
        },
        "aplicar_timbre" => match serde_json::from_value::<Proposal>(input.clone()) {
            Err(e) => ToolOutcome::Error(format!("formato inválido: {e}")),
            Ok(p) => match resolve(&p) {
                Ok(blocks) => ToolOutcome::Apply(p, blocks),
                Err(errs) => ToolOutcome::Error(format!(
                    "Nada foi aplicado. Corrija:\n- {}",
                    errs.join("\n- ")
                )),
            },
        },
        other => ToolOutcome::Error(format!("ferramenta desconhecida: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ToolContext<'static> {
        ToolContext {
            preset: None,
            resources: None,
        }
    }

    fn proposal_json() -> Value {
        json!({
            "title": "Teste",
            "explanation": "drive britânico com delay",
            "blocks": [
                {"module": "AMP", "algorithm": "marshell 800", "on": true, "params": {"Gain": 60, "treble": 55}},
                {"module": "DLY", "algorithm": "Vintage Rack", "on": true, "params": {"Time": 450, "Sync": false}},
                {"module": "FX1", "algorithm": "Classic PS", "on": false, "params": {"Range": "+2 Oct"}}
            ]
        })
    }

    #[test]
    fn resolves_a_valid_proposal_to_protocol_values() {
        let p: Proposal = serde_json::from_value(proposal_json()).unwrap();
        let r = resolve(&p).expect("proposta válida");
        assert_eq!(r[0].block, 2); // AMP
        assert_eq!(r[0].algorithm, 65); // Marshell 800
        assert!(r[0].params.contains(&(0, 60)) && r[0].params.contains(&(5, 55)));
        assert_eq!(r[1].block, 7); // DLY
        assert!(r[1].params.contains(&(2, 430))); // 450 ms → bruto 430 (faixa exibida começa em 20)
        assert!(r[1].params.contains(&(5, 0))); // Sync off
        assert_eq!((r[2].block, r[2].algorithm, r[2].on), (0, 162, false));
        assert_eq!(r[2].params, vec![(0, 3)]); // "+2 Oct" → opção 3
    }

    #[test]
    fn reports_every_problem_so_the_agent_can_fix_it() {
        let p: Proposal = serde_json::from_value(json!({
            "title": "x", "blocks": [
                {"module": "AMP", "algorithm": "Marshall 800", "on": true, "params": {}},
                {"module": "DLY", "algorithm": "Vintage Rack", "on": true, "params": {"Time": 9000, "Wobble": 3}},
                {"module": "PEDAL", "algorithm": "x", "on": true, "params": {}}
            ]
        }))
        .unwrap();
        let errs = resolve(&p).unwrap_err();
        assert_eq!(errs.len(), 4, "{errs:?}");
        assert!(
            errs.iter()
                .any(|e| e.contains("Marshall 800") && e.contains("não existe"))
        );
        assert!(
            errs.iter()
                .any(|e| e.contains("9000") && e.contains("fora da faixa"))
        );
        assert!(errs.iter().any(|e| e.contains("Wobble")));
        assert!(errs.iter().any(|e| e.contains("PEDAL")));
    }

    #[test]
    fn catalog_tools_answer_from_params_json() {
        let ToolOutcome::Text(list) =
            run_tool("listar_algoritmos", &json!({"module": "AMP"}), &ctx())
        else {
            panic!("esperava texto")
        };
        assert!(list.contains("Marshell 800"));
        let ToolOutcome::Text(detail) = run_tool(
            "detalhar_algoritmo",
            &json!({"module": "DLY", "name": "vintage rack"}),
            &ctx(),
        ) else {
            panic!("esperava texto")
        };
        assert!(detail.contains("- Time: 20..4000"), "{detail}");
        assert!(detail.contains("Sync (chave on/off)"));
        assert!(matches!(
            run_tool("listar_algoritmos", &json!({"module": "XYZ"}), &ctx()),
            ToolOutcome::Error(_)
        ));
        assert!(matches!(
            run_tool("aplicar_timbre", &proposal_json(), &ctx()),
            ToolOutcome::Apply(_, blocks) if blocks.len() == 3
        ));
        let bad = json!({"title": "t", "explanation": "e", "blocks": [
            {"module": "AMP", "algorithm": "Marshall 800", "on": true, "params": {}}
        ]});
        let ToolOutcome::Error(err) = run_tool("aplicar_timbre", &bad, &ctx()) else {
            panic!("esperava erro")
        };
        assert!(
            err.contains("Nada foi aplicado") && err.contains("Marshall 800"),
            "{err}"
        );
    }
}
