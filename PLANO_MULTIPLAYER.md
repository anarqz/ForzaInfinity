# Plano: rework do multiplayer (contas, servidores comunitários e site)

Status: proposta, nada implementado. Datado de 14/10/2026.

## Objetivo

- Qualquer pessoa hospeda um servidor FH2Online, e qualquer jogador se conecta a ele.
- A conta é criada pelo próprio jogo, sem site e sem Xbox Live, no formato `nome#12345`.
- Enquanto o jogador está conectado, o servidor é a fonte da verdade do progresso, e o save local deixa de valer.
- O site deixa de ser um painel de debug em tempo real e vira uma comunidade, com perfis públicos, rankings, clubes e interações autenticadas.

## O que já sabemos e torna isso viável

| Peça | Situação |
|---|---|
| Rede | Todo o tráfego do jogo já passa pelo hook (WinHTTP), que encaminha para `[Network] Server`. |
| Identidade Xbox | O hook já implementa partes de `Windows.Xbox.System.User` (aparece "XBL: User.GetTokenAndSignature" no log). Dá para devolver o XUID e o gamertag da conta. |
| UI do jogo | O hook já intercepta o parser de XML (`CXML4Resource`, `garage.c`) e reescreve `ui_screens.xml` e `pause.xml` em memória. É o mesmo caminho para criar uma tela ou opção nova. |
| Entrada de texto | O jogo tem `VirtualKeyboardPopup`, usado para nomes de clube e de ajuste, e o Azimuth implementa o teclado virtual. Serve para digitar nome e endereço do servidor. |
| Opções | As configurações são chaves `Options/*` (por exemplo `Options/Ghosts`) ligadas a linhas da tela de configurações. |
| Limite do gamertag | `UserData.wzGamerTag` e `UserLogonData.gamerTag` têm no máximo 15 caracteres. Com `#12345` ocupando 6, o nome fica com **no máximo 9 caracteres**. |
| Mensagens no jogo | A central de mensagens funciona (`Notifications`), então o servidor consegue mandar um código para dentro do jogo. É a base do login no site. |

Falta investigar: como uma linha de configuração vira XML e script, e como o XML abre o `VirtualKeyboardPopup`. Isso é a fase 0.

## 1. Identidade e contas

### Fingerprint do dispositivo
- O hook calcula `SHA-256("FH2Online/v1" + MachineGuid + número de série do volume do jogo)`. O `MachineGuid` vem de `HKLM\SOFTWARE\Microsoft\Cryptography`, lido com `RegGetValueW` carregado sob demanda, como o autodrive faz com o User32.
- O `#12345` são os 5 primeiros dígitos decimais do hash (mod 10^5), sempre com zeros à esquerda.
- Só esses 5 dígitos saem do PC. O hash inteiro nunca é enviado.
- **O fingerprint não serve como senha.** São só 5 dígitos públicos, que funcionam como discriminador, igual ao `#1234` do Discord. A autenticação é feita pelo token descrito abaixo.

### Criação de conta (primeira conexão a um servidor)
1. O jogador escolhe o servidor e digita o nome, de 1 a 9 letras ou dígitos.
2. O hook chama `POST {servidor}/fh2online/v1/register` com `{name, discriminator, device_pubkey|nonce}`.
3. O servidor:
   - recusa se `name#discriminator` já existe com outro dispositivo;
   - aloca um XUID próprio (faixa `0x0009…`, como já fazemos com `2535…`);
   - devolve `{account_id, xuid, gamertag, token}`.
4. O hook guarda o token em `data\fh2online\servers\<host>.json`. Cada servidor tem seu próprio token, e um token nunca é enviado a outro servidor.
5. A partir daí, o hook coloca `Authorization: Bearer <token>` em todo pedido que encaminha. O servidor resolve "quem sou eu" pelo token, em vez de usar `latest_player`, o que fecha a pendência da M2.
6. O XUID e o gamertag que o jogo vê (`User.Xuid`/`GameDisplayName` no shim do hook) passam a ser os da conta. As linhas de placar, os clubes e os rivais passam a mostrar `nome#12345`.

**Transporte:** um servidor público deve usar HTTPS, que o WinHTTP faz nativamente. HTTP puro só é aceito em `127.0.0.1` ou na rede local, para quem hospeda em casa. Se o endereço digitado for `http://` público, o hook avisa.

**Recuperar a conta em outro PC:** pelo site, depois do login. O site gera um código de transferência que é digitado no jogo do PC novo.

## 2. UI dentro do jogo

Duas entradas novas em **Configurações → FH2Online**, criadas pelo patch de XML:

| Linha | Tipo | Ação |
|---|---|---|
| Servidor | texto | Abre o `VirtualKeyboardPopup` com o endereço atual. Salva em `FH2_ExtraOptions.ini [Network] Server`. |
| Conta | botão | Sem conta: pede o nome e cria a conta. Com conta: mostra `nome#12345`, com as opções "trocar de conta" e "sair". |
| Código do site | botão | Pede ao servidor um código de 6 dígitos para fazer login no site. Também chega pela central de mensagens. |
| Modo | offline/online | Offline usa o save local de sempre e não conecta. |

- **Onde fica o valor:** em `Options/*` no save, se der para criar uma chave nova, ou no ini do hook, que é mais simples e não altera o save. A recomendação é o ini.
- **Trocar de servidor:** exige reconectar. O mais simples é pedir para reiniciar o jogo. Uma alternativa é forçar o `PermissionsLoader` e o `Logon` de novo, o que precisa de RE.
- **Plano B:** se criar uma tela nova se mostrar difícil, reaproveitar um fluxo que já abre o teclado (o de nome de clube, por exemplo) e reapontar o resultado com o hook. Ou, como último recurso, um popup próprio desenhado pelo hook (o autodrive já desenha notificações no HUD).

## 3. Save: o servidor como fonte da verdade

O jogo grava o save pelo Azimuth (`data\saves\player-1\User_*\revisions\<n>`, com um arquivo `commit` que lista o SHA e o tamanho de cada blob). Proposta:

1. **Save separado por servidor e conta.** O hook redireciona a pasta de saves do Azimuth para `saves\online\<host>\<account>\` quando estiver em modo online. O save offline continua intocado em `player-1`.
2. **Download antes do jogo carregar.** No `DllMain`, antes do jogo ler o save, o hook baixa a última revisão aceita pelo servidor (`GET /fh2online/v1/save`). O hook já faz HTTP síncrono no boot, no teste do DataWriter. Uma conta nova começa com o save inicial do jogo, não com o save offline.
3. **Upload a cada commit do Azimuth.** O hook observa o commit da revisão (o hook de save do `azimuth_save.c` já toca nisso) e envia `PUT /fh2online/v1/save` com os blobs e o `commit`.
4. **Validação no servidor**, em etapas:
   - v1: aceita só uma revisão que continua a última aceita (número e hash anteriores) e guarda o histórico. Um cliente não consegue voltar uma revisão nem colar um save offline.
   - v2: compara com o que o servidor já sabe pelas chamadas Forza: créditos, XP e nível (`SubmitXp`, `UpdateAllTables`, `GetPayout`), carros na garagem e recordes. Revisões incoerentes ficam em quarentena.
   - v3, opcional: decodificar o `SaveState` (já mexemos em `IESaveState`) para validar campo a campo.
5. **Conflito:** o servidor sempre ganha. O hook descarta o save online local se ele divergir do servidor.

Ser honesto sobre o anti-cheat: como o jogo roda no PC do jogador, dá para dificultar, mas não para impedir. A v2 já pega o grosso, como créditos inventados e carros que não se sabe de onde vieram.

## 4. Servidor

- **Camada de dados:** criar um trait `Store` com dois backends, `SqliteStore` (atual) e `PgStore`.
  - Migrações numeradas em `server/migrations/NNNN_*.sql`, com SQL compatível com os dois (o `ON CONFLICT` funciona nos dois). As diferenças ficam isoladas por backend (`INTEGER PRIMARY KEY` vs `BIGSERIAL`, `BLOB` vs `BYTEA`).
  - Os fantasmas e os blobs de save saem do banco e vão para disco ou armazenamento de objetos (`data/blobs/<sha256>`). O banco guarda só a referência.
- **Postgres:** o crate `postgres` (síncrono) depende do `tokio` por baixo. Nosso problema com `dlltool` acontece só no toolchain GNU do Windows. Os servidores comunitários vão rodar em Linux ou Docker, onde isso não é problema. Para o Windows local, o SQLite continua sendo o padrão.
- **Tabelas novas:**
  - `accounts` (id, name, discriminator, xuid, created_at, banned);
  - `devices` (account_id, token_hash, fingerprint_prefix, last_seen);
  - `web_sessions`, `login_codes`, `save_revisions`;
  - `follows` (substitui o `friends` manual);
  - `profile_comments`, `audit_log`.
- **Configuração do servidor:**
  - nome;
  - descrição;
  - registro aberto ou só com convite;
  - mensagem do dia (que vira notificação de "Novidades");
  - lista de admins.
- **API pública** (versionada em `/api/v1`, JSON, só leitura sem login):
  - perfis, rankings por rota, evento e classe;
  - clubes, atividade e estatísticas do servidor.

## 5. Site: comunidade, não painel

### Autenticação no site
Fica sem senha, usando a central de mensagens do jogo:
1. No site, a pessoa digita `nome#12345`.
2. O servidor envia um código de 6 dígitos para o inbox do jogo daquela conta. Dá também para gerar o código em Configurações → FH2Online → Código do site.
3. Com o código, o site cria uma sessão (cookie `HttpOnly` e `SameSite=Lax`, que expira em 30 dias).

Só quem tem o jogo logado naquela conta consegue entrar.

### Páginas
- **Início:**
  - destaques da semana: recordes quebrados, clubes em alta, novos jogadores;
  - status do servidor.
- **Perfil** (`/u/nome-12345`):
  - capa com o carro favorito, nível, título, clube;
  - estatísticas (km, vitórias, créditos), recordes por rota com o fantasma disponível;
  - carros da garagem, conquistas, atividade recente.
- **Rankings** por rota, classe e tipo (corrida, showcase), com filtros. Cada linha leva ao perfil.
- **Clubes:** página pública com mural, membros, escada de XP, além de pedir para entrar e convidar.
- **Rivais:** os desafios da pessoa e o botão **"Desafiar"** no perfil de outro jogador, que manda um desafio que aparece em Meus Rivais no jogo.
- **Admin** (`/admin`, só para admins do servidor): o painel atual de eventos em tempo real, moderação e banimentos.

### Interações autenticadas
- seguir ou deixar de seguir, o que vira amizade no jogo pelo `SetFriends`/`People`;
- mandar mensagem, que chega no inbox do jogo;
- comentar no perfil e no mural do clube;
- desafiar em uma rota;
- gerenciar o próprio clube.

### Visual
- **Base:** escuro, com destaques no laranja e rosa do Festival Horizon, e tipografia condensada parecida com a do jogo (por exemplo Barlow Condensed e Inter).
- **Componentes:** cards grandes com foto do carro e microanimações discretas.
- Responsivo para celular.
- O React atual (`web/src`) continua, com:
  - roteamento por URL;
  - dados por fetch na API v1;
  - WebSocket só no admin e para notificações ao vivo.

## Fases

| Fase | Entrega | Risco |
|---|---|---|
| 0. RE da UI | Dump de `ui_screens.xml` e das telas de configuração; achar como uma linha liga a `Options/*` e como o XML chama o `VirtualKeyboardPopup`. Uma opção de teste no menu. | Médio. É o ponto que decide se a UI fica dentro do jogo. |
| 1. Contas no servidor | `accounts`, `devices`, `/register`, token em todo pedido, "me" passa a ser o dono do token. Por enquanto o hook lê o servidor e o nome do ini. | Baixo |
| 2. Identidade no jogo | O hook gera o fingerprint, registra a conta e faz o shim do User com o XUID e o gamertag da conta. | Baixo a médio |
| 3. UI no jogo | Configurações → FH2Online (servidor, conta, código do site). | Médio (depende da fase 0) |
| 4. Save online | Pasta de save por conta, download no boot, upload no commit, validação v1. | Médio a alto |
| 5. Camada de dados | Trait `Store`, migrações, Postgres, blobs fora do banco. | Baixo |
| 6. Site comunidade | API v1, login por código no jogo, perfis, rankings, clubes, interações e o novo visual. | Baixo |
| 7. Validação v2 | Coerência do save com a economia e o placar, mais quarentena. | Médio |

As fases 1, 5 e 6 não dependem de RE e podem andar em paralelo com a fase 0.

## Decisões
1. **Decidido (Alex, 14/10/2026):** o nome tem de 1 a 9 caracteres e é sempre seguido de `#` e 5 dígitos (`#00000` a `#99999`). Total de até 15 caracteres, o máximo do gamertag.

2. **Decidido:** a conta online sempre começa do zero. Nada do save offline é importado.
3. **Decidido:** uma conta por PC em cada servidor.
   - O hook envia `device`, o SHA-256 em hex do fingerprint completo junto com o host do servidor. Assim cada servidor vê um id diferente para o mesmo PC e não dá para cruzar jogadores entre servidores.
   - O servidor guarda só o SHA-1 desse id (`accounts.device`, único).
4. **Decidido:** existe um servidor principal, que vem como padrão nas opções. Outro endereço continua podendo ser digitado.

## Andamento
- **Fase 1, no servidor (feita):**
  - `server/src/db/accounts.rs` e os endpoints:
    - `POST /fh2online/v1/register` com `{name, discriminator, device}`, que devolve `{xuid, gamertag, token}`;
    - `GET /fh2online/v1/me` com `X-Fh2-Token`;
    - `GET /fh2online/v1/info`.
  - Os pedidos do jogo com `X-Fh2-Token` passam a ser da conta.
    - Sem token, um XUID de conta é ignorado, inclusive o `loginData.xuid` do `Logon`.
    - Uma conta suspensa recebe 403 (`POST`/`DELETE /api/users/<xuid>/ban`).
  - O `Logon` usa o XUID e o `nome#12345` da conta.
- **Fase 2, identidade no jogo (feita e testada no jogo em 14/10/2026, hook `fh2online.15`):** `hook/adapters/azimuth_account.c`.
  - **Configuração no ini:** `[Account] Name=Alex`, com `[Network] Server` (aceita `https://`).
  - **Fingerprint:** SHA-256 de `FH2Online/v1|MachineGuid|serial do volume`. O `#` é formado pelos 8 primeiros bytes, mod 100000.
  - **Device id:** SHA-256 de `hex(fingerprint)|host:porta`.
  - **Token:** guardado em `data\bin\FH2Online_Accounts.ini`, numa seção por servidor. Nas aberturas seguintes é validado com `GET /me`. Se o servidor responder 401, o hook registra a conta de novo.
  - **Identidade no Azimuth:**
    - `LocalUser.get_XboxUserId` (slot `azimuth+0x24ff88`, que formatava o fixo `0x9000000000000`) devolve o XUID da conta.
    - O nome do jogador é uma `std::wstring` em `azimuth+0x318c60`, lida por todos os getters de gamertag e nome de exibição (`LocalDisplayInfo`, slots `0x24ffe8`, `0x24fff8` e `0x250000`). O hook reescreve essa string no lugar.
    - Os getters esperam a thread de registro terminar, por no máximo 10 s.
  - **Resultado:** com o XUID novo, o Azimuth grava em `saves\player-1\User_<xuid hex>`, uma pasta própria. O jogo trata a conta como um jogador novo (boas-vindas, ajuste de brilho, introdução) e o save offline (`User_9000000000000`) não é tocado. A conta começa do zero.
  - **Validado no jogo:**
    - todas as chamadas chegam com o XUID da conta e o `X-Fh2-Token`;
    - o `GetUserData` devolve `Alex#72178`;
    - o menu de pausa mostra "Alex#72178";
    - na segunda abertura o token é reaproveitado sem criar outra conta.
