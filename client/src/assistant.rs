//! Assistente de timbre: um agente de IA que conversa com o usuário e monta presets usando só o que o
//! Ampero tem (catálogo de `params.json` + descrições da instalação oficial, quando disponíveis).
//!
//! Provedores: Claude (API Messages da Anthropic) e qualquer serviço no formato OpenAI `chat/completions`
//! (ChatGPT via API da OpenAI, Ollama, LM Studio, OpenRouter…). O agente não mexe no aparelho: ele
//! devolve propostas validadas que a interface mostra com um botão "Aplicar".

use crate::params::{self, AlgoMeta, Kind, ParamMeta};
use crate::preset::{MODULES, Preset};
use crate::resources::Resources;
use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

// ---------------------------------------------------------------- configuração

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// API Messages da Anthropic.
    Claude,
    /// API da OpenAI (formato chat/completions).
    #[serde(rename = "chatgpt")]
    ChatGpt,
    /// Qualquer servidor compatível com chat/completions (Ollama, LM Studio, OpenRouter…).
    Compatible,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Claude, Provider::ChatGpt, Provider::Compatible];

    pub fn label(self) -> &'static str {
        match self {
            Provider::Claude => "Claude (Anthropic)",
            Provider::ChatGpt => "ChatGPT (OpenAI)",
            Provider::Compatible => "Outro compatível (Ollama, LM Studio, OpenRouter…)",
        }
    }

    /// Variável de ambiente usada quando nenhuma chave foi salva na configuração.
    pub fn key_env(self) -> Option<&'static str> {
        match self {
            Provider::Claude => Some("ANTHROPIC_API_KEY"),
            Provider::ChatGpt => Some("OPENAI_API_KEY"),
            Provider::Compatible => None,
        }
    }

    /// Modelos oferecidos no seletor antes de consultar a API (a lista real vem de [`list_models`]).
    pub fn suggested_models(self) -> &'static [&'static str] {
        match self {
            Provider::Claude => &["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"],
            Provider::ChatGpt => &["gpt-5", "gpt-5-mini"],
            Provider::Compatible => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    pub base_url: String,
    pub model: String,
    /// Chave salva na configuração (texto puro no arquivo do usuário). Vazia = usar a variável de ambiente.
    #[serde(default)]
    pub api_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub provider: Provider,
    pub claude: Endpoint,
    pub chatgpt: Endpoint,
    pub compatible: Endpoint,
    /// Prompt de sistema (editável na interface).
    pub system_prompt: String,
    /// Claude: `output_config.effort` (`low`…`max`); vazio = padrão do modelo.
    #[serde(default = "default_effort")]
    pub claude_effort: String,
    /// Claude: se o modelo recusar por classificador de segurança, o servidor refaz a resposta em outro
    /// modelo (`fallbacks: "default"`). Só vale na API oficial da Anthropic.
    #[serde(default = "yes")]
    pub claude_server_fallback: bool,
}

fn default_effort() -> String {
    "medium".into()
}
fn yes() -> bool {
    true
}

pub const DEFAULT_SYSTEM_PROMPT: &str = "\
Você é um especialista em timbres de guitarra e baixo e ajuda o usuário a montar presets na pedaleira \
Hotone Ampero. Responda em português do Brasil, de forma direta e prática.

A cadeia do Ampero tem blocos fixos, cada um com um algoritmo: FX1, FX2, AMP, NR, CAB, EQ, FX3, DLY, RVB. \
Os nomes dos algoritmos são inspirados em equipamentos reais (ex.: \"Marshell 800\" = Marshall JCM800); \
use as descrições do catálogo para fazer essa ponte.

Como trabalhar:
1. Entenda o pedido: música, banda, trecho (base, solo, limpo), e, se fizer diferença, a guitarra e os captadores \
do usuário. Pergunte só o que for essencial.
2. Consulte o catálogo com listar_algoritmos e detalhar_algoritmo. Use somente algoritmos e parâmetros que \
existem; se não houver um equivalente exato, escolha o mais próximo e diga isso.
3. Veja o preset atual com preset_atual quando ajustar algo que o usuário já tem.
4. Proponha o timbre com propor_timbre (valores na unidade exibida, ex.: 450 para 450 ms). Se a ferramenta \
apontar erros, corrija e proponha de novo. Combine o CAB com o estilo do AMP.
5. Explique em poucas frases o porquê das escolhas e dê dicas de execução (volume da guitarra, captador, palhetada).

A proposta só vai para a pedaleira quando o usuário clicar em Aplicar, e nada é salvo automaticamente.";

impl Default for Config {
    fn default() -> Self {
        Config {
            provider: Provider::Claude,
            claude: Endpoint {
                base_url: "https://api.anthropic.com".into(),
                model: "claude-opus-5-5".into(),
                api_key: String::new(),
            },
            chatgpt: Endpoint {
                base_url: "https://api.openai.com/v1".into(),
                model: "gpt-5".into(),
                api_key: String::new(),
            },
            compatible: Endpoint {
                base_url: "http://localhost:11434/v1".into(),
                model: "llama3.1".into(),
                api_key: String::new(),
            },
            system_prompt: DEFAULT_SYSTEM_PROMPT.into(),
            claude_effort: default_effort(),
            claude_server_fallback: true,
        }
    }
}

impl Config {
    /// `%APPDATA%\ampero\assistant.json`, `~/Library/Application Support/ampero/…` ou `~/.config/ampero/…`.
    pub fn path() -> Option<PathBuf> {
        let base = if cfg!(windows) {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else if cfg!(target_os = "macos") {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        }?;
        Some(base.join("ampero").join("assistant.json"))
    }

    pub fn load() -> Config {
        Config::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Config::path().context("pasta de configuração não encontrada")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn endpoint(&self) -> &Endpoint {
        match self.provider {
            Provider::Claude => &self.claude,
            Provider::ChatGpt => &self.chatgpt,
            Provider::Compatible => &self.compatible,
        }
    }

    pub fn endpoint_mut(&mut self) -> &mut Endpoint {
        match self.provider {
            Provider::Claude => &mut self.claude,
            Provider::ChatGpt => &mut self.chatgpt,
            Provider::Compatible => &mut self.compatible,
        }
    }

    /// Chave salva, ou a da variável de ambiente do provedor.
    pub fn api_key(&self) -> String {
        let saved = self.endpoint().api_key.trim();
        if !saved.is_empty() {
            return saved.to_string();
        }
        self.provider
            .key_env()
            .and_then(|v| std::env::var(v).ok())
            .unwrap_or_default()
    }

    /// Cabeçalhos de autenticação do provedor atual.
    fn auth_headers(&self) -> Vec<(String, String)> {
        let key = self.api_key();
        match self.provider {
            Provider::Claude => vec![
                ("x-api-key".to_string(), key),
                ("anthropic-version".to_string(), "2023-06-01".to_string()),
            ],
            _ if key.is_empty() => Vec::new(),
            _ => vec![("Authorization".to_string(), format!("Bearer {key}"))],
        }
    }
}

/// Modelos que a chave configurada pode usar, consultados na API do provedor (`GET /v1/models` na
/// Anthropic, `GET /models` no formato OpenAI). Na OpenAI oficial, tira os que não conversam
/// (embeddings, áudio, imagem…).
pub fn list_models(cfg: &Config, transport: &dyn Transport) -> Result<Vec<String>> {
    if cfg.api_key().is_empty() && cfg.provider != Provider::Compatible {
        bail!(
            "sem chave da API: informe-a acima ou defina {}",
            cfg.provider.key_env().unwrap_or("")
        );
    }
    let base = cfg.endpoint().base_url.trim_end_matches('/');
    let url = match cfg.provider {
        Provider::Claude => format!("{base}/v1/models?limit=100"),
        _ => format!("{base}/models"),
    };
    let (status, resp) = transport.get(&url, &cfg.auth_headers())?;
    if status != 200 {
        return Err(api_error(status, &resp));
    }
    let mut ids: Vec<String> = resp
        .get("data")
        .and_then(Value::as_array)
        .context("resposta sem a lista \"data\"")?
        .iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    if cfg.provider == Provider::ChatGpt {
        const NOT_CHAT: [&str; 9] = [
            "embedding",
            "tts",
            "whisper",
            "dall-e",
            "image",
            "moderation",
            "audio",
            "realtime",
            "transcribe",
        ];
        ids.retain(|id| !NOT_CHAT.iter().any(|n| id.contains(n)));
    }
    if cfg.provider != Provider::Claude {
        ids.sort(); // a Anthropic já devolve do mais novo para o mais antigo
    }
    Ok(ids)
}

// ---------------------------------------------------------------- propostas

/// Proposta de timbre feita pelo agente (valores na unidade exibida).
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
            name: "propor_timbre",
            description: "Propõe um timbre ao usuário. Inclua só os blocos que devem mudar; blocos omitidos ficam \
                          como estão. Os valores vão na unidade exibida (ex.: 450 para 450 ms), opções de combo \
                          pelo nome e chaves como true/false. Se houver erros, a ferramenta os lista para você \
                          corrigir; o usuário decide se aplica.",
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

/// O que o agente pode consultar ao executar ferramentas.
pub struct ToolContext<'a> {
    pub preset: Option<&'a Preset>,
    pub resources: Option<&'a Resources>,
}

pub enum ToolOutcome {
    Text(String),
    Proposal(Proposal, String),
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
                    .map(|(_, n)| match ctx.resources.and_then(|r| r.description(m, n)) {
                        Some(d) => format!("- {n}: {}", first_sentence(d, 140)),
                        None => format!("- {n}"),
                    })
                    .collect();
                ToolOutcome::Text(format!("Algoritmos do bloco {m}:\n{}", lines.join("\n")))
            }
        },
        "detalhar_algoritmo" => {
            let m = match module() {
                Ok(m) => m,
                Err(e) => return ToolOutcome::Error(e),
            };
            let n = input.get("name").and_then(Value::as_str).unwrap_or_default();
            match params::find_by_name(m, n) {
                None => ToolOutcome::Error(format!("{m}: algoritmo \"{n}\" não existe (use listar_algoritmos)")),
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
            None => ToolOutcome::Text("Nenhum preset carregado (a pedaleira não está conectada).".into()),
            Some(p) => {
                let mut s = format!("{} \"{}\" — volume {}, tempo {} BPM\nOrdem da cadeia: {}\n", p.label, p.name, p.volume, p.tempo, p.chain.join(" > "));
                for b in &p.blocks {
                    let meta = params::algorithm(b.module, b.algorithm);
                    let name = b.algorithm_name.as_deref().unwrap_or("?");
                    let values = meta.map_or_else(String::new, |a| {
                        a.params
                            .iter()
                            .filter_map(|pm| Some(format!("{}={}", pm.name, shown_value(pm, *b.params.get(pm.index)?))))
                            .collect::<Vec<_>>()
                            .join(", ")
                    });
                    s += &format!("- {} [{}] {name}: {values}\n", b.module, if b.on { "on" } else { "off" });
                }
                ToolOutcome::Text(s)
            }
        },
        "propor_timbre" => match serde_json::from_value::<Proposal>(input.clone()) {
            Err(e) => ToolOutcome::Error(format!("formato inválido: {e}")),
            Ok(p) => match resolve(&p) {
                Ok(_) => ToolOutcome::Proposal(
                    p,
                    "Proposta válida e mostrada ao usuário com um botão Aplicar. Explique as escolhas em poucas frases.".into(),
                ),
                Err(errs) => ToolOutcome::Error(format!("Corrija e proponha de novo:\n- {}", errs.join("\n- "))),
            },
        },
        other => ToolOutcome::Error(format!("ferramenta desconhecida: {other}")),
    }
}

// ---------------------------------------------------------------- transporte HTTP

/// POST/GET JSON → (status HTTP, corpo JSON). Abstraído para os testes simularem respostas.
pub trait Transport {
    fn post(&self, url: &str, headers: &[(String, String)], body: &Value) -> Result<(u16, Value)>;
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<(u16, Value)>;
}

#[cfg(feature = "assistant")]
pub struct HttpTransport {
    agent: ureq::Agent,
}

#[cfg(feature = "assistant")]
impl Default for HttpTransport {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(240)))
            .http_status_as_error(false)
            .build()
            .into();
        HttpTransport { agent }
    }
}

#[cfg(feature = "assistant")]
impl Transport for HttpTransport {
    fn post(&self, url: &str, headers: &[(String, String)], body: &Value) -> Result<(u16, Value)> {
        let mut req = self.agent.post(url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .send_json(body)
            .map_err(|e| anyhow!("falha de rede: {e}"))?;
        read_json(resp)
    }

    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<(u16, Value)> {
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req.call().map_err(|e| anyhow!("falha de rede: {e}"))?;
        read_json(resp)
    }
}

#[cfg(feature = "assistant")]
fn read_json(mut resp: ureq::http::Response<ureq::Body>) -> Result<(u16, Value)> {
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| anyhow!("resposta ilegível: {e}"))?;
    let json = serde_json::from_str(&text).unwrap_or_else(|_| json!({"raw": text}));
    Ok((status, json))
}

fn api_error(status: u16, body: &Value) -> anyhow::Error {
    let msg = body
        .pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| body.get("raw").and_then(Value::as_str))
        .unwrap_or("erro desconhecido");
    let hint = match status {
        401 | 403 => " (confira a chave da API na configuração do assistente)",
        404 => " (confira a URL e o nome do modelo)",
        429 => " (limite de uso atingido; tente de novo em instantes)",
        _ => "",
    };
    anyhow!("HTTP {status}: {msg}{hint}")
}

// ---------------------------------------------------------------- conversa

/// Resultado de uma rodada (uma mensagem do usuário → resposta final do agente).
#[derive(Debug, Default)]
pub struct Turn {
    pub text: String,
    pub proposals: Vec<Proposal>,
}

const MAX_STEPS: usize = 12;

pub struct Conversation {
    cfg: Config,
    /// Histórico no formato do provedor (append-only: as respostas entram completas, sem edição).
    messages: Vec<Value>,
}

impl Conversation {
    pub fn new(cfg: Config) -> Conversation {
        Conversation {
            cfg,
            messages: Vec::new(),
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Envia uma mensagem do usuário e roda o loop de ferramentas até a resposta final.
    /// `activity` recebe descrições curtas do que o agente está consultando.
    pub fn send(
        &mut self,
        user: &str,
        ctx: &ToolContext,
        transport: &dyn Transport,
        activity: &mut dyn FnMut(&str),
    ) -> Result<Turn> {
        if self.cfg.api_key().is_empty() && self.cfg.provider != Provider::Compatible {
            bail!(
                "sem chave da API: informe-a na configuração do assistente ou defina {}",
                self.cfg.provider.key_env().unwrap_or("")
            );
        }
        let checkpoint = self.messages.len();
        let result = match self.cfg.provider {
            Provider::Claude => self.send_claude(user, ctx, transport, activity),
            Provider::ChatGpt | Provider::Compatible => {
                self.send_openai(user, ctx, transport, activity)
            }
        };
        if result.is_err() {
            // descarta a rodada incompleta para a próxima mensagem não herdar um histórico inválido
            self.messages.truncate(checkpoint);
        }
        result
    }

    fn handle_tool(
        &self,
        name: &str,
        input: &Value,
        ctx: &ToolContext,
        turn: &mut Turn,
        activity: &mut dyn FnMut(&str),
    ) -> (String, bool) {
        activity(&match name {
            "listar_algoritmos" => format!(
                "consultando os algoritmos de {}…",
                input.get("module").and_then(Value::as_str).unwrap_or("?")
            ),
            "detalhar_algoritmo" => format!(
                "vendo os parâmetros de {}…",
                input.get("name").and_then(Value::as_str).unwrap_or("?")
            ),
            "preset_atual" => "lendo o preset atual…".into(),
            "propor_timbre" => "montando a proposta…".into(),
            other => format!("{other}…"),
        });
        match run_tool(name, input, ctx) {
            ToolOutcome::Text(t) => (t, false),
            ToolOutcome::Error(e) => (e, true),
            ToolOutcome::Proposal(p, msg) => {
                turn.proposals.push(p);
                (msg, false)
            }
        }
    }

    fn send_claude(
        &mut self,
        user: &str,
        ctx: &ToolContext,
        transport: &dyn Transport,
        activity: &mut dyn FnMut(&str),
    ) -> Result<Turn> {
        let ep = self.cfg.claude.clone();
        let url = format!("{}/v1/messages", ep.base_url.trim_end_matches('/'));
        let official = ep.base_url.contains("api.anthropic.com");
        let mut headers = self.cfg.auth_headers();
        if official && self.cfg.claude_server_fallback {
            headers.push((
                "anthropic-beta".to_string(),
                "server-side-fallback-2026-07-01".to_string(),
            ));
        }
        let tools: Vec<Value> = tool_specs()
            .into_iter()
            .map(
                |t| json!({"name": t.name, "description": t.description, "input_schema": t.schema}),
            )
            .collect();

        self.messages.push(json!({"role": "user", "content": user}));
        let mut turn = Turn::default();
        for _ in 0..MAX_STEPS {
            let mut body = json!({
                "model": ep.model,
                "max_tokens": 16000,
                "system": [{"type": "text", "text": self.cfg.system_prompt}],
                "tools": tools,
                "messages": self.messages,
                "cache_control": {"type": "ephemeral"},
            });
            if !self.cfg.claude_effort.trim().is_empty() {
                body["output_config"] = json!({"effort": self.cfg.claude_effort.trim()});
            }
            if official && self.cfg.claude_server_fallback {
                body["fallbacks"] = json!("default");
            }
            let (status, resp) = transport.post(&url, &headers, &body)?;
            if status != 200 {
                return Err(api_error(status, &resp));
            }
            let content = resp.get("content").cloned().unwrap_or_else(|| json!([]));
            let stop = resp
                .get("stop_reason")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            for block in content.as_array().into_iter().flatten() {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    let t = block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !t.trim().is_empty() {
                        if !turn.text.is_empty() {
                            turn.text.push_str("\n\n");
                        }
                        turn.text.push_str(t.trim());
                    }
                }
            }
            match stop.as_str() {
                "refusal" => {
                    // a resposta recusada não entra no histórico; a conversa pode continuar com outro pedido
                    self.messages.pop();
                    bail!("o modelo recusou responder a este pedido; tente reformular");
                }
                "max_tokens" => {
                    bail!("a resposta ficou longa demais e foi cortada; peça algo mais específico")
                }
                _ => {}
            }
            // histórico append-only: a resposta entra inteira (inclui blocos de thinking e de fallback)
            self.messages
                .push(json!({"role": "assistant", "content": content}));
            if stop != "tool_use" {
                return Ok(turn);
            }
            let mut results = Vec::new();
            for block in content.as_array().into_iter().flatten() {
                if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                    continue;
                }
                let name = block
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                let (out, is_error) = self.handle_tool(name, &input, ctx, &mut turn, activity);
                results.push(json!({"type": "tool_result", "tool_use_id": block.get("id").cloned().unwrap_or_default(), "content": out, "is_error": is_error}));
            }
            // todos os resultados numa única mensagem do usuário
            self.messages
                .push(json!({"role": "user", "content": results}));
        }
        bail!("o agente usou ferramentas demais nesta rodada; tente um pedido mais específico")
    }

    fn send_openai(
        &mut self,
        user: &str,
        ctx: &ToolContext,
        transport: &dyn Transport,
        activity: &mut dyn FnMut(&str),
    ) -> Result<Turn> {
        let ep = self.cfg.endpoint().clone();
        let url = format!("{}/chat/completions", ep.base_url.trim_end_matches('/'));
        let headers = self.cfg.auth_headers();
        let tools: Vec<Value> = tool_specs()
            .into_iter()
            .map(|t| json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.schema}}))
            .collect();

        if self.messages.is_empty() {
            self.messages
                .push(json!({"role": "system", "content": self.cfg.system_prompt}));
        }
        self.messages.push(json!({"role": "user", "content": user}));
        let mut turn = Turn::default();
        for _ in 0..MAX_STEPS {
            let body = json!({"model": ep.model, "messages": self.messages, "tools": tools, "tool_choice": "auto"});
            let (status, resp) = transport.post(&url, &headers, &body)?;
            if status != 200 {
                return Err(api_error(status, &resp));
            }
            let choice = resp
                .pointer("/choices/0")
                .ok_or_else(|| anyhow!("resposta sem choices"))?;
            let message = choice
                .get("message")
                .cloned()
                .ok_or_else(|| anyhow!("resposta sem message"))?;
            if let Some(t) = message
                .get("content")
                .and_then(Value::as_str)
                .filter(|t| !t.trim().is_empty())
            {
                if !turn.text.is_empty() {
                    turn.text.push_str("\n\n");
                }
                turn.text.push_str(t.trim());
            }
            if choice.get("finish_reason").and_then(Value::as_str) == Some("length") {
                bail!("a resposta ficou longa demais e foi cortada; peça algo mais específico");
            }
            let calls: Vec<Value> = message
                .get("tool_calls")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            self.messages.push(message);
            if calls.is_empty() {
                return Ok(turn);
            }
            for call in calls {
                let name = call
                    .pointer("/function/name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let args = call
                    .pointer("/function/arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let (out, _) = match serde_json::from_str::<Value>(args) {
                    Ok(input) => self.handle_tool(&name, &input, ctx, &mut turn, activity),
                    Err(e) => (format!("argumentos não são JSON válido: {e}"), true),
                };
                self.messages.push(json!({"role": "tool", "tool_call_id": call.get("id").cloned().unwrap_or_default(), "content": out}));
            }
        }
        bail!("o agente usou ferramentas demais nesta rodada; tente um pedido mais específico")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// (url, cabeçalhos, corpo) de cada requisição enviada.
    type Sent = (String, Vec<(String, String)>, Value);

    /// Devolve respostas prontas em ordem e guarda o que foi enviado.
    struct Fake {
        replies: RefCell<Vec<(u16, Value)>>,
        sent: RefCell<Vec<Sent>>,
    }

    impl Fake {
        fn new(replies: Vec<(u16, Value)>) -> Fake {
            Fake {
                replies: RefCell::new(replies.into_iter().rev().collect()),
                sent: RefCell::new(vec![]),
            }
        }
    }

    impl Transport for Fake {
        fn post(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &Value,
        ) -> Result<(u16, Value)> {
            self.sent
                .borrow_mut()
                .push((url.to_string(), headers.to_vec(), body.clone()));
            self.replies
                .borrow_mut()
                .pop()
                .ok_or_else(|| anyhow!("sem mais respostas"))
        }

        fn get(&self, url: &str, headers: &[(String, String)]) -> Result<(u16, Value)> {
            self.post(url, headers, &Value::Null)
        }
    }

    #[test]
    fn lists_models_from_the_provider_api() {
        let fake = Fake::new(vec![(
            200,
            json!({"data": [{"id": "claude-opus-5-5"}, {"id": "claude-sonnet-5-5"}], "has_more": false}),
        )]);
        let ids = list_models(&cfg(Provider::Claude), &fake).unwrap();
        assert_eq!(ids, ["claude-opus-5-5", "claude-sonnet-5-5"]);
        let sent = fake.sent.borrow();
        assert_eq!(sent[0].0, "https://api.anthropic.com/v1/models?limit=100");
        assert!(
            sent[0]
                .1
                .contains(&("x-api-key".into(), "chave-teste".into()))
        );

        let fake = Fake::new(vec![(
            200,
            json!({"data": [{"id": "gpt-5"}, {"id": "text-embedding-3-small"}, {"id": "gpt-5-mini"}, {"id": "tts-1"}]}),
        )]);
        let ids = list_models(&cfg(Provider::ChatGpt), &fake).unwrap();
        assert_eq!(ids, ["gpt-5", "gpt-5-mini"]);
        assert_eq!(fake.sent.borrow()[0].0, "https://api.openai.com/v1/models");

        let fake = Fake::new(vec![(
            401,
            json!({"error": {"message": "invalid x-api-key"}}),
        )]);
        let err = list_models(&cfg(Provider::Claude), &fake)
            .unwrap_err()
            .to_string();
        assert!(err.contains("401"), "{err}");
    }

    fn ctx() -> ToolContext<'static> {
        ToolContext {
            preset: None,
            resources: None,
        }
    }

    fn cfg(provider: Provider) -> Config {
        let mut c = Config {
            provider,
            ..Config::default()
        };
        c.endpoint_mut().api_key = "chave-teste".into();
        c
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
            run_tool("propor_timbre", &proposal_json(), &ctx()),
            ToolOutcome::Proposal(..)
        ));
    }

    #[test]
    fn claude_loop_runs_tools_and_collects_the_proposal() {
        let fake = Fake::new(vec![
            (
                200,
                json!({
                    "stop_reason": "tool_use",
                    "content": [
                        {"type": "text", "text": "Vou montar."},
                        {"type": "tool_use", "id": "toolu_1", "name": "listar_algoritmos", "input": {"module": "AMP"}},
                        {"type": "tool_use", "id": "toolu_2", "name": "propor_timbre", "input": proposal_json()}
                    ]
                }),
            ),
            (
                200,
                json!({"stop_reason": "end_turn", "content": [{"type": "text", "text": "Pronto, clique em Aplicar."}]}),
            ),
        ]);
        let mut conv = Conversation::new(cfg(Provider::Claude));
        let mut seen = vec![];
        let turn = conv
            .send("timbre do Slash", &ctx(), &fake, &mut |a| {
                seen.push(a.to_string())
            })
            .unwrap();
        assert_eq!(turn.proposals.len(), 1);
        assert_eq!(turn.text, "Vou montar.\n\nPronto, clique em Aplicar.");
        assert_eq!(seen.len(), 2);

        let sent = fake.sent.borrow();
        let (url, headers, first) = &sent[0];
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert!(headers.contains(&("x-api-key".into(), "chave-teste".into())));
        assert!(headers.contains(&(
            "anthropic-beta".into(),
            "server-side-fallback-2026-07-01".into()
        )));
        assert_eq!(first["model"], "claude-opus-5-5");
        assert_eq!(first["fallbacks"], "default");
        assert_eq!(first["output_config"]["effort"], "medium");
        assert_eq!(first["tools"].as_array().unwrap().len(), 4);
        // 2ª chamada: resposta do assistente inteira + os dois resultados numa única mensagem do usuário
        let msgs = sent[1].2["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1]["content"].as_array().unwrap().len(), 3);
        let results = msgs[2]["content"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["tool_use_id"], "toolu_1");
        assert_eq!(results[1]["is_error"], false);
    }

    #[test]
    fn claude_refusal_is_an_error_and_keeps_history_consistent() {
        let fake = Fake::new(vec![(
            200,
            json!({"stop_reason": "refusal", "content": []}),
        )]);
        let mut conv = Conversation::new(cfg(Provider::Claude));
        assert!(conv.send("algo", &ctx(), &fake, &mut |_| {}).is_err());
        assert!(conv.is_empty());
    }

    #[test]
    fn openai_compatible_loop_and_custom_endpoint() {
        let call = json!({"id": "call_1", "type": "function", "function": {"name": "propor_timbre", "arguments": proposal_json().to_string()}});
        let fake = Fake::new(vec![
            (
                200,
                json!({"choices": [{"finish_reason": "tool_calls", "message": {"role": "assistant", "content": null, "tool_calls": [call]}}]}),
            ),
            (
                200,
                json!({"choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "Feito."}}]}),
            ),
        ]);
        let mut c = cfg(Provider::Compatible);
        c.compatible.api_key.clear(); // servidor local sem chave
        let mut conv = Conversation::new(c);
        let turn = conv
            .send("timbre limpo", &ctx(), &fake, &mut |_| {})
            .unwrap();
        assert_eq!((turn.proposals.len(), turn.text.as_str()), (1, "Feito."));
        let sent = fake.sent.borrow();
        assert_eq!(sent[0].0, "http://localhost:11434/v1/chat/completions");
        assert!(sent[0].1.is_empty()); // sem Authorization
        let msgs = sent[1].2["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[3]["role"], "tool");
        assert_eq!(msgs[3]["tool_call_id"], "call_1");
    }

    #[test]
    fn http_errors_become_readable_messages() {
        let fake = Fake::new(vec![(
            401,
            json!({"error": {"message": "invalid x-api-key"}}),
        )]);
        let mut conv = Conversation::new(cfg(Provider::ChatGpt));
        let err = conv
            .send("oi", &ctx(), &fake, &mut |_| {})
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("401") && err.contains("invalid x-api-key") && err.contains("chave"),
            "{err}"
        );
        assert!(conv.is_empty());
    }
}
