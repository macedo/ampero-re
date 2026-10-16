//! Servidor MCP da GUI: atende em `127.0.0.1` (com token) numa thread própria e repassa cada chamada de
//! ferramenta para a thread da interface, que tem o preset atual e a conexão com a pedaleira.

use ampero::mcp::{self, ToolHost};
use eframe::egui;
use serde_json::Value;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

/// Uma chamada de ferramenta esperando a interface executar e responder.
pub struct UiCall {
    pub name: String,
    pub args: Value,
    pub reply: Sender<(String, bool)>,
}

pub struct McpServer {
    pub port: u16,
    stop: Arc<AtomicBool>,
    calls: Receiver<UiCall>,
    /// Requisições atendidas até agora (o indicador pisca quando o número muda).
    requests: Arc<AtomicUsize>,
}

struct UiHost {
    tx: Sender<UiCall>,
    ctx: egui::Context,
}

impl ToolHost for UiHost {
    fn call(&mut self, name: &str, args: &Value) -> (String, bool) {
        let (reply, rx) = channel();
        let call = UiCall {
            name: name.to_string(),
            args: args.clone(),
            reply,
        };
        if self.tx.send(call).is_err() {
            return ("o app está fechando".into(), true);
        }
        self.ctx.request_repaint();
        rx.recv_timeout(Duration::from_secs(20))
            .unwrap_or_else(|_| ("a interface do Ampero não respondeu a tempo".into(), true))
    }
}

impl McpServer {
    pub fn start(ctx: &egui::Context, port: u16) -> Result<McpServer, String> {
        let token =
            mcp::load_or_create_token().map_err(|e| format!("não deu para criar o token: {e}"))?;
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| format!("porta {port} indisponível ({e}); outro Ampero aberto?"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, calls) = channel();
        let mut host = UiHost {
            tx,
            ctx: ctx.clone(),
        };
        let flag = stop.clone();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        let repaint = ctx.clone();
        thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        counter.fetch_add(1, Ordering::Relaxed);
                        repaint.request_repaint();
                        mcp::serve_connection(stream, port, &token, &mut host);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => thread::sleep(Duration::from_millis(200)),
                }
            }
        });
        Ok(McpServer {
            port,
            stop,
            calls,
            requests,
        })
    }

    /// Chamadas que chegaram desde o último frame.
    pub fn take_calls(&self) -> Vec<UiCall> {
        self.calls.try_iter().collect()
    }

    pub fn request_count(&self) -> usize {
        self.requests.load(Ordering::Relaxed)
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
