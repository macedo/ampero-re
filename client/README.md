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

- No Windows, instale com `ampero-X.Y.Z-windows-x64-setup.exe` da página de releases (por usuário, sem admin).
- Menu ⚙ no canto superior direito: tema (seguir o sistema, claro ou escuro, salvo em `gui.json`), verificar
  atualizações, sobre e versão.
- Ao abrir, procura uma versão nova nas releases do GitHub. Se houver, uma faixa mostra as novidades e, na cópia
  instalada pelo instalador, **Atualizar e reiniciar** (baixa, confere o SHA-256 e reabre na versão nova). Em
  cópias do zip, no macOS e no Linux, o botão leva à página da release (detalhes em [`../RELEASING.md`](../RELEASING.md)).

- Lista de presets (User / Factory) com busca. Clicar seleciona o preset no aparelho.
- Cadeia de sinal na ordem real do preset, numa faixa horizontal de cards: tipo do bloco e botão ON/OFF
  (CC 48+bloco) em cima, miniatura do efeito no meio (esmaecida quando desligado) e nome embaixo. Quando não
  cabe, a roda do mouse sobre a faixa rola os pedais na horizontal.
- Editor do bloco selecionado: troca de algoritmo e painel híbrido de parâmetros (nomes, faixas e opções vindos de
  `assets/params.json`):
  - controles numa placa centralizada (na horizontal e na vertical) ao lado da foto; os knobs crescem quando são
    poucos (até 3) e ficam compactos quando são muitos (7 ou mais);
  - knobs para contínuos: arrastar na vertical (Shift = ajuste fino), scroll, duplo clique volta ao padrão do efeito,
    clique no número para digitar o valor na unidade exibida (ex.: `856 ms`, `1.2k`, `+6`; Enter confirma, Esc cancela);
    o arco mostra o valor e um ponto na borda marca o padrão;
  - faders verticais para as bandas de EQ, com linhas horizontais de referência (+50/+25/0/−25/−50): arrasto
    relativo em meia velocidade (Shift = 5× mais fino), scroll ou setas ↑/↓ de 1 em 1, duplo clique = padrão,
    clique no número para digitar;
  - botões segmentados para opções de até 6 itens e para on/off (listas longas, como Mic Type, ficam em dropdown);
  - valores com unidade real (ms, Hz/kHz, dB, semitons) e um ponto dourado quando difere do preset salvo.
- Ao selecionar um efeito, aparecem a foto e a descrição dele, como no editor oficial. Elas são lidas em tempo de
  execução da instalação do Ampero Editor (não são redistribuídas aqui). A pasta é detectada sozinha no Windows
  (`Program FilesHotoneAmpero Editor`) e no macOS (`/Applications`); em outros casos, defina
  `AMPERO_RESOURCES` com o caminho da pasta `Resource/Ampero`. Sem ela, a GUI funciona normalmente, só sem fotos.
- Edições vão ao vivo para o buffer de edição. "não salvo" aparece quando o buffer difere do preset gravado.
  Liga/desliga feito pela pedaleira (CTRL, footswitch) é uso ao vivo e não conta como edição.
- Salvar (só no banco de usuário) pede confirmação.
- Mudanças feitas na pedaleira (CTRL, footswitches, knobs, pedal de expressão, troca de preset) aparecem na tela
  em tempo real. A barra superior mostra o último evento recebido, e o cabeçalho mostra o estado do CTRL.

Testado no Windows contra o aparelho: seleção, edição de parâmetro, troca de algoritmo, ON/OFF e salvar,
sempre conferidos relendo o buffer ou ressincronizando.

### Assistente de timbre (IA)

Painel à direita, recolhível (começa recolhido: « na faixa da direita abre, » recolhe), para conversar com um agente que monta timbres de
músicas, bandas ou estilos usando **só o que o Ampero tem**:

- o agente consulta o catálogo por ferramentas (`listar_algoritmos`, `detalhar_algoritmo`, `preset_atual`) em vez de
  receber os 435 algoritmos no prompt; as descrições da instalação oficial ajudam a ligar os nomes aos equipamentos
  reais ("Marshell 800" = Marshall JCM800);
- ele propõe o timbre com `propor_timbre`; a proposta é validada (algoritmo existe, parâmetro existe, valor dentro da
  faixa) e os erros voltam ao agente para correção;
- a proposta aparece como um card com **Aplicar no buffer**: troca algoritmos, liga/desliga blocos e ajusta os
  parâmetros no buffer de edição. **Nada é salvo** sem você clicar em Salvar.

Configuração ("Configurar" no painel), salva em `%APPDATA%\ampero\assistant.json` (Windows),
`~/Library/Application Support/ampero/` (macOS) ou `~/.config/ampero/` (Linux):

| Provedor | Padrão | Chave |
|---|---|---|
| Claude (Anthropic) | `https://api.anthropic.com`, `claude-opus-5-5`, esforço `medium` | `ANTHROPIC_API_KEY` ou campo na configuração |
| ChatGPT (OpenAI) | `https://api.openai.com/v1` (formato chat/completions) | `OPENAI_API_KEY` ou campo na configuração |
| Outro compatível | `http://localhost:11434/v1` (Ollama), LM Studio, OpenRouter… | opcional |

- O modelo é escolhido num seletor: começa com os modelos conhecidos do provedor, **Buscar na API** lista os que a
  sua chave pode usar (`GET /v1/models` na Anthropic, `GET /models` no formato OpenAI) e **Outro…** aceita
  qualquer nome digitado (útil no Ollama/LM Studio).
- O prompt de sistema é editável (com "Restaurar padrão").
- No Claude, o **fallback automático** do servidor vem ligado: se um classificador de segurança recusar o pedido, a
  Anthropic refaz a resposta em outro modelo (`fallbacks: "default"`). Dá para desligar na configuração.
- As chaves são das APIs (cobrança por uso), não das assinaturas dos chats. Se salvas na configuração, ficam em
  texto puro no seu computador; deixe o campo vazio para usar a variável de ambiente.
- Trocar provedor, modelo ou prompt começa uma conversa nova.

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
ampero monitor 60                  # mostra tudo que a pedaleira envia por 60 s
ampero diff                        # buffer de edição vs preset salvo, byte a byte
ampero info AMP "Marshell 800"     # foto e descrição de um efeito (da instalação oficial)
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
| `src/assistant.rs` | assistente de timbre: configuração, ferramentas do agente, validação de propostas, Claude e chat/completions |
| `src/bin/ampero-gui/` | interface egui; `worker.rs` é a thread dona da conexão MIDI |
| `tests/fixtures/` | trecho real de captura usado nos testes |
