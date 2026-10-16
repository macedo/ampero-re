# Protocolo USB-MIDI do Hotone Ampero (engenharia reversa)

Fonte: Ampero Editor 1.6.0 (JUCE, x86) + captura via proxy `winmm.dll` (`proxy/`).
Dispositivo: USB VID `84EF` / PID `0012`, porta MIDI "Ampero MIDI". Firmware esperado pelo app: 5.2.

## Transporte

- MIDI SysEx puro via WinMM (`midiOutLongMsg` / callback de `midiInOpen`). Nada de HID/bulk.
- Modelo request/response, um pedido por vez (stop-and-wait).
- Também aparecem mensagens curtas (ex.: `B5 3D 63` = CC canal 6) — a investigar.

## Formato do frame

```
F0 21 25 7F 4D 50 2D 64 | OP | G1 G2 | CMD | ARGS... | DATA(nibbles)... | CK_hi CK_lo | F7
 0  1  2  3  4  5  6  7    8    9 10    11    12..
```

| Campo | Valor | Notas |
|---|---|---|
| ID | `F0 21 25 7F 4D 50 2D 64` | `4D 50 2D 64` = ASCII `"MP-d"` (família MP-50/MP-80 = Ampero) |
| OP | `11` = leitura (pedido) · `12` = escrita / resposta | lembra o RQ1/DT1 da Roland |
| G1 G2 | `00 00` (sistema) · `00 02` (presets) | |
| CMD | ver tabela abaixo | resposta geralmente usa CMD+1 ou outro par |
| DATA | **nibbles**: cada byte vira 2 bytes `0H 0L` (alto primeiro) | ex.: `04 0A` = `0x4A` = `'J'` |

### Checksum (rotina `0x58C9F0` no exe)

- **Frames simples** (nomes, etc.): `sum(frame[13 : len-3]) & 0xFF`, gravado como 2 nibbles em
  `frame[len-3], frame[len-2]` (logo antes do `F7`). Pedidos curtos (< 17 bytes) não têm checksum.
- **Preset em chunks** (rotina `0x58DD50`): concatenar `frame[17 : len-1]` de todos os chunks `0E`;
  o checksum é `sum(buf[0 : n-2]) & 0xFF` comparado com `buf[n-2]<<4 | buf[n-1]`. Validado em 198/198 presets.

## Comandos observados (sincronização inicial)

| Pedido (host) | Resposta (aparelho) | Significado (hipótese) |
|---|---|---|
| `11 00 00 05 00` | `12 00 00 05 00 01 00` | handshake / versão? |
| `11 00 00 05 01` | `12 00 00 05 01 01 01 01` | idem, no fim da sync |
| `11 00 02 09 00` | `12 00 02 09 00 ...` (216 B) | estado global / config atual |
| `11 00 02 05 00 NN` | `12 00 02 05 00 NN` + 32 B nibblizados | nome de preset/bloco NN (16 chars ASCII + resto; vazio = `FF`) |
| `11 00 02 07 00 NN` | `12 00 02 07 00 NN` + 32 B | outra lista de nomes (ex.: `"Cenz Celestion "` → IRs/CAB?) |
| `11 00 02 08 00 00 PP` | `12 00 02 0C 00 00 PP 07 18 01 40` | abre leitura do preset PP (cabeçalho: tamanho?) |
| `12 00 02 0D 00 00 PP 01` | `12 00 02 0E 00 00 PP 00 CC` + dados | pede 1º chunk (CC = índice do chunk) |
| `12 00 02 0F 00 00 PP 00 CC 01` | próximo `0E` ou `12 00 02 10 00 00 PP 00 05` (fim) | ACK do chunk CC / pede o próximo |
| `11 00 02 0A 00 00 00` | `12 00 02 12 ...` | abre leitura do **preset atual** (buffer de edição) |
| `12 00 02 13 ...` / `12 00 02 15 ...` | `12 00 02 14 ...` / fim `12 00 02 16` | chunks do preset atual (mesmo esquema de 0D/0E/0F/10) |

Preset completo: 5 chunks (4 × 210 B + 1 × 170 B de frame) → 920 nibbles → **459 bytes** de dados + 1 byte de checksum.
198 presets lidos na sync inicial.

## Estrutura do preset (459 bytes decodificados)

Validado em 198/198 presets: 1769 blocos com algoritmo reconhecido e nenhum switch fora de 0/1.
Também conferido no diff do P29-1 antes e depois de salvar, e contra a UI do editor.
Ferramentas: `tools/extract_presets.awk` (log → hex) e `tools/decode_preset.awk` (hex → texto).

Índice de preset (14 bits, `banco*128 + slot`): **banco 0 = 0..98, banco 1 = 128..226** (os presets da aba
"User Patches"; P29-1 = `01 54` = 212). Slot = `(N-1)*3 + (sub-1)`, igual ao Program Change.

| Offset | Tam. | Campo | Notas |
|---|---|---|---|
| `0x000` | 2 | banco, slot | `01 54` = P29-1 |
| `0x002` | 2 | Patch Volume | u16 BE (45 = `00 2D`) |
| `0x004` | 16 | nome | ASCII, padding `00` |
| `0x014` | 12 | ? | varia entre presets (`01`,`04`,`08`,`0C`) — provavelmente modos de CTRL/footswitch |
| `0x020` | 7 | ? cabeçalho do EXP | |
| `0x027` | 8×7 | **atribuições do EXP** | `[slot][alg_hi][alg_lo][param][min][max][?]`; slot 0 = vazio. P01-1: FX1/Classic PS/param 1 (Position) 0..100 |
| `0x05F` | 9×33 | **blocos** | ver abaixo, na ordem de armazenamento FX1 FX2 AMP NR CAB EQ FX3 DLY RVB |
| `0x188` | 2 | ? | `00 00` |
| `0x18A` | 2 | Patch Tempo (BPM) | u16 BE (`00 78` = 120) |
| `0x18C` | 3×4 | **Quick Knobs** | `[módulo][alg_hi][alg_lo][param]`; módulo = índice do bloco + 3 (FX1=3 … RVB=11), 2 = nível de patch? |
| `0x198` | 2 | ? | flag 0/1 |
| `0x19A` | 9×2 | **ordem da cadeia** | byte par = **posição** de cada bloco, na ordem de armazenamento FX1 FX2 AMP NR CAB EQ FX3 DLY RVB (`00 01 02 04 03 …` = FX1 FX2 AMP CAB NR …; com o FX3 antes do FX2: `00 02 03 05 04 06 01 07 08`). Na ordem padrão ler como "bloco de cada posição" dá o mesmo resultado, o que escondeu isto até 2026-10-16. Byte ímpar quase sempre 0 |
| `0x1AB` | 1 | **estado dos CTRL** | bitmask; uma apertada do CTRL no P01-1 levou de `00` a `08` (e ligou o DLY). É o valor da notificação `02 06 23 VV` |
| `0x1AC` | 1 | Vol Position? | 1 = Pre, 0 = Post bate com P01-1 e P29-1, mas o byte também muda pela notificação `02 06 24 00 VV` (footswitch); pode ser outro estado de CTRL |
| `0x1AD` | 1 | ? | sempre `01` |
| `0x1AE` | 1 | ? | flag 0/1 |
| `0x1AF`–`0x1CA` | 28 | reservado | sempre zero |

### Bloco (33 bytes)

```
+0      switch       00 = off, 01 = on
+1..2   algoritmo    alg_hi alg_lo  (código = BaseIndex + ChildIndex do algorithm.xml; o <Catalog> do XML segue a mesma ordem dos blocos)
+3..32  15 parâmetros, u16 big-endian, na ordem dos IDs de Knob/Combox do XML
```

- Combos guardam o índice da opção (Classic PS Range = 3 → "+2 Oct").
- Valores fora de 0..100 aparecem quando a unidade é física (Delay Time 814 = `03 2E` ms).
- **CAB 144–163 = slots de IR do usuário** (20 slots, os mesmos nomes da lista `02 07`). Os parâmetros 5 e 6 do CAB
  (ex.: `3593`, `101/1062`) ainda são desconhecidos; talvez identifiquem ou façam hash da IR.
- Trocar o algoritmo zera os parâmetros para os `DefaultValue` do XML e também atualiza o algoritmo em
  todas as entradas da tabela do EXP (diff do P29-1: 9 bytes `00 → 03` nos offsets `0x029 + 7k`).
- Ambiguidade: os códigos de algoritmo ≥128 vistos (`01 22`, `01 1B`) servem tanto como "7 bits hi/lo" quanto como
  u16. Hoje o decodificador usa 7 bits, e isso bate com o XML em todos os casos.

## Edição ao vivo (capturado em 2026-10-15, preset P29-1)

Nenhuma dessas mensagens tem checksum nem ACK do aparelho — o editor manda e segue.
Números de 14 bits usam 2 bytes de 7 bits (`hi lo`, valor = `hi*128 + lo`).

### MIDI padrão (canal 6 = `B5`/`C5`)

| Ação | Mensagem | Notas |
|---|---|---|
| Selecionar preset | `B5 00 BB` + `C5 PP` | CC0 = banco, PC = `(N-1)*3 + (sub-1)`; P29-1 → `B5 00 01`, `C5 54` |
| Liga/desliga bloco | `B5 CC 00` / `B5 CC 7F` | CC = 48 + índice de armazenamento: FX1=48 FX2=49 AMP=50 NR=51 CAB=52 EQ=53 FX3=54 DLY=55 RVB=56 (todos capturados) |
| Patch Volume | `B5 07 VV` | CC 7, 0–99 (o editor limita em 99). Capturado em 2026-10-16 no P29-1: 48 = `30`, 45 = `2D`, máx. `63` |
| Patch Tempo | `B5 49 HH` + `B5 4A LL` | CC 73 = 7 bits altos, CC 74 = 7 bits baixos do BPM (40–250): 125 = `00 7D`, 150 = `01 16`, 250 = `01 7A`, 40 = `00 28` |
| Ordem da cadeia | `F0 … 12 00 02 06 0D P0 … P8 F7` | posição (0–8) de cada bloco, na ordem de armazenamento, sem checksum; o mesmo formato do `0x19A`. Ler: `11 00 02 06 0D` → resposta `12 00 02 06 0D P0 … P8`. O editor oficial deixa qualquer ordem (até o CAB antes de tudo). Capturado no "Manage Signal Chain" em 2026-10-16 |
| Outros CCs vistos | `B5 3D 63` | CC61 — a investigar |

### SysEx de edição (`OP=12`, `G1=10`)

```
F0 21 25 7F 4D 50 2D 64  12 10 SS 00 01  AH AL                F7   selecionar algoritmo do slot
F0 21 25 7F 4D 50 2D 64  12 10 SS 00 02  AH AL  PP  VH VL     F7   alterar parâmetro
F0 21 25 7F 4D 50 2D 64  12 10 00 00 00  BB PP  NAME[17]      F7   nome do preset (até 16 chars ASCII + 0x00 de padding)
F0 21 25 7F 4D 50 2D 64  12 00 02 06 07  BB PP                F7   gravar buffer de edição no banco BB / slot PP
```

| Campo | Significado |
|---|---|
| `SS` | índice de armazenamento do bloco + 1: `01`=FX1 `02`=FX2 `03`=AMP `04`=NR `05`=CAB `06`=EQ `07`=FX3 `08`=DLY `09`=RVB (01 e 03 confirmados) |
| `AH AL` | código do algoritmo = `BaseIndex + ChildIndex` do `algorithm.xml` (Comprosso=0, Squeezer=3, Marshell 800=65=`00 41`, Classic PS(FX1)=162=`01 22`) |
| `PP` | índice do parâmetro (ordem dos knobs; Gain=0 … Treble=5 no Marshell 800) |
| `VH VL` | valor (Sustain 39 = `00 27`) |

- Clicar num bloco na UI reenvia "selecionar algoritmo" com o algoritmo atual.
- Salvar não gerou resposta do aparelho.

### Notificações do aparelho (mudanças feitas no hardware)

Capturadas com `ampero monitor` enquanto o usuário usava CTRL, knobs, footswitches e troca de preset
(2026-10-15). O driver MIDI do Windows entrega a entrada a todos os clientes abertos, então o editor
oficial, a GUI e o monitor recebem tudo, inclusive as respostas aos pedidos uns dos outros.

| Mensagem | Significado |
|---|---|
| `12 10 SS 00 02 AH AL PP VH VL` | parâmetro mudou (knob, Quick Knob, pedal de expressão). Mesmo formato da escrita |
| `12 10 SS 00 00 VV` | bloco SS ligou (`7F`) / desligou (`00`). Ex.: CTRL do P01-1 alterna o DLY (`SS=08`) |
| `12 10 SS 00 01 AH AL` | algoritmo trocado (mesmo formato da escrita; ainda não visto vindo do aparelho) |
| `12 00 02 06 05 00 00 TH TL` | tempo atual em BPM (14 bits): `00 78` = 120, `01 16` = 150. Vem junto com **cada** mudança e na troca de preset |
| `12 00 02 06 04 01 xx` | preset trocado na pedaleira (acompanhado de `B6 08 xx`) |
| `12 00 02 06 23 VV` | novo estado dos CTRL = byte `0x1AB` do preset |
| `12 00 02 06 24 00 VV` | outro footswitch; deve espelhar o byte `0x1AC` (0x188+0x24) |

Uma apertada do CTRL no P01-1 gerou `12 10 08 00 00 7F` + `12 00 02 06 23 08`. O `ampero diff` mostrou exatamente
dois bytes diferentes no buffer de edição: `0x146` (switch do DLY) `00→01` e `0x1AB` `00→08`.

A GUI aplica direto as notificações de parâmetro, switch, tempo e CTRL. Para troca de preset e mensagens
desconhecidas, ela relê o buffer de edição (~14 ms), com debounce de 150 ms.

### Hipóteses sobre as listas da sync

- `02 05` (11 entradas, ex. `"JCM 800 CLEAN (4"`) → provavelmente capturas/Sound Clone (NAM) do usuário.
- `02 07` (20 entradas, ex. `"Cenz Celestion "`) → slots de IR do usuário.
- O índice de preset em `02 08` também é de 14 bits (`00 01 62` = 226); a sequência não é contínua — mapear.

## Desempenho do editor oficial (medido)

| Métrica | Valor |
|---|---|
| Resposta do aparelho (OUT→IN) | média **1,98 ms**, máx. 7,3 ms |
| Latência do próprio app (IN→OUT) | média **7,04 ms** |
| 1 preset (5 chunks) | 47 ms |
| 198 presets | **11,8 s**, dos quais ~10 s são o app esperando sem motivo |

Um cliente que responde na hora cai para ~3 s na sync completa (o limite é a latência do aparelho),
sem pipelining. **Confirmado com o protótipo `client/` (Rust): 3,63 s, 2,56 ms por pedido, dados idênticos.**
Com pipelining (se o firmware aceitar), menos ainda.

## Pendências

- [x] Mapear a estrutura dos 459 bytes do preset.
- [ ] Preset: bytes 0x014–0x026 (CTRL/EXP), 0x198, 0x1AE, params 5–6 do CAB; confirmar Vol Position.
- [x] Capturar ações: trocar preset, girar knob, liga/desliga bloco, trocar algoritmo, salvar.
- [x] Capturar: patch volume e patch tempo (CCs 7 e 73/74), ordem da cadeia (`02 06 0D`).
- [ ] Capturar: renomear (só o nome, sem salvar), CAB/IR, global EQ, tuner, CTRL/EXP, import/export.
- [ ] Significado de `07 18 01 40` no cabeçalho `0C` e do payload de `02 09`.
- [ ] Mensagens curtas (CC) enviadas pelo app.
- [ ] Testar se o firmware aceita pedidos em pipeline.
