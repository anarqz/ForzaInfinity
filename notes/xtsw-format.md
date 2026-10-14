# Formato `bin/xtsw` (Xls web services) — o que foi lido do executável

Fonte: `forza_durango_release_final.exe` (base `0x140000000`). Decompilações em `notes/decomp-game/`
(Ghidra headless, projeto `C:\Users\alcmo\FH2_RE_game`, importado **sem** auto-análise; o script
`notes/ghidra-scripts/Decomp.java` cria e decompila funções sob demanda em segundos).

## Descritores de tipo

- Cada tipo é registrado por um inicializador estático:
  `lea rdx,[descritores]; lea rcx,[célula .data]; mov r8d,n; jmp 0x140ee4a00`.
  `0x140ee4a00` só grava `{descritores, n}` na célula. São 579 registros.
- `descritores` = `n` entradas de campo de `0x60` bytes, seguidas de uma entrada do próprio tipo.
- Entrada (qwords):

| off | conteúdo |
|---|---|
| +0x00 | kind |
| +0x08 | nome (char*) |
| +0x10 | offset do campo na struct em memória (`0x7fffffff` = fim/pop) |
| +0x30 | offset do campo de capacidade, ou `0x7fffffff` (capacidade = tamanho fixo) |
| +0x38 | offset onde gravar o count/len (arrays, blobs), ou `0x7fffffff` |
| +0x40 | tamanho em memória |
| +0x48 | célula do tipo (structs) |
| +0x50 | entrada do tipo do elemento (arrays) |
| +0x58 | flags: bit0 ponteiro, bit1 alocar, bit2 count indireto |

## Kinds (leitor genérico `0x140ee5880` → `0x140ee5c30` e afins)

| kind | nome | fio |
|---|---|---|
| 0x00 | u8 | 1 byte |
| 0x01/0x02 | i16/u16 | 2 BE |
| 0x03/0x04 | i32/u32 | 4 BE |
| 0x05/0x06 | i64/u64 | 8 BE |
| 0x07 | f32 | 4 BE |
| 0x09 | string (WCHAR em memória) | u32 BE nº de bytes (≤0x2000) + UTF-8 |
| 0x0c | struct | campos em ordem, sem tag |
| 0x0e | array | u32 BE count (≤0x2000 e ≤ capacidade) + elementos |
| 0x10 | datetime | string `YYYY-MM-DD[THH:MM:SS[.mmm]][Z\|±hh:mm]` → SYSTEMTIME; data inválida = erro `0x8050005c` na resposta toda |
| 0x11 | bool | 1 byte |
| 0x12 | guid | Data1 u32 BE, Data2 u16 BE, Data3 u16 BE, Data4 8 bytes |
| 0x13 | f64 | 8 BE (campos `score`, `Average`) |
| 0x14 | blob | u32 BE len (≤0x80000 e ≤ capacidade) + bytes |
| 0x08, 0x0b, 0x0f | — | **não** aceitos pelo leitor xtsw (aparecem só em tipos Live/XboxMusic/Thunderhead) |

Array com count acima da capacidade: o leitor para sem avançar e o resto da resposta fica sem ler.

## Requisição

`POST /Services.FH2Release/o.xtsw`, corpo = struct `_Parameters` serializada. O `User.Logon` de 111 bytes
bate byte a byte com `UserLogonData` (17 campos).

## Resposta (`0x141b7e080` → `0x140efcb80` → `0x140efc9e0` → `0x140efcca0`)

1. HTTP 200..206. Fora disso: HRESULT `0x80514000 + status` (100..599), `0x80514258` (outros) ou `0x80500010` (status 0).
2. Cabeçalho de 12 bytes, três u32 BE (`ntohl`): `[status][code][msgLen]`.
   - status 0: sucesso, `code`/`msgLen` ignorados, o `_Results` vem logo após os 12 bytes.
   - status 2: erro `0x80510000 | code`; lê `msgLen` bytes de mensagem e loga.
   - outro: erro `0x8050000e`.
3. `_Results` desserializado pelo esquema. Compressão só se o stub pedir (`param_10` de `0x140efc200`);
   no `Logon` é 0.

## Stubs

`0x140efc200(obj, className, method, mode, body, resultsCell, out, outSize, ?, compressed)`.
Stub do Logon: `0x140f15010`. Os nomes de classe vêm como `"Forza.WebServices.User, Forza.WebServices"`;
existe também `Xls.WebServices.User` (outra classe, outros métodos).

## Pendente

- Como o jogo usa `permissions` (MethodPermission: methodId/tier/isEnabled/access). Hoje mandamos vazio.
- `isProfileValid=false` provavelmente leva a `OnlineProfile.UpdateAll*`.
