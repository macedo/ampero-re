# ampero-re

Engenharia reversa do protocolo USB-MIDI do **Hotone Ampero** e um editor multiplataforma mais rápido que o oficial.

| Pasta / arquivo | Conteúdo |
|---|---|
| [`PROTOCOLO.md`](PROTOCOLO.md) | documentação do protocolo: frames SysEx, checksums, comandos e estrutura do preset |
| [`client/`](client/) | cliente em Rust: CLI `ampero` e GUI `ampero-gui` (egui) para Windows, macOS e Linux |
| [`proxy/`](proxy/) | proxy `winmm.dll` (Windows, 32 bits) que registra todo o tráfego MIDI do editor oficial |
| [`tools/`](tools/) | scripts de análise: extração/decodificação de presets dos logs e geração de `params.json` |

## Capturar tráfego do editor oficial (Windows)

1. Copie a pasta de instalação do Ampero Editor para `app/` (fica fora do git).
2. Rode `proxy/build.sh` (precisa do GCC i686 do WinLibs); ele instala `winmm.dll` e `winmm_real.dll` em `app/`.
3. Abra `app/Ampero Editor.exe` com `AMPERO_LOG_DIR` apontando para `logs/`.

Cada mensagem enviada ou recebida vira uma linha `ms DIREÇÃO handle len: bytes` em `logs/ampero_midi_*.log`.
