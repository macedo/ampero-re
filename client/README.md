# ampero — cliente multiplataforma para o Hotone Ampero

Protótipo em Rust que fala o protocolo SysEx do Ampero, documentado em [`../PROTOCOLO.md`](../PROTOCOLO.md).
MIDI via [`midir`](https://crates.io/crates/midir): WinMM no Windows, CoreMIDI no macOS e ALSA no Linux.

## Compilar

```bash
cargo build --release                  # só a CLI (ampero)
cargo build --release --features gui   # CLI + interface gráfica (ampero-gui)
```

- **Linux:** precisa dos cabeçalhos do ALSA (`libasound2-dev`). Para a GUI, também
  `libxkbcommon-dev`, `libxcb-render0-dev`, `libxcb-shape0-dev`, `libxcb-xfixes0-dev` e drivers OpenGL.
- **macOS / Windows:** nada além do Rust.

## Interface gráfica (`ampero-gui`)

egui/eframe, um binário nativo por SO. Ao abrir, conecta e sincroniza sozinha (~2,5 s).

- Lista de presets (User / Factory) com busca. Clicar seleciona o preset no aparelho.
- Cadeia de sinal na ordem real do preset, com botão ON/OFF por bloco (CC 48+bloco).
- Editor do bloco selecionado: troca de algoritmo e todos os parâmetros, com nomes, faixas, unidades e opções
  vindos de `assets/params.json`.
- Edições vão ao vivo para o buffer de edição. "não salvo" aparece quando o buffer difere do preset gravado.
- Salvar (só no banco de usuário) pede confirmação.
- Mudanças feitas no hardware (knobs, pedal de expressão) aparecem na tela em tempo real.

Testado no Windows contra o aparelho: seleção, edição de parâmetro, troca de algoritmo, ON/OFF e salvar,
sempre conferidos relendo o buffer ou ressincronizando.

`assets/params.json` é gerado do `algorithm.xml` da instalação oficial:
`deno run --allow-read --allow-write ../tools/gen_params.ts <algorithm.xml> assets/params.json`.

Feche o Ampero Editor oficial antes de usar: no Windows a porta MIDI do aparelho é exclusiva.

## Uso

```bash
ampero ports                       # lista portas MIDI
ampero sync --json presets.json    # lê nomes + 198 presets e mede o tempo
ampero show P29-1                  # lê um preset do banco de usuário
ampero current                     # buffer de edição (inclui alterações não salvas)
ampero select P29-1                # seleciona preset
ampero set AMP 0 20                # Gain do AMP = 20 (só no buffer de edição, não salva)
ampero decode-log LOG --json x     # decodifica um log do proxy winmm, sem aparelho
```

## Desempenho (Windows, Ampero via USB, 2026-10-15)

| Operação | Editor oficial 1.6.0 | `ampero` |
|---|---|---|
| Sync completa (nomes + 198 presets) | 11,8 s | **3,6 s** |
| Ler 1 preset | 47 ms | **14 ms** |
| Tempo por pedido | ~9 ms (2 ms do aparelho + 7 ms do app) | **2,6 ms** |

O editor oficial perde ~7 ms por pedido antes de responder ao aparelho. O `ampero` responde na hora,
então o limite passa a ser a latência do próprio aparelho. Os dados lidos foram comparados com a captura
do editor oficial e são idênticos nos 198 presets.

## Estrutura

| Arquivo | Conteúdo |
|---|---|
| `src/protocol.rs` | frames, nibbles, checksums, construtores de mensagens |
| `src/preset.rs` | decodificação dos 459 bytes do preset |
| `src/device.rs` | conexão MIDI, request/response com timeout e retry, sync em chunks |
| `src/logfile.rs` | leitura dos logs do proxy, usada em testes e na decodificação offline |
| `src/params.rs` | metadados de parâmetros (nomes, faixas, opções) |
| `src/bin/ampero-gui/` | interface egui; `worker.rs` é a thread dona da conexão MIDI |
| `tests/fixtures/` | trecho real de captura usado nos testes |
