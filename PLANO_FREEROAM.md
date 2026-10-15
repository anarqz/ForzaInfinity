# Plano: free roam online com mundos hospedados pelos jogadores

Status: proposta, nada implementado. Datado de 15/10/2026. É o plano da M7 (`MILESTONES.md`).

## Objetivo

- Vários jogadores no mesmo mapa do free roam online, dirigindo juntos e, depois, correndo uns contra os outros.
- **Os jogadores hospedam os mundos.** O ForzaInfinity não roda servidor de jogo para ninguém.
- **Só conexão direta** (hole punching). Nenhum tráfego de jogo passa pela nossa infra, nem como reserva (decidido pelo Alex em 15/10/2026).
- Quem não conseguir conectar recebe um diagnóstico claro e um guia para resolver.

## Base: a pesquisa do fh2-mods

`github.com/conner-mcvay/fh2-mods`, pasta `tools/online` (licença MIT, commits de 14 e 15/10/2026). Mesmo build do jogo (1.0.0.64) e Azimuth 0.9.7. A DLL deles (`fh2net`) ocupa o mesmo lugar que o nosso hook (`azimuth-tls.dll`).

### Como o FH2 conecta os jogadores

- **Não é P2P.** Cada console se liga a um **servidor dedicado** (Xbox Live Compute), num formato estrela. Nenhum console é host.
- O servidor não simula física: ele **retransmite** os objetos que cada console possui (handle com a flag "relay" e o índice de 8 bits do dono).
- Ou seja: o "servidor" do FH2 é pequeno, e o jogador que hospeda roda esse papel, não o jogo.

### Caminho até o free roam (provado por eles com um console)

1. `gameserverds …/variants` devolve `Release_6CC152FD.FreeRoam` (todos os campos como string JSON; o hash do build fica em `0x142F4DAF0`). `/qosservers` com exatamente uma chave `qosServers`.
2. MPSD (`sessiondirectory`, contrato 105) em memória.
   - `GET /sessions` volta vazio de propósito: listar sessões velhas fazia o jogo tentar reentrar nelas.
   - **Truque:** quando a sessão tem `constants.custom.gameServerPlatform`, o servidor preenche `servers.gameServerPlatform.properties.custom = {status: "Active", datacenter, secureDeviceAddress, hostName}`. O jogo lê isso em `0x14183B360` e o `AllocateServer` (`0x141836190`) pula a alocação de cluster.
3. Stubs do Azimuth trocados (cada um era "offline"):

| Stub | O que a fh2net faz |
|---|---|
| Conectividade | nível 4 (o nosso hook já faz desde a `.11`) |
| `DataWriter`/`DataReader` para o XSAPI | implementação própria (o nosso hook já tem uma para o jogo) |
| `QualityOfService.MeasureQualityOfServiceAsync` (slot 6) | resultado pronto, status 1, 20 ms |
| `SecureDeviceAddress.GetLocal` (slot 9) e `FromBase64String` (slot 11) | base64 de verdade (vazio quebra o membro do MPSD) |
| `SecureDeviceAssociationTemplate` `{46745a18-…}` | `CreateAssociationAsync` (slot 8) devolve uma associação cujo `GetRemoteSocketAddressBytes` (slot 9) dá o `sockaddr_in6` do servidor. `DestroyAsync` concluído evita crash ao sair (`0x14179EA70`) |
| `IPartyStatics` `{e6110c4c-…}` | `RegisterGameSessionAsync`/`RegisterMatchSessionAsync` (slots 6 e 7) concluídos |
| ws2_32 | imports para o ws2_32 real (o Azimuth devolve `WSAENETDOWN` no `WSASocketW`) |
| `bind` | tira `IPV6_V6ONLY` do socket `[::]:5100` do jogo (dual-stack) |

4. Patches de stateflow:
   - `SkipWeatherWait`: `EB 48` em `0x1409641EA` (o clima não é compartilhado ainda).
   - `SkipTrafficWait`: NOP no `jne` em `0x1408E0658`. Sem isso os carros ficam "ghosted" para sempre, porque o tráfego era do servidor.
5. **NerdCore** (UDP, protocolo do jogo):
   - cabeçalho de 11 bytes `<HHIBH` (id, ack, ackbits, nº de mensagens, tamanho); id `0xFFFF` = sem sequência;
   - mensagens empacotadas em bits, id de 7 bits, sem tamanho entre elas; tabela com 77 mensagens em `0x142F4DB20`;
   - handshake: OpenConnection (versão `6CC152FD`, `Flagship`, XUID) → OpenConnectionResponse → ResponseAck → TimeSync + ConnectionSyncDone → Ping/Pong;
   - objetos (`SyncedObject`): router, handle, tipo, seq, tamanho em bits, dados. Router 6 = lobby (SessionHost, GameOptions, só o servidor cria), 9 = Road Trip, 10 = clima;
   - `ConsoleSynced` (router 3): 0 RenderReady, 1 SessionEntryFinalised, 2 ParticipatingInRace, 3 RaceInProgress, 4 Ready, 5 HasLoadedCarsForAllParticipants, 6 HasGrid, 7 AvailableArbitration, 8 MatchTicketArbiter. **1, 2 e 4 são do servidor**;
   - carro: `CarPredictionFull` (msg 15), 710 bits, ~30 Hz, sem sequência.
6. Relay (`relay.py`, UDP 5101): ao sincronizar, cria o SessionHost e repassa o histórico; repassa Create/Update e carros com o handle de relay; responde `FinaliseSessionEntry` (34) com a var 1 = true; ping a cada 0,5 s (o console larga o servidor após 10 s calado); remove quem some por 15 s e manda `ObjectDelete` aos outros.

### O que eles **não** provaram

- **Dois jogadores reais juntos.** Só um console vendo a si mesmo atrasado (`--mirror`).
- Clima, tráfego de IA, mensagens 35/71/72 (uma mensagem desconhecida corta o resto do pacote).
- Corridas entre jogadores, Road Trip, convites, party.
- Os endereços do Azimuth são da 0.9.7. Na 0.9.9 têm de ser achados de novo (`offsets_099`). Os endereços do jogo (`0x14…`) valem nas duas.
- Todo endereço acima deve ser conferido na nossa decompilação antes de usar.

## Arquitetura

| Peça | Onde roda | Função |
|---|---|---|
| **`dedicated_server.exe`** + `server.cfg` | PC do jogador que hospeda | Relay NerdCore (UDP 5101), em Rust só com std, no `server/` ou num crate irmão. Anuncia o mundo ao ForzaInfinity e coordena o hole punching. O jogo do próprio host entra por loopback. |
| **Servidor ForzaInfinity** | GCP, atrás do Cloudflare (só HTTPS) | Diretório de mundos (anúncios), `variants`/`qosservers`, MPSD com o truque do `gameServerPlatform`, troca de endereços para o punching, senha e amizades. **Nada de UDP e nada de tráfego de jogo.** A VM não muda. |
| **Hook** | Cada jogador | Stubs da tabela acima reescritos no nosso padrão (C freestanding, sem CRT). STUN e punching no socket do jogo. A `Association` devolve o endereço do mundo escolhido, não um valor do INI. |

### `server.cfg`

```ini
[World]
Name=Mundo do Alex
Message=Bem-vindos!
Visibility=public        ; public | unlisted | friends | off
Password=                ; vazio = sem senha
MaxPlayers=8             ; até 12 (maxAllowedPlayers da variant)

[Network]
Port=5101
UPnP=1
Announce=https://infinity.anarqz.dev

[Access]
Allow=                   ; gamertags (vazio = todos)
Ban=
```

O host se identifica pela conta (token do `FH2Online_Accounts.ini`), então o mundo aparece como sendo de `nome#12345`.

### Anúncios

- `POST /fh2online/v1/worlds` (heartbeat a cada ~15 s, em long-poll): nome, jogadores, limite, flags (senha, só amigos), versão do hook, tipo de NAT e endereços (STUN IPv6, STUN IPv4, rede local).
- A resposta traz os pedidos de entrada pendentes e a lista de XUIDs liberados.
- O mundo some da lista sem heartbeat por 45 s.
- O relay recusa o OpenConnection de um XUID que o servidor não liberou. A senha é conferida pelo servidor e nunca chega aos outros jogadores.

### Entrar num mundo

**FREE ROAM ONLINE (join automático):**
1. O hook pega os candidatos (`GET /worlds?auto=1`): só mundos públicos, sem senha, com vaga e com NAT compatível.
2. Mede a latência e escolhe o melhor.
3. O servidor grava a escolha para a conta. Quando o jogo escreve a sessão no MPSD, o servidor preenche o `gameServerPlatform` com esse mundo.

**JOIN WORLD (ações do jogador):**
- Novo item no menu de ações (`azimuth_online_menu.c`, mesmo mecanismo do item `0x40000000`). Aparece quando o jogador está hospedando ou está num mundo em que dá para entrar.
- Senha: `VirtualKeyboardPopup`.
- Ao confirmar, o servidor guarda o mundo para a conta e o próximo FREE ROAM ONLINE vai para lá. Abrir o free roam direto pelo menu é RE à parte.

**Sem mundo disponível:** o jogo mostra "nenhum mundo disponível" (texto do hook). O servidor não sobe mundo próprio.

### Hole punching (sem UDP nosso)

1. O hook consulta **servidores STUN públicos** (por exemplo `stun.l.google.com:19302` e `stun.cloudflare.com:3478`) a partir do **socket do jogo** (porta 5100, pego no `bind`). O host faz o mesmo no socket do relay (5101).
2. Os endereços descobertos viajam pela API HTTPS (`POST /worlds/<id>/join`, e a resposta do heartbeat do host).
3. Host e cliente mandam pacotes um ao outro ao mesmo tempo, nesta ordem: **rede local** (se o IP público for o mesmo), **IPv6**, **IPv4**.
4. A `Association` devolve ao jogo o primeiro endereço que respondeu. O NerdCore conecta normalmente.
5. Os pacotes de punch e de STUN são filtrados pelo hook e nunca chegam ao parser do NerdCore.

O relay identifica cada jogador pelo XUID do OpenConnection, além do endereço, para sobreviver a uma troca de porta do NAT.

### Compatibilidade de NAT

O tipo é medido com dois servidores STUN: se a porta pública muda conforme o destino, o NAT é **simétrico**.

| Host \ Cliente | Aberto, UPnP ou IPv6 | NAT cone (a maioria) | Simétrico ou CGNAT |
|---|---|---|---|
| **Aberto, UPnP ou IPv6** | ✅ | ✅ | ✅ |
| **NAT cone** | ✅ | ✅ | ⚠️ costuma falhar |
| **Simétrico ou CGNAT** | ✅ | ⚠️ costuma falhar | ❌ |

**Risco aceito:** quem está em CGNAT ou NAT simétrico e não consegue IP público ou IPv6 só joga com hosts abertos.

### Banda e queda do host

- Carro: ~100 B por pacote a ~30 Hz. Upload do host ≈ N·(N−1)·3 KB/s: ~36 KB/s com 4 jogadores, ~400 KB/s (~3 Mbit/s) com 12. Juntar vários carros num pacote reduz isso.
- Se o host sai, o mundo cai (não há migração de host). O "rejoin" provado pela fh2net permite mandar os jogadores para outro mundo.

## Troubleshooting

### No jogo: TEST CONNECTION → Free Roam

Na tela INFINITY, em linguagem simples:
- IPv6: sim ou não;
- NAT: Aberto, Moderado (cone) ou Restrito (simétrico);
- CGNAT: sim ou não (IP WAN do roteador, pelo UPnP, na faixa `100.64.0.0/10` ou diferente do IP público do STUN);
- UPnP: disponível ou não;
- conclusão, por exemplo "Você entra na maioria dos mundos, mas não consegue hospedar para todos", com o link do guia.

### No `dedicated_server.exe`

- O mesmo diagnóstico ao abrir, mais a tentativa de UPnP/NAT-PMP para a porta 5101.
- Selo no anúncio: 🟢 aberto, 🟡 só NAT compatível, 🔴 só rede local.
- **Teste de alcance por outro jogador:** o servidor pede a um cliente online com NAT aberto que mande um pacote ao host e confirme. Dispensa UDP nosso.
- Mensagens claras no console e num log.

### Na lista e no join

- O join automático pula mundos incompatíveis com o NAT do jogador.
- O JOIN WORLD avisa o motivo ("Seu NAT e o do host são incompatíveis") e aponta para o guia.

### Guia (site e `docs/INSTALL.md`)

Correções em ordem de facilidade:
1. Ativar UPnP no roteador.
2. Ativar IPv6 no roteador ou pedir à operadora.
3. **Duplo NAT** (modem da operadora mais roteador próprio): modem em modo bridge.
4. **CGNAT:** pedir "IP público" à operadora (comum no Brasil, muitas vezes gratuito).
5. Encaminhar UDP 5101 à mão para o PC do host.
6. Firewall do Windows: regra para o `azimuth.exe` e o `dedicated_server.exe` (o instalador cria).
7. Desligar VPN.

### Logs e Report

- Cada tentativa de punch fica no `FH2_ExtraOptions.log`: tipo de NAT dos dois lados, endereços tentados, onde falhou.
- O host grava o mesmo no log dele.
- O ForzaInfinity-Report passa a incluir a seção Free Roam.

## Fases

| Fase | Entrega | Risco |
|---|---|---|
| **F0. Dois jogadores reais** 🟡 (código pronto, falta o teste no jogo) | Stubs no hook (0.9.7 e 0.9.9), patches de stateflow, relay em Rust, `variants`/`qosservers`/MPSD no servidor, endereço do relay ainda no INI. **Teste: PC Linux e Deck no mesmo mundo, na rede local.** Prova o que o fh2-mods não provou. | Médio |
| **F1. Host e anúncios** | `dedicated_server.exe`, `server.cfg`, `/worlds`, endereço do mundo vindo da sessão MPSD, join automático pela latência, XUIDs liberados. | Baixo |
| **F2. JOIN WORLD** | Item nas ações do jogador, senha, só amigos, allow e ban. | Baixo a médio |
| **F3. Conexão direta pela internet** | STUN, punching coordenado pela API, ordem rede local → IPv6 → IPv4, UPnP no host, teste de alcance, diagnóstico no jogo e no exe, guia, logs no Report. | Médio |
| **F4. Corridas e clima** | RE da entrada em corrida: `ParticipatingInRace`, `Ready`, `GameOptions`, grid, contagem pelo relógio do TimeSync. Clima compartilhado (msgs 53/54, router 10). Mensagens 35/71/72. | Alto |
| Depois | Road Trip (router 9), Bucket List (router 12). O tráfego de IA fica de fora: era simulado pelo servidor original. | — |

## F0: estado e como testar

**Feito em 15/10/2026** (branch `freeroam` do monorepo e do hook; nada disso está na `main` nem no servidor público):

- **Hook** (`adapters/azimuth_freeroam.c`, `tests/freeroam_test.py`):
  - liga com `[FreeRoam] Enabled=1`, que exige `[Network] Mode=Proxy`;
  - embrulha `QualityOfService`, `SecureDeviceAddress`, `SecureDeviceAssociationTemplate` e `Party`;
  - leva os imports ws2_32 do jogo para o ws2_32 do Windows, com `[::]:5100` dual-stack;
  - aplica os patches de clima e tráfego (bytes conferidos no nosso exe);
  - o mundo vem da sessão (`fh2inf:<host>:<port>` em base64), ou de `[FreeRoam] World`, ou é `[::1]:5101`.
- **Servidor** (`server/src/freeroam.rs`, `docs/SERVER.md`):
  - responde `variants` (FreeRoam), `qosservers`, `gameserverms /clusters` e o MPSD em memória com o `gameServerPlatform` preenchido;
  - só com `--freeroam-world HOST:PORT` (ou `--freeroam`); sem a flag, nada muda.
- **`dedicated_server`** (`dedicated/`, Rust só com std):
  - relay NerdCore (handshake, objetos, carros, FinaliseSessionEntry, ping, saída e volta);
  - histórico compacto, reenvio do que o console não confirmou, limite de jogadores e `server.cfg`;
  - `build-windows.sh` gera o `.exe`.

**Ainda não visto no jogo.** O roteiro abaixo é o teste da F0 (PC Linux 192.168.68.40 + Deck, servidor local):

1. No PC Linux: `cd dedicated && cargo build --release && ./target/release/dedicated_server` (UDP 5101 aberta na rede local).
2. No PC Linux, o servidor local com o mundo: `cd server && ./run.sh --freeroam-world 192.168.68.40:5101`.
3. O hook da branch `freeroam` (`python build.py <zig> --azimuth`) nos dois aparelhos, com o jogo fechado, conferindo o SHA-256. Nos dois INIs:
   ```ini
   [Network]
   Mode=Proxy
   Server=http://192.168.68.40:8081
   [FreeRoam]
   Enabled=1
   ```
   O modelo de INI da release ainda não leva `[FreeRoam]`: um hook antigo recusaria o arquivo inteiro.
4. Nos dois: ONLINE → FREE ROAM ONLINE.
5. O que olhar:
   - `FH2_ExtraOptions.log`: linhas `FREEROAM:` (statics embrulhados, `world [192.168.68.40]:5101 (from the session)`, socket dual-stack);
   - console do `dedicated_server`: "jogador N entrou no mundo";
   - no jogo: os dois carros no mapa, colidindo.
6. Para testar com um aparelho só: `dedicated_server --mirror 3` (o seu carro volta 3 s atrasado, como jogador 250).

**Riscos a observar no teste:**
- **0.9.9 (Deck):** o hook de ativação já tem os offsets da 0.9.9, mas os stubs de rede dessa versão nunca foram vistos com o free roam.
- **Reason de "mundo cheio":** usamos 4, sem confirmação.
- **SessionEntryFinalised dos outros:** o relay não repassa as variáveis do servidor de um jogador para os outros, igual ao `relay.py`. Pode importar na F4.

## Pendências do relay herdadas da fh2net

- O histórico guarda todo `ObjectUpdate` para sempre e é repassado inteiro a quem entra: guardar só o estado mais recente de cada objeto.
- O conjunto de deduplicação cresce sem limite.
- Mensagens desconhecidas cortam o resto do pacote: mapear os tamanhos para poder pulá-las.
- Um relay serve uma sessão só (no nosso caso, um mundo por host, o que basta).

## Decisões

1. **Decidido (Alex, 15/10/2026):** os jogadores hospedam os mundos; o servidor ForzaInfinity só anuncia e coordena.
2. **Decidido (Alex, 15/10/2026):** só conexão direta por hole punching. Nenhum relay nosso, nem como reserva. O investimento vai para o diagnóstico e o guia.
3. A decidir: host só como exe separado, ou também um "HOSPEDAR MUNDO" no menu INFINITY que abre o mesmo relay. Proposta: começar pelo exe.
4. A decidir: começar a F0 já ou depois de fechar a V1 assíncrona.
