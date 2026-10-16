//! Servidor MCP (Model Context Protocol) do assistente de timbre: deixa um cliente MCP, como o Claude
//! Desktop ou o Claude Code, usar as mesmas ferramentas do painel do assistente (catálogo, preset atual)
//! e aplicar timbres no buffer de edição, com a assinatura do próprio usuário, sem chave de API.
//!
//! Como a porta MIDI do Ampero é exclusiva no Windows, quem fala com a pedaleira é a GUI aberta: ela
//! atende JSON-RPC por HTTP em `127.0.0.1` (só esta máquina, com token). O comando `ampero mcp` é a
//! ponte stdio que o Claude Desktop abre: responde sozinho ao handshake e à lista de ferramentas e
//! repassa as chamadas para a GUI.
//!
//! Nada é salvo na pedaleira por aqui: `aplicar_timbre` só muda o buffer de edição.

use crate::timbre;
use serde_json::{Value, json};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

pub const DEFAULT_PORT: u16 = 47800;
const PATH: &str = "/mcp";
const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_BODY: usize = 1 << 20;

const INSTRUCTIONS: &str = "Ferramentas do Ampero (pedaleira Hotone Ampero). Para montar um timbre: consulte \
os blocos com listar_algoritmos e detalhar_algoritmo, use preset_atual quando o pedido for ajustar o timbre \
atual, e aplique com aplicar_timbre usando só algoritmos e parâmetros que existem. aplicar_timbre muda só o \
buffer de edição; o usuário salva pela pedaleira ou pelo app se gostar.";

// ---------------------------------------------------------------- ferramentas

/// Quem executa as ferramentas (a GUI, que tem a conexão com a pedaleira).
pub trait ToolHost {
    /// Executa uma ferramenta; devolve (texto, é erro).
    fn call(&mut self, name: &str, args: &Value) -> (String, bool);
}

/// Ferramentas expostas (as de `timbre`).
pub fn tool_list() -> Value {
    let tools: Vec<Value> = timbre::tool_specs()
        .into_iter()
        .map(|t| json!({"name": t.name, "description": t.description, "inputSchema": t.schema}))
        .collect();
    json!({ "tools": tools })
}

// ---------------------------------------------------------------- JSON-RPC

fn result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn initialize_result(params: &Value) -> Value {
    let asked = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or("");
    let version = SUPPORTED_VERSIONS
        .iter()
        .find(|v| **v == asked)
        .unwrap_or(&SUPPORTED_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "ampero", "version": env!("CARGO_PKG_VERSION")},
        "instructions": INSTRUCTIONS,
    })
}

/// Responde uma mensagem JSON-RPC. `None` para notificações (sem `id`), que não têm resposta.
pub fn handle_message(msg: &Value, host: &mut dyn ToolHost) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let reply = match msg.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => result(&id, initialize_result(&params)),
        "ping" => result(&id, json!({})),
        "tools/list" => result(&id, tool_list()),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let (text, is_error) = host.call(name, &args);
            result(
                &id,
                json!({"content": [{"type": "text", "text": text}], "isError": is_error}),
            )
        }
        other => error(&id, -32601, &format!("método não suportado: {other}")),
    };
    Some(reply)
}

// ---------------------------------------------------------------- token

/// Arquivo com o token que a GUI exige nas requisições (criado na primeira vez).
pub fn token_path() -> Option<PathBuf> {
    Some(crate::config_dir()?.join("mcp-token"))
}

/// Lê o token, ou cria um novo (aleatório) se ainda não existir.
pub fn load_or_create_token() -> io::Result<String> {
    let path =
        token_path().ok_or_else(|| io::Error::other("pasta de configuração não encontrada"))?;
    if let Ok(t) = std::fs::read_to_string(&path)
        && t.trim().len() >= 32
    {
        return Ok(t.trim().to_string());
    }
    use std::hash::{BuildHasher, Hasher};
    let mut token = String::new();
    for i in 0..4u64 {
        // RandomState é semeado pelo sistema operacional a cada processo
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u32(std::process::id());
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        token += &format!("{:016x}", h.finish());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, &token)?;
    Ok(token)
}

// ---------------------------------------------------------------- HTTP (servidor)

pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub fn read_request(stream: &mut impl Read) -> io::Result<HttpRequest> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();
    let mut headers = Vec::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
        if headers.len() > 100 {
            return Err(io::Error::other("cabeçalhos demais"));
        }
    }
    let len: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    if len > MAX_BODY {
        return Err(io::Error::other("corpo grande demais"));
    }
    let mut body = vec![0; len];
    reader.read_exact(&mut body)?;
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

pub fn write_response(stream: &mut impl Write, status: u16, body: &str) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

/// Confere a requisição: só desta máquina (Host), nunca de uma página web (Origin) e com o token.
/// `Err((status, motivo))` quando deve ser recusada.
pub fn check_request(req: &HttpRequest, port: u16, token: &str) -> Result<(), (u16, &'static str)> {
    if req.path.split('?').next() != Some(PATH) {
        return Err((404, "use o caminho /mcp"));
    }
    if req.method != "POST" {
        return Err((405, "só POST"));
    }
    let host_ok = req.header("host").is_some_and(|h| {
        [format!("127.0.0.1:{port}"), format!("localhost:{port}")]
            .iter()
            .any(|ok| h.eq_ignore_ascii_case(ok))
    });
    if !host_ok {
        return Err((403, "host não permitido"));
    }
    // navegadores sempre mandam Origin num POST; clientes MCP não
    if req.header("origin").is_some() {
        return Err((403, "requisições de páginas web não são aceitas"));
    }
    let auth = req.header("authorization").unwrap_or("");
    if auth.strip_prefix("Bearer ").map(str::trim) != Some(token) {
        return Err((401, "token inválido"));
    }
    Ok(())
}

/// Atende uma conexão HTTP: valida, passa o JSON-RPC para `host` e responde.
pub fn serve_connection(mut stream: TcpStream, port: u16, token: &str, host: &mut dyn ToolHost) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let req = match read_request(&mut stream) {
        Ok(r) => r,
        Err(_) => return,
    };
    if let Err((status, why)) = check_request(&req, port, token) {
        let _ = write_response(&mut stream, status, &json!({"error": why}).to_string());
        return;
    }
    let msg: Value = match serde_json::from_slice(&req.body) {
        Ok(v) => v,
        Err(e) => {
            let reply = error(&Value::Null, -32700, &format!("JSON inválido: {e}"));
            let _ = write_response(&mut stream, 400, &reply.to_string());
            return;
        }
    };
    match handle_message(&msg, host) {
        Some(reply) => {
            let _ = write_response(&mut stream, 200, &reply.to_string());
        }
        None => {
            let _ = write_response(&mut stream, 202, "");
        }
    }
}

// ---------------------------------------------------------------- HTTP (cliente) e ponte stdio

/// POST de um JSON-RPC para a GUI. Erro de conexão = GUI fechada ou servidor desligado.
pub fn post(port: u16, token: &str, body: &str, timeout: Duration) -> io::Result<(u16, String)> {
    let addr = ("127.0.0.1", port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::other("endereço inválido"))?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(timeout))?;
    write!(
        stream,
        "POST {PATH} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()?;
    let mut raw = String::new();
    stream.read_to_string(&mut raw)?;
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    Ok((status, body.to_string()))
}

/// Ferramentas executadas na GUI via HTTP (usado pela ponte stdio).
pub struct RemoteHost {
    pub port: u16,
}

impl ToolHost for RemoteHost {
    fn call(&mut self, name: &str, args: &Value) -> (String, bool) {
        const CLOSED: &str = "A GUI do Ampero não está aberta, ou o servidor MCP está desligado (menu ⚙ → \
                              \"Conectar ao Claude…\" → Ligar). Peça ao usuário para abrir o app e ligar o servidor.";
        let token = match token_path().and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(t) => t.trim().to_string(),
            None => return (CLOSED.into(), true),
        };
        let msg = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                         "params": {"name": name, "arguments": args}});
        let (status, body) =
            match post(self.port, &token, &msg.to_string(), Duration::from_secs(30)) {
                Ok(r) => r,
                Err(_) => return (CLOSED.into(), true),
            };
        if status == 401 {
            return (
                "O token não confere: reinicie o servidor MCP no app (menu ⚙).".into(),
                true,
            );
        }
        let reply: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        let res = &reply["result"];
        let text = res["content"][0]["text"].as_str().map(str::to_string);
        match text {
            Some(t) => (t, res["isError"].as_bool().unwrap_or(false)),
            None => (
                format!("resposta inesperada da GUI (HTTP {status}): {body}"),
                true,
            ),
        }
    }
}

/// Ponte stdio: lê JSON-RPC por linha da entrada padrão e responde na saída padrão (formato que o
/// Claude Desktop usa para servidores locais). Mensagens de log vão para a saída de erro.
pub fn run_stdio_bridge(port: u16) -> io::Result<()> {
    let mut host = RemoteHost { port };
    let stdin = io::stdin();
    let mut out = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        // alguns terminais (PowerShell) mandam um BOM UTF-8 no começo da entrada
        let line = line.trim_start_matches('\u{feff}');
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(msg) => handle_message(&msg, &mut host),
            Err(e) => Some(error(&Value::Null, -32700, &format!("JSON inválido: {e}"))),
        };
        if let Some(reply) = reply {
            writeln!(out, "{reply}")?;
            out.flush()?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- configuração do Claude Desktop

/// Entrada `mcpServers.ampero` para o Claude Desktop: ele abre `ampero mcp` como servidor local.
pub fn claude_desktop_entry(cli: &std::path::Path, port: u16) -> Value {
    let mut args = vec![json!("mcp")];
    if port != DEFAULT_PORT {
        args.push(json!(port.to_string()));
    }
    json!({"command": cli.to_string_lossy(), "args": args})
}

/// Arquivos de configuração do Claude Desktop que existem nesta máquina (no Windows, a versão da
/// Microsoft Store guarda uma cópia dentro do pacote). Se nenhum existir, o caminho padrão.
pub fn claude_desktop_config_paths() -> Vec<PathBuf> {
    let home = |rel: &str| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rel));
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            candidates.push(
                PathBuf::from(appdata)
                    .join("Claude")
                    .join("claude_desktop_config.json"),
            );
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA")
            && let Ok(entries) = std::fs::read_dir(PathBuf::from(local).join("Packages"))
        {
            for e in entries.flatten() {
                if e.file_name().to_string_lossy().starts_with("Claude_") {
                    candidates.push(
                        e.path()
                            .join(r"LocalCache\Roaming\Claude\claude_desktop_config.json"),
                    );
                }
            }
        }
    } else if cfg!(target_os = "macos") {
        candidates.extend(home(
            "Library/Application Support/Claude/claude_desktop_config.json",
        ));
    } else {
        candidates.extend(home(".config/Claude/claude_desktop_config.json"));
    }
    let existing: Vec<PathBuf> = candidates.iter().filter(|p| p.is_file()).cloned().collect();
    if existing.is_empty() {
        candidates.into_iter().take(1).collect()
    } else {
        existing
    }
}

/// Acrescenta (ou atualiza) `mcpServers.ampero` num JSON de configuração, sem mexer no resto.
pub fn merge_claude_desktop_config(existing: &str, entry: Value) -> Result<String, String> {
    let mut root: Value = if existing.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(existing)
            .map_err(|e| format!("o arquivo atual não é um JSON válido: {e}"))?
    };
    let obj = root
        .as_object_mut()
        .ok_or("o arquivo atual não é um objeto JSON")?;
    let servers = obj.entry("mcpServers").or_insert_with(|| json!({}));
    servers
        .as_object_mut()
        .ok_or("\"mcpServers\" não é um objeto")?
        .insert("ampero".into(), entry);
    serde_json::to_string_pretty(&root).map_err(|e| e.to_string())
}

/// Grava a entrada do Ampero em cada configuração do Claude Desktop, com backup do arquivo anterior.
/// Devolve os arquivos alterados.
pub fn install_claude_desktop(cli: &std::path::Path, port: u16) -> Result<Vec<PathBuf>, String> {
    let entry = claude_desktop_entry(cli, port);
    let mut done = Vec::new();
    for path in claude_desktop_config_paths() {
        let current = std::fs::read_to_string(&path).unwrap_or_default();
        let merged = merge_claude_desktop_config(&current, entry.clone())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        if !current.is_empty() {
            let _ = std::fs::write(path.with_extension("json.bak-ampero"), &current);
        }
        std::fs::write(&path, merged).map_err(|e| format!("{}: {e}", path.display()))?;
        done.push(path);
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Echo(Vec<String>);
    impl ToolHost for Echo {
        fn call(&mut self, name: &str, _args: &Value) -> (String, bool) {
            self.0.push(name.to_string());
            (format!("ok {name}"), name == "falha")
        }
    }

    #[test]
    fn handshake_tools_and_calls() {
        let mut host = Echo(vec![]);
        let init = handle_message(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}),
            &mut host,
        )
        .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(init["result"]["serverInfo"]["name"], "ampero");
        let unknown = handle_message(
            &json!({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {"protocolVersion": "1999"}}),
            &mut host,
        )
        .unwrap();
        assert_eq!(unknown["result"]["protocolVersion"], SUPPORTED_VERSIONS[0]);

        assert!(
            handle_message(
                &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                &mut host
            )
            .is_none()
        );

        let list = handle_message(
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
            &mut host,
        )
        .unwrap();
        let names: Vec<&str> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "listar_algoritmos",
                "detalhar_algoritmo",
                "preset_atual",
                "aplicar_timbre"
            ]
        );

        let call = json!({"jsonrpc": "2.0", "id": "x", "method": "tools/call", "params": {"name": "falha", "arguments": {}}});
        let r = handle_message(&call, &mut host).unwrap();
        assert_eq!(r["id"], "x");
        assert_eq!(r["result"]["content"][0]["text"], "ok falha");
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(host.0, ["falha"]);

        let bad = handle_message(
            &json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"}),
            &mut host,
        )
        .unwrap();
        assert_eq!(bad["error"]["code"], -32601);
    }

    fn req(method: &str, path: &str, headers: &[(&str, &str)]) -> HttpRequest {
        HttpRequest {
            method: method.into(),
            path: path.into(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: vec![],
        }
    }

    #[test]
    fn only_local_clients_with_the_token_get_in() {
        let good = [("Host", "127.0.0.1:47800"), ("Authorization", "Bearer abc")];
        assert!(check_request(&req("POST", "/mcp", &good), 47800, "abc").is_ok());
        assert_eq!(
            check_request(&req("GET", "/mcp", &good), 47800, "abc")
                .unwrap_err()
                .0,
            405
        );
        assert_eq!(
            check_request(&req("POST", "/x", &good), 47800, "abc")
                .unwrap_err()
                .0,
            404
        );
        let wrong_token = [
            ("Host", "localhost:47800"),
            ("Authorization", "Bearer nope"),
        ];
        assert_eq!(
            check_request(&req("POST", "/mcp", &wrong_token), 47800, "abc")
                .unwrap_err()
                .0,
            401
        );
        // DNS rebinding: host de fora
        let rebinding = [
            ("Host", "evil.example:47800"),
            ("Authorization", "Bearer abc"),
        ];
        assert_eq!(
            check_request(&req("POST", "/mcp", &rebinding), 47800, "abc")
                .unwrap_err()
                .0,
            403
        );
        // página web no navegador
        let browser = [
            ("Host", "127.0.0.1:47800"),
            ("Origin", "https://evil.example"),
            ("Authorization", "Bearer abc"),
        ];
        assert_eq!(
            check_request(&req("POST", "/mcp", &browser), 47800, "abc")
                .unwrap_err()
                .0,
            403
        );
    }

    #[test]
    fn claude_desktop_config_keeps_everything_else() {
        let existing = r#"{"preferences": {"zeta": true, "alpha": 1}, "mcpServers": {"outro": {"command": "x"}}}"#;
        let entry =
            claude_desktop_entry(std::path::Path::new(r"C:\Ampero\ampero.exe"), DEFAULT_PORT);
        let merged = merge_claude_desktop_config(existing, entry).unwrap();
        let v: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["mcpServers"]["outro"]["command"], "x");
        assert_eq!(
            v["mcpServers"]["ampero"]["command"],
            r"C:\Ampero\ampero.exe"
        );
        assert_eq!(v["mcpServers"]["ampero"]["args"], json!(["mcp"]));
        // a ordem das chaves do usuário é preservada
        assert!(merged.find("zeta").unwrap() < merged.find("alpha").unwrap());

        let fresh = merge_claude_desktop_config(
            "",
            claude_desktop_entry(std::path::Path::new("ampero"), 5000),
        )
        .unwrap();
        let v: Value = serde_json::from_str(&fresh).unwrap();
        assert_eq!(v["mcpServers"]["ampero"]["args"], json!(["mcp", "5000"]));
        assert!(merge_claude_desktop_config("[1]", json!({})).is_err());
        assert!(merge_claude_desktop_config("{quebrado", json!({})).is_err());
    }

    #[test]
    fn http_round_trip_over_a_real_socket() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                serve_connection(stream, port, "segredo", &mut Echo(vec![]));
            }
        });
        let call = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "preset_atual"}});
        let (status, body) =
            post(port, "segredo", &call.to_string(), Duration::from_secs(5)).unwrap();
        assert_eq!(status, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["content"][0]["text"], "ok preset_atual");
        let (status, _) = post(port, "errado", &call.to_string(), Duration::from_secs(5)).unwrap();
        assert_eq!(status, 401);
        server.join().unwrap();
    }
}
