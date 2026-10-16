//! Atualização do próprio app pelas Releases do GitHub: procura uma versão mais nova, baixa o
//! instalador, confere o SHA-256 com o `SHA256SUMS` da release e roda o instalador em modo
//! silencioso. O instalador fecha o app, atualiza no lugar e abre a versão nova (`/RESTARTAPP=1`).
//!
//! Só o Windows tem instalador; no macOS e no Linux o aviso leva à página da release.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use eframe::egui;
use sha2::{Digest, Sha256};

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/macedo/ampero-re/releases/latest";
/// Instalador e checksums só são aceitos das releases deste repositório.
const DOWNLOAD_PREFIX: &str = "https://github.com/macedo/ampero-re/releases/download/";
/// Chave que o Inno Setup grava para o app (`AppId` em installer/ampero.iss).
#[cfg(windows)]
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{50676992-6BAD-419D-914B-010B3C748C3B}_is1";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Uma versão publicada e onde baixá-la.
#[derive(Clone, Debug, PartialEq)]
pub struct AppRelease {
    pub version: String,
    /// Página da release, com as novidades.
    pub notes_url: String,
    /// Instalador do Windows e o arquivo de checksums, quando a release tem os dois.
    installer: Option<Installer>,
}

#[derive(Clone, Debug, PartialEq)]
struct Installer {
    name: String,
    url: String,
    sums_url: String,
}

impl AppRelease {
    /// Dá para atualizar sozinho: Windows, release com instalador e cópia instalada pelo instalador.
    pub fn can_self_update(&self) -> bool {
        self.installer.is_some() && running_installed_copy()
    }
}

/// `true` se `latest` (ex.: "0.10.0") é mais nova que `current` ("0.9.2").
pub fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim()
            .trim_start_matches('v')
            .split('.')
            .map(|p| p.parse().unwrap_or(0))
            .collect()
    };
    parse(latest) > parse(current)
}

/// Lê o JSON de "latest release" do GitHub. O instalador só conta se vier deste repositório e
/// tiver checksum publicado.
fn parse_release(v: &serde_json::Value) -> Option<AppRelease> {
    let version = v
        .get("tag_name")?
        .as_str()?
        .trim_start_matches('v')
        .to_owned();
    let notes_url = v.get("html_url")?.as_str()?.to_owned();
    let assets = v.get("assets").and_then(|a| a.as_array());
    let url_of = |name: &str| {
        assets?
            .iter()
            .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(name))
            .and_then(|a| a.get("browser_download_url")?.as_str())
            .filter(|url| url.starts_with(DOWNLOAD_PREFIX))
            .map(str::to_owned)
    };
    let name = format!("ampero-{version}-windows-x64-setup.exe");
    let installer = match (url_of(&name), url_of("SHA256SUMS")) {
        (Some(url), Some(sums_url)) => Some(Installer {
            name,
            url,
            sums_url,
        }),
        _ => None,
    };
    Some(AppRelease {
        version,
        notes_url,
        installer,
    })
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(timeout))
        .user_agent(concat!("ampero-gui/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// Resultado de uma verificação: `Ok(Some)` quando há versão mais nova, `Ok(None)` quando esta já é
/// a mais recente.
pub type CheckResult = Result<Option<AppRelease>, String>;

/// Consulta o GitHub em segundo plano.
pub fn check_for_update(ctx: egui::Context) -> Receiver<CheckResult> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = (|| {
            let mut resp = agent(Duration::from_secs(10))
                .get(LATEST_RELEASE_URL)
                .header("Accept", "application/vnd.github+json")
                .call()
                .map_err(|e| format!("não deu para consultar o GitHub: {e}"))?;
            let body = resp
                .body_mut()
                .read_to_string()
                .map_err(|e| format!("resposta ilegível do GitHub: {e}"))?;
            let json = serde_json::from_str(&body)
                .map_err(|e| format!("resposta ilegível do GitHub: {e}"))?;
            let release = parse_release(&json).ok_or("o GitHub não informou a última versão")?;
            Ok(Some(release).filter(|r| is_newer(&r.version, VERSION)))
        })();
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}

/// Esta cópia é a que o instalador colocou no lugar? Uma cópia rodando de outra pasta (ex.: do zip ou
/// da pasta de build) não seria trocada pelo instalador, então ela recebe o link da release.
pub fn running_installed_copy() -> bool {
    let (Some(dir), Ok(exe)) = (installed_dir(), std::env::current_exe()) else {
        return false;
    };
    let canonical = |p: &Path| std::fs::canonicalize(p).ok();
    match (exe.parent().and_then(canonical), canonical(&dir)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn installed_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(UNINSTALL_KEY)
            .ok()?;
        let dir: String = key.get_value("InstallLocation").ok()?;
        Some(PathBuf::from(dir))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

pub enum UpdateMsg {
    /// 0.0 a 1.0, quando o tamanho é conhecido.
    Progress(f32),
    /// Instalador conferido, pronto para rodar.
    Ready(PathBuf),
    Failed(String),
}

/// Baixa o instalador da release para a pasta temporária e confere o SHA-256.
pub fn download_update(release: AppRelease, ctx: egui::Context) -> Receiver<UpdateMsg> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let progress_tx = tx.clone();
        let progress_ctx = ctx.clone();
        let report = move |fraction: f32| {
            let _ = progress_tx.send(UpdateMsg::Progress(fraction));
            progress_ctx.request_repaint();
        };
        let msg = match download_and_verify(&release, report) {
            Ok(path) => UpdateMsg::Ready(path),
            Err(e) => UpdateMsg::Failed(e),
        };
        let _ = tx.send(msg);
        ctx.request_repaint();
    });
    rx
}

fn download_and_verify(release: &AppRelease, report: impl Fn(f32)) -> Result<PathBuf, String> {
    let installer = release
        .installer
        .as_ref()
        .ok_or("esta release não tem instalador")?;
    let sums = agent(Duration::from_secs(30))
        .get(&installer.sums_url)
        .call()
        .and_then(|mut r| r.body_mut().read_to_string())
        .map_err(|e| format!("não deu para baixar o SHA256SUMS: {e}"))?;
    let expected = expected_sha256(&sums, &installer.name)
        .ok_or("a release não tem checksum do instalador")?;

    let mut resp = agent(Duration::from_secs(20 * 60))
        .get(&installer.url)
        .call()
        .map_err(|e| format!("não deu para baixar o instalador: {e}"))?;
    let total = resp.body().content_length();
    let path = std::env::temp_dir().join(&installer.name);
    let mut file =
        File::create(&path).map_err(|e| format!("não deu para salvar o instalador: {e}"))?;
    let mut reader = resp.body_mut().as_reader();
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let (mut done, mut last_pct) = (0u64, 0u64);
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| format!("o download foi interrompido: {e}"))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n])
            .map_err(|e| format!("não deu para salvar o instalador: {e}"))?;
        done += n as u64;
        if let Some(total) = total.filter(|t| *t > 0) {
            let pct = done * 100 / total;
            if pct > last_pct {
                last_pct = pct;
                report(done as f32 / total as f32);
            }
        }
    }
    drop(file);

    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected {
        let _ = std::fs::remove_file(&path);
        return Err(
            "o checksum do instalador não confere com a release; nada foi instalado".into(),
        );
    }
    Ok(path)
}

/// O SHA-256 de `file_name` num SHA256SUMS (`<hash>  <nome>` por linha; `*nome` no modo binário).
fn expected_sha256(sums: &str, file_name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == file_name
            && hash.len() == 64
            && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| hash.to_ascii_lowercase())
    })
}

/// Roda o instalador em silêncio. Ele fecha o app se ainda estiver aberto, atualiza e abre a versão
/// nova; quem chama deve fechar o app logo em seguida.
pub fn run_installer(path: &Path) -> std::io::Result<()> {
    std::process::Command::new(path)
        .args([
            "/SP-",
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NORESTART",
            "/CLOSEAPPLICATIONS",
            "/RESTARTAPP=1",
        ])
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://github.com/macedo/ampero-re/releases/download/v0.3.0/";

    fn release_json(installer_url: &str) -> serde_json::Value {
        serde_json::json!({
            "tag_name": "v0.3.0",
            "html_url": "https://github.com/macedo/ampero-re/releases/tag/v0.3.0",
            "assets": [
                {"name": "SHA256SUMS", "browser_download_url": format!("{BASE}SHA256SUMS")},
                {"name": "ampero-0.3.0-linux-x64.tar.gz", "browser_download_url": format!("{BASE}ampero-0.3.0-linux-x64.tar.gz")},
                {"name": "ampero-0.3.0-windows-x64-setup.exe", "browser_download_url": installer_url}
            ]
        })
    }

    #[test]
    fn parses_the_latest_release_and_its_installer() {
        let url = format!("{BASE}ampero-0.3.0-windows-x64-setup.exe");
        let r = parse_release(&release_json(&url)).unwrap();
        assert_eq!(r.version, "0.3.0");
        assert!(r.notes_url.ends_with("/tag/v0.3.0"));
        let i = r.installer.unwrap();
        assert_eq!(
            (i.name.as_str(), i.url.as_str()),
            ("ampero-0.3.0-windows-x64-setup.exe", url.as_str())
        );
        assert!(i.sums_url.ends_with("/v0.3.0/SHA256SUMS"));
    }

    #[test]
    fn installer_from_elsewhere_or_missing_only_links_to_the_release() {
        let r = parse_release(&release_json(
            "https://example.com/ampero-0.3.0-windows-x64-setup.exe",
        ))
        .unwrap();
        assert_eq!((r.version.as_str(), r.installer), ("0.3.0", None));

        // releases antigas (v0.1.0) só têm zip/tar.gz
        let mut old = release_json("x");
        old["assets"].as_array_mut().unwrap().pop();
        assert_eq!(parse_release(&old).unwrap().installer, None);
        assert!(!parse_release(&old).unwrap().can_self_update());
    }

    #[test]
    fn reads_the_installer_checksum() {
        let hash = "22eb60286e78e86f2a4d4a8e95ba8a7bea33a8bf95d6e27961019d655d7ea462";
        let sums = format!(
            "{}  ampero-0.3.0-linux-x64.tar.gz\n{}  ampero-0.3.0-windows-x64-setup.exe\n",
            "0".repeat(64),
            hash.to_uppercase()
        );
        assert_eq!(
            expected_sha256(&sums, "ampero-0.3.0-windows-x64-setup.exe").as_deref(),
            Some(hash)
        );
        assert_eq!(
            expected_sha256(&format!("{hash} *a.exe"), "a.exe").as_deref(),
            Some(hash)
        );
        assert_eq!(
            expected_sha256(&sums, "ampero-9.9.9-windows-x64-setup.exe"),
            None
        );
        assert_eq!(expected_sha256("nao-e-hash  a.exe", "a.exe"), None);
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }
}
