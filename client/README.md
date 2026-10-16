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
  cabe, a roda do mouse sobre a faixa rola os pedais na horizontal; quando cabe, a faixa fica centralizada. Setas entre
  os cards (e nas pontas, entrada e saída) mostram o caminho do sinal. **Arrastar um card** muda a ordem da cadeia (como o
  "Manage Signal Chain" do editor oficial): uma linha dourada mostra onde ele vai entrar, e a ordem nova vai na hora
  para o buffer de edição (marca "não salvo").
- Editor do bloco selecionado: troca de algoritmo e painel híbrido de parâmetros (nomes, faixas e opções vindos de
  `assets/params.json`):
  - controles numa placa centralizada (na horizontal e na vertical) ao lado da foto; os knobs crescem quando são
    poucos (até 3) e ficam compactos quando são muitos (7 ou mais);
  - knobs para contínuos: arrastar na vertical ou rolar a roda (com Shift, ajuste fino: na roda, 1 unidade por dente), duplo clique volta ao padrão do efeito,
    clique no número para digitar o valor na unidade exibida (ex.: `856 ms`, `1.2k`, `+6`; Enter confirma, Esc cancela);
    o arco mostra o valor e um ponto na borda marca o padrão;
  - faders verticais para as bandas de EQ, com linhas horizontais de referência (+50/+25/0/−25/−50): arrasto
    relativo em meia velocidade (Shift = 5× mais fino), roda ou setas ↑/↓ de 1 em 1 (Shift + roda = 1 por dente), duplo clique = padrão,
    clique no número para digitar;
  - botões segmentados para opções de até 6 itens e para on/off (listas longas, como Mic Type, ficam em dropdown);
  - valores com unidade real (ms, Hz/kHz, dB, semitons) e um ponto dourado quando difere do preset salvo.
- Ao selecionar um efeito, aparecem a foto e a descrição dele, como no editor oficial. Elas são lidas em tempo de
  execução da instalação do Ampero Editor (não são redistribuídas aqui). A pasta é detectada sozinha no Windows
  (`Program FilesHotoneAmpero Editor`) e no macOS (`/Applications`); em outros casos, defina
  `AMPERO_RESOURCES` com o caminho da pasta `Resource/Ampero`. Sem ela, a GUI funciona normalmente, só sem fotos.
- Edições vão ao vivo para o buffer de edição. "não salvo" aparece quando o buffer difere do preset gravado.
  Liga/desliga feito pela pedaleira (CTRL, footswitch) é uso ao vivo e não conta como edição.
- **CTRL:** os cards mostram um selo azul nos blocos que algum CTRL liga e desliga (ex.: "CTRL 4"). O indicador do
  cabeçalho mostra qual CTRL está acionado e, ao clicar, abre a janela do CTRL: para cada CTRL 1–4, a função (só
  leitura) e caixas para marcar os blocos que ele liga e desliga. As mudanças vão na hora para a pedaleira e marcam
  "não salvo".
- No cabeçalho, **Volume** do patch num knob compacto (0–99; arraste para cima/direita ou role a roda; Shift = fino)
  com o número clicável para digitar, e **Tempo** clicável como o nome (40–250 BPM; Enter confirma,
  Esc cancela); os dois vão na hora para o buffer de edição. Tap tempo feito na pedaleira não conta como "não salvo".
- Salvar (só no banco de usuário) pede confirmação. Clicar no nome do preset no cabeçalho abre um campo para renomear
  (até 16 caracteres ASCII; Enter confirma, Esc cancela); o nome novo marca "não salvo" e é gravado no Salvar.
- Mudanças feitas na pedaleira (CTRL, footswitches, knobs, pedal de expressão, troca de preset) aparecem na tela
  em tempo real. A barra superior mostra o último evento recebido, e o cabeçalho mostra o estado do CTRL.

Testado no Windows contra o aparelho: seleção, edição de parâmetro, troca de algoritmo, ON/OFF e salvar,
sempre conferidos relendo o buffer ou ressincronizando.

### Montar timbres com o Claude (MCP)

O Claude Desktop (ou o Claude Code) monta timbres de músicas, bandas ou estilos usando **só o que o Ampero tem**,
por [MCP](https://modelcontextprotocol.io), com a conta do usuário (sem chave de API). Passo a passo para o usuário,
com problemas comuns: [`docs/claude-mcp.md`](../docs/claude-mcp.md).

1. Na GUI, menu ⚙ → **Conectar ao Claude…** → **Ligar**. A GUI aberta passa a atender em `127.0.0.1:47800`
   (porta em `gui.json`); só programas desta máquina com o token de `mcp-token` (na pasta de configuração) entram,
   e requisições de páginas web são recusadas. A barra superior sempre mostra uma bolinha **MCP**: verde ligado,
   vermelho desligado, laranja com erro, piscando enquanto o Claude usa o servidor; clicar nela abre a mesma janela.
2. Na mesma janela, **Configurar o Claude Desktop**: acrescenta `ampero` em `mcpServers` no
   `claude_desktop_config.json` (com backup), apontando para `ampero mcp`. Feche o Claude Desktop pela bandeja e
   abra de novo. No Claude Code: `claude mcp add ampero -- "<pasta do app>/ampero" mcp`.
3. Peça no Claude: "monte no Ampero o timbre do solo de Comfortably Numb".

Ferramentas:

- `listar_algoritmos` e `detalhar_algoritmo` consultam o catálogo em vez de mandar os 435 algoritmos de uma vez; as
  descrições da instalação oficial ajudam a ligar os nomes aos equipamentos reais ("Marshell 800" = Marshall JCM800);
- `preset_atual` lê o buffer de edição da pedaleira;
- `aplicar_timbre` exige o preset confirmado pelo usuário (`preset`, que precisa ser o carregado; o Claude pergunta
  antes, e a ferramenta recusa se não bater), valida tudo (algoritmo existe, parâmetro existe, valor dentro da faixa; os erros voltam ao
  Claude para correção) e só então troca algoritmos, liga/desliga blocos e ajusta parâmetros no buffer de edição.
  Nada é salvo nessa etapa.
- `salvar_preset` grava o buffer no preset carregado, só no banco de usuário e com o rótulo confirmado em `preset` (o
  Claude pergunta "Salvar em P29-1 '…'? Isso substitui o que está gravado"); `nome` opcional renomeia (até 16
  caracteres ASCII).

Prompt padrão: o servidor manda **instruções** no handshake, que o Claude coloca no contexto enquanto o `ampero`
estiver conectado (especialista em timbres, em português; lembrar o equipamento original, consultar o catálogo antes
de escolher, aplicar só o que muda, resumir bloco/algoritmo/valores e dar dicas de execução). E há **prompts
prontos**, no menu **+** do Claude Desktop:

- **Montar timbre no Ampero:** música, banda ou estilo; trecho e guitarra são opcionais;
- **Ajustar o timbre atual:** o que mudar (ex.: "mais encorpado nas bases");
- **Explicar o preset atual:** bloco a bloco, com até três sugestões, sem aplicar nada.

Quem fala com a pedaleira é a GUI, porque no Windows a porta MIDI é exclusiva: `ampero mcp` é só uma ponte stdio
que responde o handshake sozinha e repassa as chamadas. Com a GUI fechada, as ferramentas avisam o Claude para
pedir que você abra o app.

`assets/params.json` é gerado do `algorithm.xml` da instalação oficial:
`deno run --allow-read --allow-write ../tools/gen_params.ts <algorithm.xml> assets/params.json`.

Feche o Ampero Editor oficial antes de usar: no Windows a porta MIDI do aparelho é exclusiva.

## Uso

```bash
ampero ports                       # lista portas MIDI
ampero sync --json presets.json    # lê nomes + 198 presets e mede o tempo
ampero show P29-1                  # lê um preset do banco de usuário
ampero current                     # buffer de edição (inclui alterações não salvas)
ampero dump P33-3                  # os 459 bytes crus do preset em hexa (sem argumento: o buffer de edição)
ampero select P29-1                # seleciona preset
ampero set AMP 0 20                # Gain do AMP = 20 (só no buffer de edição, não salva)
ampero decode-log LOG --json x     # decodifica um log do proxy winmm, sem aparelho
ampero monitor 60                  # mostra tudo que a pedaleira envia por 60 s
ampero mcp                         # ponte MCP (stdio) do Claude Desktop para a GUI aberta
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
| `src/mcp.rs` | servidor MCP: JSON-RPC, HTTP local com token, ponte stdio `ampero mcp`, configuração do Claude Desktop |
| `src/timbre.rs` | ferramentas de timbre do MCP: catálogo, preset atual e validação de timbres |
| `src/bin/ampero-gui/` | interface egui; `worker.rs` é a thread dona da conexão MIDI |
| `tests/fixtures/` | trecho real de captura usado nos testes |
