# FH2Online

Uma bancada para descobrir e reimplementar os serviços online do Forza Horizon 2 rodando no Azimuth 0.9.7. A ideia é reaproveitar o Azimuth, não reescrevê-lo.

```
jogo ──IXMLHTTPRequest2──▶ azimuth-tls.dll (hook) ──HTTP──▶ fh2-server (Rust)
                                 │                               │
                       FH2_Network.jsonl               captures/*.jsonl + responses/
```

## Como funciona

- **`hook/`** é um fork do ExtraOptions 0.2.34, na branch `fh2online`. Todas as opções do ExtraOptions continuam funcionando, e a parte nova fica em `adapters/azimuth_net.c`.
  - O jogo faz todo o HTTP por COM: `CoCreateInstance(CLSID_FreeThreadedXMLHTTP60)`.
  - O Azimuth implementa o próprio `CoCreateInstance` (RVA `0x6dbe0`) e, para esse CLSID, devolve um `OfflineRequest` que falha com `0x800C0005`.
  - O hook coloca um jump nessa função, depois de conferir o SHA-256 dos 32 primeiros bytes. Para o XHR, ele devolve o nosso `IXMLHTTPRequest2`.
  - O MMDeviceEnumerator continua indo para a fábrica do Azimuth. Qualquer outro CLSID recebe `REGDB_E_CLASSNOTREG` e é registrado no log.
- **`server/`** é o servidor de testes. Ele recebe tudo o que o hook encaminha, grava em `captures/`, e responde a partir de arquivos em `responses/`. Esses arquivos são relidos a cada requisição, então dá para editar com o jogo aberto.

## Instalação

1. Rode `install.bat`. Ele:
   - faz backup do `azimuth-tls.dll` original como `azimuth-tls.dll.stock`;
   - copia o novo DLL;
   - cria `data\bin\FH2_ExtraOptions.ini`, a partir do seu INI do ExtraOptions mais a seção `[Network]`;
   - copia o `Gamerpic.png`.
2. Para voltar ao original, rode `uninstall.bat`.

A seção `[Network]` no `FH2_ExtraOptions.ini` (em `azimuth-windows\data\bin`):

| Mode | Efeito |
|---|---|
| `Off` | Comportamento original do Azimuth. |
| `Capture` | Grava cada requisição em `data\bin\FH2_Network.jsonl` e responde "offline", igual ao Azimuth. **Comece por aqui.** |
| `Proxy` | Encaminha para `Server=host:porta` e entrega ao jogo status, headers e corpo da resposta. |

O `FH2_ExtraOptions.log` mostra se o hook entrou (`NETWORK: ...`).

## Servidor

1. Instale o Rust uma vez: `winget install Rustlang.Rustup`.
2. Rode `server\build.bat`. Ele usa o toolchain `x86_64-pc-windows-gnu`, que não precisa do Visual Studio.
3. Rode `server\run.bat`. O servidor escuta em `127.0.0.1:8081`.

O servidor também roda em Linux (`cargo run --release -- --listen 0.0.0.0:8081`). Nesse caso, aponte o `Server=` para o IP dessa máquina.

O hook reescreve a URL `https://host/caminho` para `http://<Server>/host/caminho` e adiciona os headers `X-Fh2-Original-Url` e `X-Fh2-Request-Id`. O servidor escolhe a resposta nesta ordem:

1. A primeira regra que casar em `responses/routes.json`, com campos `host`, `method`, `path`, `class`, `xmethod` e glob `*`. Veja `routes.example.json`.
2. Para chamadas Forza (`X-ClassName` / `X-Method`): `responses/forza/<Classe>/<Metodo>.json`, depois `forza/<Classe>/_default.json`, depois `forza/_default.json`.
3. `responses/hosts/<host>/_default.json`.
4. `responses/_default.json`. Hoje ele responde 503.

Formato de uma resposta:

```json
{ "status": 200, "headers": { "Content-Type": "application/json" },
  "json": { ... } }
```

Em vez de `json`, o corpo também pode vir em `body` (texto), `body_b64` (binário copiado de uma captura) ou `body_file`. O campo opcional `delay_ms` atrasa a resposta.

## Roteiro sugerido

1. Use `Mode=Capture`, jogue até o menu online e feche o jogo. Depois veja quais hosts e métodos aparecem no `FH2_Network.jsonl`.
   - Se o arquivo vier vazio, o jogo nem tentou ir à rede, porque o Azimuth finge um usuário e uma rede offline (`OfflineUserView`, `NetworkInformation`). O próximo passo, então, é virar essas flags.
   - Confira também `HideOnlineTabs` em `[Frontend]`, se você usar essa opção.
2. Use `Mode=Proxy` com o servidor rodando e responda as chamadas uma a uma. Os formatos de `_Parameters`/`_Results` do `Forza.WebServices` (25 classes, cerca de 185 métodos) saem dos serializadores do executável.
3. O P2P (Secure Device Associations e sockets) é uma fase separada.

## Limitações conhecidas

- Só funciona no Azimuth **0.9.7**. Em outra versão, o hash da função não confere e o hook não é instalado.
- `SetCookie` e `GetCookie` são no-ops. `OnRedirect` nunca é chamado.
- O tráfego entre o hook e o servidor é HTTP puro, sem TLS, e vai para o endereço do `Server=`.
- O hook não foi executado contra o jogo ainda. A primeira rodada em `Capture` é o teste real.
