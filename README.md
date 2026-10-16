# ampero-re

Engenharia reversa do protocolo USB-MIDI do **Hotone Ampero** e um editor multiplataforma mais rápido que o oficial.

Baixe a versão mais recente em [Releases](https://github.com/macedo/ampero-re/releases) (Windows, Linux e macOS).

**Novo: monte timbres com o Claude.** Peça "monta o timbre do solo de Comfortably Numb" no Claude Desktop e ele
escolhe os efeitos e aplica no Ampero, com a sua conta e sem chave de API. Veja
[como conectar e usar](docs/claude-mcp.md).

| Pasta / arquivo | Conteúdo |
|---|---|
| [`PROTOCOLO.md`](PROTOCOLO.md) | documentação do protocolo: frames SysEx, checksums, comandos e estrutura do preset |
| [`client/`](client/) | cliente em Rust: CLI `ampero` e GUI `ampero-gui` (egui) para Windows, macOS e Linux |
| [`proxy/`](proxy/) | proxy `winmm.dll` (Windows, 32 bits) que registra todo o tráfego MIDI do editor oficial |
| [`tools/`](tools/) | scripts de análise: extração/decodificação de presets dos logs e geração de `params.json` |
| [`docs/claude-mcp.md`](docs/claude-mcp.md) | guia: conectar o Claude Desktop/Claude Code e montar timbres por MCP |
| [`RELEASING.md`](RELEASING.md) | fluxo de PRs e como lançar uma versão (tag → release com pacotes para download) |

## Capturar tráfego do editor oficial (Windows)

1. Copie a pasta de instalação do Ampero Editor para `app/` (fica fora do git).
2. Rode `proxy/build.sh` (precisa do GCC i686 do WinLibs); ele instala `winmm.dll` e `winmm_real.dll` em `app/`.
3. Abra `app/Ampero Editor.exe` com `AMPERO_LOG_DIR` apontando para `logs/`.

Cada mensagem enviada ou recebida vira uma linha `ms DIREÇÃO handle len: bytes` em `logs/ampero_midi_*.log`.
