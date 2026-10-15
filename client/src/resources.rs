//! Fotos e descrições dos efeitos, lidas em tempo de execução da instalação do editor oficial.
//!
//! Esses arquivos são da Hotone e não são redistribuídos com este projeto: o usuário precisa ter o
//! Ampero Editor instalado (ou apontar `AMPERO_RESOURCES` para a pasta `Resource/Ampero` dele).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Resources {
    pub root: PathBuf,
    descriptions: HashMap<(String, String), String>,
}

impl Resources {
    /// Procura a pasta `Resource/Ampero` do editor oficial nos lugares habituais de cada SO.
    pub fn locate() -> Option<Resources> {
        candidates().into_iter().find_map(|p| Resources::open(&p))
    }

    /// Abre uma pasta `Resource/Ampero` (a que contém `Image/` e `File/`).
    pub fn open(root: &Path) -> Option<Resources> {
        if !root.join("Image").is_dir() {
            return None;
        }
        let descriptions = std::fs::read_to_string(root.join("File").join("description_en.xml"))
            .map(|xml| parse_descriptions(&xml))
            .unwrap_or_default();
        Some(Resources {
            root: root.to_path_buf(),
            descriptions,
        })
    }

    /// PNG do efeito (`Image/<MÓDULO>/<nome>.png`), se existir.
    pub fn image_path(&self, module: &str, name: &str) -> Option<PathBuf> {
        let p = self
            .root
            .join("Image")
            .join(module)
            .join(format!("{name}.png"));
        p.is_file().then_some(p)
    }

    pub fn description(&self, module: &str, name: &str) -> Option<&str> {
        self.descriptions
            .get(&(module.to_string(), normalize(name)))
            .map(String::as_str)
    }

    pub fn description_count(&self) -> usize {
        self.descriptions.len()
    }
}

fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = std::env::var_os("AMPERO_RESOURCES") {
        v.push(PathBuf::from(p));
    }
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(pf) = std::env::var_os(var) {
            v.push(
                Path::new(&pf)
                    .join("Hotone")
                    .join("Ampero Editor")
                    .join("Resource")
                    .join("Ampero"),
            );
        }
    }
    // macOS: o recurso fica dentro do bundle do app
    if let Ok(apps) = std::fs::read_dir("/Applications") {
        for app in apps.flatten().map(|e| e.path()) {
            if app
                .file_name()
                .is_some_and(|n| n.to_string_lossy().contains("Ampero"))
            {
                v.push(app.join("Contents/Resources/Resource/Ampero"));
                v.push(app.join("Contents/Resources/Ampero"));
            }
        }
    }
    // cópia local do editor ao lado do repositório (app/, usada na engenharia reversa)
    if let Ok(exe) = std::env::current_exe() {
        for dir in exe.ancestors().skip(1).take(5) {
            v.push(dir.join("app").join("Resource").join("Ampero"));
        }
    }
    v
}

/// Nomes comparados sem diferença de caixa/pontuação ("Dr. 38 Clean" ≡ "dr38clean").
fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Lê `<Catalog Name="MOD"> <Algorithm Name="…" description="…"> …` sem depender de um parser XML.
fn parse_descriptions(xml: &str) -> HashMap<(String, String), String> {
    let mut out = HashMap::new();
    let mut module = String::new();
    let mut rest = xml;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        let Some(end) = tag_end(rest) else { break };
        let tag = &rest[..end];
        rest = &rest[end + 1..];
        let (name, body) = tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
        let attrs = parse_attrs(body);
        match name {
            "Catalog" => module = attrs.get("Name").cloned().unwrap_or_default(),
            "Algorithm" => {
                if let (Some(n), Some(d)) = (attrs.get("Name"), attrs.get("description")) {
                    out.insert((module.clone(), normalize(n)), d.trim().to_string());
                }
            }
            _ => {}
        }
    }
    out
}

/// Posição do `>` que fecha a tag, ignorando os que aparecem dentro de valores entre aspas.
fn tag_end(s: &str) -> Option<usize> {
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '>' if !quoted => return Some(i),
            _ => {}
        }
    }
    None
}

fn parse_attrs(s: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = s;
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq]
            .trim()
            .rsplit(char::is_whitespace)
            .next()
            .unwrap_or("")
            .to_string();
        let after = rest[eq + 1..].trim_start();
        let Some(after) = after.strip_prefix('"') else {
            break;
        };
        let Some(close) = after.find('"') else { break };
        out.insert(key, unescape(&after[..close]));
        rest = &after[close + 1..];
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_descriptions_with_odd_spacing_and_entities() {
        let xml = r#"<Modules>
            <Catalog Name="AMP" >
                <Algorithm Name="Dr. 38 Clean" description="Based on the &quot;Dr.&quot; amp.
Controls: Gain &amp; Tone"
                ID="9"></Algorithm>
            </Catalog>
            <Catalog Name="FX1"><Algorithm Name="Comprosso" description="Holy Grail > all" ID="0"/></Catalog>
        </Modules>"#;
        let d = parse_descriptions(xml);
        assert_eq!(
            d.get(&("AMP".into(), normalize("Dr. 38 Clean")))
                .map(String::as_str),
            Some("Based on the \"Dr.\" amp.\nControls: Gain & Tone")
        );
        assert_eq!(
            d.get(&("FX1".into(), "comprosso".into()))
                .map(String::as_str),
            Some("Holy Grail > all")
        );
    }
}
