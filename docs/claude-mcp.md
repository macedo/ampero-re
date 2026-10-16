# Montar timbres com o Claude

O app do Ampero pode ser controlado pelo **Claude Desktop** (ou pelo **Claude Code**): você pede "monta o timbre
do solo de Comfortably Numb", e o Claude consulta o que a sua pedaleira tem, escolhe os efeitos e aplica o timbre
no Ampero. Funciona com a sua conta do Claude, **sem chave de API**, usando o padrão
[MCP](https://modelcontextprotocol.io) (Model Context Protocol).

## O que você precisa

- O Ampero ligado no computador pelo USB.
- O app do Ampero instalado (`ampero-X.Y.Z-windows-x64-setup.exe` da página de
  [Releases](https://github.com/macedo/ampero-re/releases); no macOS e no Linux, o pacote `.tar.gz`).
- O [Claude Desktop](https://claude.ai/download) instalado e com login feito.
- O **Ampero Editor oficial fechado**: no Windows só um programa por vez consegue falar com a pedaleira.

## Conectar (uma vez só)

1. **Abra o app do Ampero** e espere ele conectar e sincronizar com a pedaleira.
2. **Ligue o servidor MCP:** clique no ⚙ (canto superior direito) e depois em **Conectar ao Claude…**. Na janela,
   clique em **Ligar**. A bolinha **MCP** na barra superior fica **verde**.
3. **Configure o Claude Desktop:** na mesma janela, clique em **Configurar o Claude Desktop**. O app acrescenta o
   Ampero na configuração do Claude Desktop (`claude_desktop_config.json`), guarda um backup do arquivo anterior e
   não mexe no resto.
4. **Reinicie o Claude Desktop de verdade:** fechar a janela não basta. Clique com o botão direito no ícone do
   Claude na bandeja do sistema (perto do relógio), escolha **Sair** e abra de novo.
5. **Confira:** numa conversa nova do Claude Desktop, o menu de ferramentas/conectores deve listar o **ampero** com
   5 ferramentas: `listar_algoritmos`, `detalhar_algoritmo`, `preset_atual`, `aplicar_timbre` e `salvar_preset`.

O servidor fica ligado nas próximas vezes que você abrir o app. Para desligar, use a mesma janela.

## Usar

Com o app do Ampero aberto (bolinha verde), converse normalmente com o Claude. Exemplos:

- "Monta no Ampero o timbre do solo de Comfortably Numb."
- "Quero a base de Back in Black. Minha guitarra é uma SG com humbuckers."
- "Deixa o timbre atual mais encorpado nas bases."
- "Explica o que cada bloco do preset atual está fazendo."
- "Monta a base de Comfortably Numb e deixa o solo no CTRL."

Também há **prompts prontos**, no botão **+** da caixa de mensagem do Claude Desktop, dentro de **ampero**:

| Prompt | Para quê | Campos |
|---|---|---|
| Montar timbre no Ampero | timbre de uma música, banda ou estilo | música (obrigatório), trecho e guitarra (opcionais) |
| Ajustar o timbre atual | mudar o preset que está carregado | o que mudar |
| Explicar o preset atual | entender o preset, com até três sugestões | nenhum; não aplica nada |

**Antes de aplicar, o Claude sempre pergunta em qual preset.** Ele diz qual está carregado (ex.: "P29-1 'Meu
Lead'") e espera você confirmar; se o preset tiver alterações não salvas, ele avisa que serão substituídas. Para
usar outro, selecione-o na pedaleira ou no app e diga ao Claude. Isso não depende só dele obedecer: a ferramenta
`aplicar_timbre` exige o preset confirmado e recusa se ele não for o carregado naquele momento. Se você trocar de
preset no meio da conversa, ele precisa confirmar de novo.

Enquanto o Claude usa o Ampero, a bolinha **MCP** pisca. Quando ele aplica um timbre, a barra do app mostra
"Claude aplicou … no buffer" e a tela já exibe os blocos e valores novos.

### O que o Claude pode e não pode fazer

- **Pode:** consultar os efeitos, amplificadores e caixas que o Ampero tem, ler o preset carregado e mudar
  algoritmos, ligar e desligar blocos e ajustar parâmetros **no buffer de edição**. Também escolhe quais blocos cada
  CTRL (footswitch) liga e desliga, por exemplo para deixar o boost e o delay do solo no seu CTRL.
- **Salva só quando você pede**, no preset que você confirmar (veja abaixo).
- **Não pode:** trocar de preset, gravar no banco de fábrica, apagar nada nem mexer no firmware. Tudo o que ele aplica é
  validado antes (algoritmo existe, parâmetro existe, valor dentro da faixa); se algo estiver errado, nada vai
  para a pedaleira e ele corrige.

### Gostou do timbre? Salve

O que o Claude aplica fica só no buffer de edição: aparece "não salvo" no app. Para guardar, há três jeitos:

- **Pedir ao Claude:** "salva" ou "salva com o nome Numb Solo". Ele lê o preset carregado e pergunta, com todas as
  letras, algo como "Salvar em P29-1 'Meu Lead'? Isso substitui o que está gravado nele." Só grava depois do seu
  sim. A ferramenta `salvar_preset` também recusa se o preset não for o carregado ou for do banco de fábrica, e o
  nome aceita até 16 caracteres, sem acentos.
- **No app:** botão **Salvar** (pede confirmação). Para renomear, clique no nome do preset no cabeçalho, digite e
  aperte Enter (Esc cancela); o nome novo vai junto no próximo Salvar.
- **Na pedaleira**, como sempre.

Se trocar de preset sem salvar, o timbre se perde. Salvar grava por cima do que estava no preset; o app não tem
"desfazer", então confirme o slot com calma.

## Problemas comuns

| Sintoma | O que fazer |
|---|---|
| Bolinha **vermelha** | O servidor está desligado: ⚙ → Conectar ao Claude… → **Ligar**. |
| Bolinha **laranja** | Erro ao ligar, quase sempre porta ocupada (outro app do Ampero aberto). Feche a outra cópia e clique em Desligar e Ligar. Para usar outra porta, mude `mcp_port` em `gui.json` e configure o Claude Desktop de novo. |
| O Claude diz que a GUI do Ampero não está aberta | Abra o app do Ampero e confira se a bolinha está verde. |
| O Claude diz que a pedaleira não está conectada | Ligue o Ampero no USB, feche o Ampero Editor oficial e clique em **Reconectar** no app. |
| O ampero não aparece no Claude Desktop | Saia do Claude Desktop pela bandeja e abra de novo. Se ainda não aparecer, confira o arquivo de configuração (abaixo). |
| "O token não confere" | Desligue e ligue o servidor na janela Conectar ao Claude. |

## Configurar à mão

A janela **Conectar ao Claude** mostra o trecho pronto para copiar, já com o caminho certo. Ele vai no arquivo
de configuração do Claude Desktop (no app: Configurações > Desenvolvedor > Editar config):

```json
{
  "mcpServers": {
    "ampero": {
      "command": "C:\\Users\\voce\\AppData\\Local\\Programs\\Ampero\\ampero.exe",
      "args": ["mcp"]
    }
  }
}
```

Onde fica o arquivo:

- **Windows:** `%APPDATA%\Claude\claude_desktop_config.json`. Na versão da Microsoft Store, também há uma cópia em
  `%LOCALAPPDATA%\Packages\Claude_…\LocalCache\Roaming\Claude\`, e o botão do app atualiza as duas.
- **macOS:** `~/Library/Application Support/Claude/claude_desktop_config.json`.

### Claude Code

No terminal, com o caminho do `ampero` instalado (a janela Conectar ao Claude mostra o comando pronto):

```bash
claude mcp add ampero -- "C:\Users\voce\AppData\Local\Programs\Ampero\ampero.exe" mcp
```

## Como funciona

```
Claude Desktop ──stdio──▶ ampero mcp ──HTTP 127.0.0.1:47800 + token──▶ app do Ampero ──USB──▶ pedaleira
```

- O Claude Desktop abre o `ampero mcp`, uma ponte pequena que responde ao Claude e repassa as ferramentas para o
  app aberto. Quem fala com a pedaleira é o app, porque no Windows a porta MIDI é de um programa só.
- O servidor só aceita conexões **desta máquina** e com o token guardado em `mcp-token` (na pasta de configuração
  do app: `%APPDATA%\ampero` no Windows). Pedidos vindos de páginas da web são recusados.
- O que o Claude lê (catálogo, preset atual) vai para a conversa com o Claude como qualquer outra mensagem.

## Desconectar

- **Por um tempo:** ⚙ → Conectar ao Claude… → **Desligar** (a bolinha fica vermelha).
- **De vez:** além de desligar, apague o bloco `"ampero"` de `mcpServers` no arquivo de configuração do Claude
  Desktop e reinicie o Claude Desktop.
