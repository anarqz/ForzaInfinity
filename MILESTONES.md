# FH2Online: milestones

O jogo é só o título. Tudo o que o Xbox One fazia por ele (sistema, SDK, Xbox Live) não existe no PC, então o FH2Online reimplementa:

- **no servidor (`server/`):** os serviços Forza (`Forza.WebServices`, `Xls.WebServices`) e os serviços Xbox Live que o jogo chama por HTTP (profile, presence, people, achievements, sessiondirectory…);
- **no hook (`hook/`):** as APIs do console que o Azimuth deixa offline ou fecha na hora (conectividade, telas do sistema, privilégios, party…). Também as telas do sistema que o jogo espera abrir: cartão de perfil, seletor de amigos, mensagens e convites.

O multiplayer em tempo real (vários jogadores no mesmo mapa) **está fora do escopo por enquanto**.

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
- ✅ `profile.xboxlive.com`: settings em lote e `People`, ainda vazio.
- ✅ `userpresence.xboxlive.com`: aceita o rich presence.
- 🟡 `achievements.xboxlive.com`: hoje responde vazio. O desbloqueio é local, no Azimuth, e o hook precisa repassá-lo.
- ⬜ Guardar e servir a presença (online, "em Free Roam"…) para os amigos.
- ⬜ `social.xboxlive.com` / `People` com os amigos de verdade.
- ⬜ `eds.xboxlive.com /media/*/fields` (hoje 503).
- ⬜ `sessiondirectory` / MPSD e `qosservers` com respostas coerentes: clubes, convites, Road Trip online.
- ⬜ Privacidade e privilégios, para o multiplayer e o conteúdo gerado pelos jogadores.

## M4: Social (telas do sistema pelo hook) ⬜
- ⬜ Mapear as telas do sistema que o jogo chama e o Azimuth fecha na hora: cartão de perfil, seletor de amigos, enviar mensagem, convites.
- ⬜ Uma UI própria injetada pelo hook (overlay) para:
  - adicionar e remover amigos;
  - criar grupos;
  - trocar mensagens;
  - aceitar convites.
- ⬜ O mesmo pelo manager web (`web/`), como alternativa fora do jogo.
- ⬜ Car Clubs (`CarClub.*`) de verdade: criar, entrar, palavras-chave, recomendações.

## M5: Competição assíncrona entre amigos ⬜
- ⬜ Rivais com os tempos e fantasmas reais dos amigos, em vez de aleatórios.
- ⬜ Placares por evento (amigos e global) e Time Trials.
- ⬜ Bucket List, desafios e `GetCompareStatsForRelatedUsers` com dados reais.
- ⬜ Drivatars dos amigos nas corridas offline.
- ⬜ Storefront: compartilhar pinturas, tunes e fotos entre jogadores.

## M6: Manager web 🟡
- ✅ Perfil ao vivo por WebSocket: XP, garagem, placares, fantasmas, feed, tráfego.
- ⬜ Administração: jogadores, amigos, grupos, moderação, ajustes da economia.
- ⬜ Conquistas e estatísticas completas.

## Fora de escopo (por enquanto)
- Multiplayer em tempo real no mesmo mapa: `Windows.Xbox.Networking` (SecureDeviceAssociation, QoS), `Windows.Xbox.Multiplayer` (party) e sessões P2P. O Azimuth stuba tudo isso como offline. Vai exigir substituir essas APIs no hook e ter um relay de rede.

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
