# FH2Online: milestones

O jogo é só o título. Tudo o que o Xbox One fazia por ele (sistema, SDK, Xbox Live) não existe no PC, então o FH2Online reimplementa:

- **no servidor (`server/`):** os serviços Forza (`Forza.WebServices`, `Xls.WebServices`) e os serviços Xbox Live que o jogo chama por HTTP (profile, presence, people, achievements, sessiondirectory…);
- **no hook (`hook/`, fork `anarqz/ForzaHorizon2_ExtraOptions`):** as APIs do console que o Azimuth deixa offline ou fecha na hora (conectividade, telas do sistema, privilégios, party…). Também as telas do sistema que o jogo espera abrir: cartão de perfil, seletor de amigos, mensagens e convites.

**V1** (M0–M6) é o online assíncrono: amigos, clubes, rivais, corridas contra fantasmas, placares, pinturas e Storefront. O multiplayer em tempo real (vários jogadores no mesmo mapa) é a **V2** (M7), que só começa depois da V1.

Legenda: ✅ feito · 🟡 em andamento / parcial · ⬜ a fazer

---

## M0: Fundação ✅
- ✅ O hook no `azimuth-tls.dll` (fork do ExtraOptions) encaminha o HTTP do jogo para o servidor.
- ✅ O esquema `Forza.WebServices` foi extraído do executável (`server/schema.json`), com o codec `bin/xtsw` nos dois sentidos.
- ✅ `User.Logon` e o envelope de erro Xls estão implementados.
- ✅ O hook substitui o DataWriter/DataReader do Azimuth, que perdia dados.
- ✅ Captura de todo o tráfego (`captures/*.jsonl`) e respostas editáveis a quente (`responses/`).

## M1: Carreira conectada (single-player com serviços) 🟡
- ✅ Perfil em SQLite: `UpdateAll`, `UpdateAllTables`, transações, `Reconcile`.
- ✅ XP, créditos, economia com os valores reais do jogo, bônus diário.
- ✅ Drivatars: salvar, servir para corridas e cache.
- ✅ Rivais (aleatórios por enquanto) e fantasmas: melhor tempo por evento e por jogador.
- ✅ Nomes e gamerpics para qualquer XUID: gerados, ou o avatar da gamertag.
- ✅ Tabela de permissões (`User.GetPermissionsTable`): o jogo considera o "servidor Forza" no ar.
- ✅ Conectividade: o hook informa `XboxLiveAccess` (nível 4), o que destrava o menu online.
- 🟡 Ver no jogo o `UploadFile` de fantasmas (modo Rivals) e o tamanho real deles.
- ⬜ Time trials, fotos em destaque, conteúdo curado e storefront com conteúdo real (hoje são listas vazias).

## M2: Autenticação e contas ⬜
- ⬜ Login no nosso sistema no lugar do token falso (`fh2online-token`). Isso cobre conta, senha ou chave e sessão por PC. O servidor emite os tokens XBL3.0/XSTS e devolve sempre "Xbox Live = 1".
- ⬜ Registro de jogadores: gamertag única, XUID estável, gamerpic, tier "Gold".
- ⬜ O hook lê as credenciais do `FH2_ExtraOptions.ini` (ou de um login na primeira vez) e assina as requisições.
- ⬜ Servidor acessível pela rede, não só em `127.0.0.1`: HTTPS ou túnel, e um mínimo de segurança.

## M3: Serviços Xbox Live reimplementados 🟡
- ✅ `profile.xboxlive.com`: settings em lote, e `People` com os amigos do servidor.
- ✅ `social.xboxlive.com /users/xuid()/people`: quem o jogador segue, com "favorito" e "segue de volta".
- ✅ `userpresence.xboxlive.com`: guarda o rich presence que o jogo posta. `groups/People` devolve a presença dos amigos. Online = o jogo postou nos últimos 5 min.
- ✅ `Xls.User.SetFriends`: aceito, sem nada a guardar.
- 🟡 "me" nas chamadas Xbox Live vira o último jogador logado. Com autenticação (M2), passa a ser o dono do token.
- 🟡 `achievements.xboxlive.com`: hoje responde vazio. O desbloqueio é local, no Azimuth, e o hook precisa repassá-lo.
- ⬜ `eds.xboxlive.com /media/*/fields` (hoje 503).
- ⬜ `sessiondirectory` / MPSD e `qosservers` com respostas coerentes: clubes, convites, Road Trip online.
- ⬜ Privacidade e privilégios, para o multiplayer e o conteúdo gerado pelos jogadores.

## M4: Social, gerenciado pela web 🟡
Decisão (2026-10-13): **sem UI dentro do jogo**. O jogo só consome (amigos, presença, clubes, placares), e toda a gestão é CRUD no servidor, feita pelo manager web. As telas do sistema do Xbox que o jogo tentaria abrir (cartão de perfil, convidar amigo, mensagens) não funcionam no Azimuth; essas ações passam a ser feitas pela web.
- ✅ Jogadores: criar (XUIDs a partir de 2535000000000001) e remover.
- ✅ Amigos: adicionar (nos dois sentidos), remover, favoritar, presença ao vivo.
- ✅ Car Clubs:
  - **Leitura** (`Forza.WebServices.CarClub`): palavras-chave, recomendados, clubes dos amigos, detalhes, info completa, busca por tag, por nome e por filtros.
  - **Ações** (`Xls.WebServices.Club`): criar, editar, convidar (por XUID ou gamertag), aceitar convite, pedir para entrar (direto se o clube for aberto ou aberto para amigos), aprovar, sair, remover, mudar cargo, sugerir remoção, histórico, mural, `GetUsersToInvite` e `GetUsersToJoin`.
  - **Na web:** criar e apagar clubes, privacidade, membros e cargos, convites, pedidos, mural.
  - **Valores do jogo:** cargos 1 = Líder, 2 = Vice, 3 = Membro; privacidade 0 = Aberto, 1 = Amigos, 2 = Convite, 3 = Privado.
  - ⬜ Conferir no jogo se `Category` 0/1/2 das palavras-chave é mesmo região/atitude/interesse.
  - ⬜ Convites dentro do jogo: o jogo provavelmente os recebe via `Notifications.RetrieveForUser`, que ainda vem vazio.
- ✅ "Meus rivais" (`User.GetCompareStatsForRelatedUsers`): as estatísticas de carreira dos amigos, vindas do `UpdateAll` deles. Também `GetCompareStatsByBucket` e `SetPlayerCardInfo`.
- ⬜ Grupos e mensagens (`Xls.Notifications.SendTextNotification` / `RetrieveForUser`).
- ⬜ Convites para clube (`Xls.User.AcceptClubInvites`).

## M5: Competição assíncrona entre amigos 🟡
- ✅ Meus Rivais: o melhor tempo de cada amigo em cada rota que o jogador ainda não bateu ("TIME TO BEAT"), sincronizado a cada `RetrieveForUser`; "Ir para" corre contra o fantasma do amigo. Só corridas por tempo numa rota: o jogo não tem rival para radar, zona ou habilidade.
- ✅ Placares de amigos (`GetRows`, `GetRowsAnthem`): radares, zonas e habilidade separados por evento (`pivotId`).
- ✅ Drivatars dos amigos nas corridas e nos encontros de carros, com a pintura do carro (pintura registrada sem camadas, ou a cor de fábrica gerada pelo servidor).
- ✅ Storefront entre amigos e clube (designs, grupos de vinil, ajustes): compartilhar, RECOMMENDED DESIGNS, busca, baixar, curtidas, downloads, meus arquivos, remover; denúncias com 3 strikes de jogadores diferentes.
- ⬜ Pinturas com camadas de outros jogadores nos encontros e drivatars: o Azimuth para em `CreatePlacementTexture2D` (formato de textura que não emula). Hoje esses carros aparecem na cor de fábrica.
- ⬜ Presentes de design/ajuste para um amigo (`GiftLivery`, `GiftTuning`, `GiftLayerGroup`, `Gifting.*`).
- ⬜ Time Trials, fotos e replays na Storefront, Bucket List e desafios.

## M6: Manager web 🟡
- ✅ Perfil ao vivo por WebSocket: XP, garagem, placares, fantasmas, feed, tráfego.
- ⬜ Administração: jogadores, amigos, grupos, moderação, ajustes da economia.
- ⬜ Conquistas e estatísticas completas.

## M7: Online ao vivo (V2) ⬜
Só depois da V1. Levantamento de 2026-10-14 (ONLINE ROAD TRIP mostra "UNABLE TO CONNECT TO THE LIVE CONFIGURATION SERVERS", `Lobby.str` 0xf3f0):
- O online ao vivo do FH2 rodava em **servidores dedicados do Xbox Live Compute**. Fluxo, pelo exe e pelo `Microsoft.Xbox.Services.dll`:
  1. `GameServerPlatform`: `POST gameserverds.xboxlive.com/titles/1368890819/variants` (`{"locale":"en-us","maxVariants":8,"publisherOnly":false}`; resposta `variants[]` com `variantId`, `name`, `isPublisher`, `rank`, `gameVariantSchemaId`, e `variantSchemas[]` com `variantSchemaId`, `name`, `schemaContent`) e `GET …/qosservers` (`qosServers[]` com `serverFullQualifiedDomainName`, `secureDeviceAddressBase64`, `targetLocation`). Hoje os dois caem no mesmo stub de variants vazio (`responses/hosts/gameserverds.xboxlive.com/_default.json`): é isso que dá o erro.
  2. Medição de latência: `Windows.Xbox.Networking.QualityOfService` / `MeasureQualityOfServiceResult`.
  3. Alocação do servidor: `AllocateClusterAsync` (`ClusterResult`; mensagem "AllocateClusterAsync returned an unexpected GameServerFulfillmentState").
  4. Sessão do Xbox Live: `Microsoft.Xbox.Services.Multiplayer.MultiplayerSession` (sessiondirectory / MPSD).
  5. Conexão criptografada até o servidor: `Windows.Xbox.Networking.SecureDeviceAssociation` (`SecureDeviceAssociationTemplate`, `SecureDeviceAddress`). Também `Windows.Xbox.Multiplayer` (party). O Azimuth stuba tudo isso como offline.
- O servidor dedicado do Forza nunca foi distribuído: o protocolo de jogo ao vivo tem de ser reescrito a partir do que o jogo envia e espera. Corridas do servidor reportam por `Forza.WebServices.Hopper.SubmitServerRaceResults`.
- **2026-10-15:** o `fh2-mods` (`conner-mcvay/fh2-mods`, `tools/online`, MIT) já fez os passos 1 a 5 e o protocolo NerdCore até entrar no free roam com um console. O "servidor" do FH2 só **retransmite** os objetos e os carros de cada console (não simula), então um jogador pode rodar esse papel.
- **Plano:** `PLANO_FREEROAM.md`.
  - **Decidido (Alex, 15/10/2026):** os jogadores hospedam os mundos (`dedicated_server.exe` + `server.cfg`). O servidor ForzaInfinity só anuncia os mundos, preenche a sessão MPSD e troca os endereços.
  - **Decidido (Alex, 15/10/2026):** só conexão direta (hole punching com STUN público). Nenhum relay nosso, nem como reserva. Diagnóstico de NAT no jogo e no host, mais um guia.
  - ✅ **F0 (15/10/2026):** PC e Deck (Azimuth 0.9.9) no mesmo mundo, na rede local, dirigindo juntos, inclusive com pintura em camadas. Branch `freeroam` (hook `azimuth_freeroam.c` e `azimuth_livery.c`, `server/src/freeroam.rs` com SmartMatch, `dedicated/`).
  - ✅ **F1 (15/10/2026):** hospedar pelo jogo: HOSPEDAR MUNDO (INFINITY, mundo público) e a SESSÃO ONLINE PARTICULAR do próprio jogo (mundo fora da lista), com o `dedicated_server.exe` num Job Object; `server.cfg`; `/infinity/v1/worlds`; HORIZON SOLO encerra o mundo.
  - ⬜ **F2:** JOIN WORLD nas ações do jogador, senha, só amigos.
  - ✅ **F3:** conexão direta pela internet (STUN, punching, UPnP), diagnóstico e guia. **Provado em 16/10/2026:** o SDECK no 5G entrou no mundo do Windows atrás do NAT de casa (sem UPnP), sem abrir porta. Relay `dedicated/src/{stun,nat,upnp}.rs`, servidor `worlds/punch`, hook `azimuth_proxy.c` (proxy local, que também é o filtro de origem), TESTAR CONEXÃO com o NAT e o guia `docs/FREEROAM.md`.
  - ⬜ **F4:** corridas entre jogadores e clima compartilhado (engenharia reversa nova).

---

## Pendências abertas (última sessão)
| Requisição | Situação |
|---|---|
| `userpresence …/titles/current` (POST) | ✅ 200 (`611e7a0`) |
| `profile …/people/People` | ✅ 200, lista vazia (`611e7a0`) |
| `Scoreboard.GetTimeTrials` | ✅ lista vazia (`611e7a0`) |
| `Storefront.GetFeaturedPhotos` | ✅ lista vazia (`611e7a0`) |
| `eds.xboxlive.com /media/pt-BR/fields` | ⬜ 503: descobrir o formato que o XSAPI espera |
| Profile Card / telas do sistema | ⬜ ver M4 |
