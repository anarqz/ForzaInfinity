# Desenvolvimento

Como o ForzaInfinity funciona por dentro e como trabalhar nele.

```
jogo ──IXMLHTTPRequest2──▶ azimuth-tls.dll (hook) ──HTTP──▶ fh2-server (Rust)
                                 │                               │
                       FH2_Network.jsonl               captures/*.jsonl + responses/
```

## Partes

| Pasta | O que é |
|---|---|
| `hook/` | Submódulo: fork do ExtraOptions 0.2.34 (branch `fh2online`). C freestanding, só Kernel32, `-Werror`. |
| `server/` | O servidor em Rust (std + SQLite). |
| `web/` | O manager (React + Vite), servido em `/ui/` pelo servidor, só para a própria máquina. |
| `landing/` | A página pública (React + Vite), na branch `landing`. |
| `release/` | Instaladores e modelo do INI que vão no pacote dos jogadores. |
| `tools/` | Extrator do esquema `xtsw`, decodificador/replay de capturas, empacotador da release. |

## Hook

- O jogo faz todo o HTTP por COM: `CoCreateInstance(CLSID_FreeThreadedXMLHTTP60)`. O Azimuth implementa o próprio `CoCreateInstance` (RVA `0x6dbe0`) e, para esse CLSID, devolve um `OfflineRequest`. O hook põe um jump nessa função (depois de conferir o SHA-256 dos primeiros bytes) e devolve o próprio `IXMLHTTPRequest2`.
- O hook também responde o token do usuário do Xbox Live, troca o `DataWriter`/`DataReader` do Azimuth (que perdiam dados), cuida da conta (`azimuth_account.c`), do save na nuvem (`azimuth_cloud.c`, codec próprio em `azimuth_codec.c`), da tela INFINITY (`azimuth_online_menu.c`, `azimuth_ui.c`) e de correções do jogo sob o Azimuth (clipes do Game DVR).
- Build: `python hook/build.py <zig> --azimuth --build-dir build --output build/azimuth-tls.dll` (Zig 0.15.2). Confira que os imports são só KERNEL32 e que o TLS continua igual (template de 336 bytes, alinhamento 16, sem callbacks).
- Testes: `python hook/tests/codec_test.py <zig> <bundles ou .sqlite>` (codec do save) e `python hook/tests/ui_edits_test.py <zig> <Mount>` (edições de XAML; o `.exe` gerado roda no Windows, ou no Linux pelo Wine do Proton dentro do `SteamLinuxRuntime_4/run`).
- **Nova versão do Azimuth:** os endereços do `azimuth.exe` que a rede e a conta usam ficam em `hook/adapters/azimuth_offsets.c`, um bloco por versão. Para achar os da versão nova: `tools/azimuth_map.py <azimuth.exe antigo> <novo> <rva…>` (precisa do `capstone`). Ele casa as funções pelas instruções com os endereços apagados e confere as tabelas pelo ponteiro guardado. Os hashes são o SHA-256 dos primeiros 0x20 bytes de cada função. Depois, ajuste os instaladores (`SUPPORTED_AZIMUTH` / `$SupportedAzimuth`).

A seção `[Network]` do `FH2_ExtraOptions.ini`:

| Mode | Efeito |
|---|---|
| `Off` | Comportamento original do Azimuth. |
| `Capture` | Grava cada requisição em `data/bin/FH2_Network.jsonl` e responde "offline". |
| `Proxy` | Encaminha para `Server=host:porta` ou `Server=https://host[:porta]` (TLS pelo WinHTTP). É o modo dos jogadores. |

## Servidor

O hook reescreve `https://host/caminho` para `http://<Server>/host/caminho` e manda `X-Fh2-Original-Url` e `X-Fh2-Request-Id`. As chamadas Forza (`bin/xtsw`) são decodificadas pelo esquema extraído do exe (`server/schema.json`, `tools/extract_schema.py`) e respondidas pelos handlers em `server/src/forza*.rs`. Sem handler, a resposta sai de `server/responses/`, relido a cada requisição:

1. a primeira regra que casar em `responses/routes.json` (`host`, `method`, `path`, `class`, `xmethod`, glob `*`);
2. `responses/forza/<Classe>/<Metodo>.json`, depois `forza/<Classe>/_default.json`;
3. `responses/hosts/<host>/_default.json`;
4. `responses/_default.json` (503).

```json
{ "status": 200, "headers": { "Content-Type": "application/json" }, "json": { } }
```

`tools/xtsw.py --replay <captura> <servidor>` reenvia as requisições Forza de uma captura e decodifica requisição e resposta.

## Release dos jogadores

`tools/package_release.sh [--server host:port | https://host]` (para a release pública: `--server https://infinity.anarqz.dev`) compila o hook e monta `dist/ForzaInfinity.zip` mais os mesmos arquivos soltos. Anexe tudo de `dist/` a uma release do GitHub: a landing e os instaladores apontam para `releases/latest/download/…`. O `install.ps1` precisa de um teste num Windows antes de cada release.

A release é assinada: o `package_release.sh` chama `tools/sign_release.sh`, que grava `azimuth-tls.dll.sig` (assinatura Ed25519 crua de 64 bytes da linha ASCII `fh2online-update-v1 <SHA-256 do DLL em hex minúsculo>`, sem quebra de linha) e confere com a chave pública embutida em `hook/adapters/azimuth_sign.c`. A chave privada fica só em `~/.config/forzainfinity/release-ed25519.pem` (outra com `FH2_SIGNING_KEY`), nunca no repositório; sem ela o pacote não é montado. O auto-update do hook baixa o `.sig` da mesma release e não instala nada sem assinatura válida (log `UPDATE: signed update` ou `UPDATE: signature does not verify; not installed`). Hooks anteriores a essa mudança ainda atualizam só pelo SHA-256; a partir da primeira atualização passam a exigir a assinatura. Perder a chave obriga a trocar a chave embutida e os jogadores a atualizarem à mão uma vez. Teste: `python3 hook/tests/sign_test.py ~/tools/zig/zig` (vetores do RFC 8032 e uma assinatura feita com a chave real).
