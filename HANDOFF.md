# FH2Online: handoff

Estado em 2026-10-13. Este documento substitui a conversa anterior, que rodou no Claude.ai/cloud: tudo o que foi descoberto e provado está aqui.

## Estado em 2026-10-15, noite (hook fh2online.42, v0.1.6 Latest): leia primeiro

- **v0.1.6** (hook .42, SHA `042bd365…`), instalado nos três aparelhos. O servidor anuncia a versão.
- **AÇÕES DO JOGADOR** (`azimuth_online_menu.c`, seção "player actions"):
  - o montador é `FUN_140e13be0`. Os desvios são em `FUN_140dfec20` (acrescenta uma ação; ela é filtrada pela máscara em `página + 0x4e0`) e em `FUN_140e080a0` (ação escolhida; lê o item em `página + 0x4a0`, com o ID em `item + 0x68`);
  - o jogador é `(*(página + 0x4d0)) + 0x78`, e a vtable `+0x38` dá o XUID;
  - o item nosso tem o ID `0x40000000` e entra logo depois de EXIBIR CARTÃO (ID 1).
  - No servidor: `friends/status`, `friends` com `{xuid}` e `friends/remove`.
- **Idioma:**
  - `azimuth_lang.c` traduz, pelo texto em inglês, quando o `MessageCenter.str` carregado é o do `br.zip` (16604 bytes, conferido como LSB2);
  - o hook manda `X-Fh2-Lang: pt-BR`. O `server/src/l10n.rs` traduz `error`/`message` (modelos com `{}`), e o `account_lang` guarda o idioma de cada conta para os avisos do Message Center;
  - texto novo no hook → nova entrada no `azimuth_lang.c`. No servidor, a entrada vai em `TEXTS`.
- **Limites:** as rotas de conta contam por token, ou seja, por aparelho (60 de folga, 2/s). `info`/`queue`/`stats` contam como tráfego do jogo. Antes, três aparelhos atrás do mesmo IP batiam no 429.
- **Clubes:** a privacidade é numerada como no jogo (1 Aberto … 4 Particular).
- Captura de tela do Deck sem mexer em nada: `XDG_RUNTIME_DIR=/run/user/1000 gamescopectl screenshot /tmp/x.png` pelo SSH.

## Estado em 2026-10-15, fim da tarde (hook fh2online.39, v0.1.5 Latest): leia primeiro

- **v0.1.5** (hook .39, SHA `ec75be37…`). Instalado à mão no Windows e no Deck; o PC se atualizou sozinho.
- **Crash da .38 (corrigido):** 24 arquivos dos zips de carros (`.modelbin`, `.gr2`, shaders) têm o mesmo tamanho de algum `MessageCenter.str`. A .38 lia esses fluxos para testar e voltava ao início, e uma entrada comprimida voltava vazia. Resultado: crash no Windows em `game+0x13e7124`, com "Name"/"Model" nos registradores, quando drivatars traziam esses carros. Agora a tabela é conferida numa segunda abertura da entrada, e o fluxo do jogo só é trocado quando é ela.
- **Crash na viagem rápida no PC** (`game+0x1fd18f`, `Release()` → destrutor → `this` nulo): sumiu com `GetInCarFromPauseMenu=0`. Esse patch do upstream mexe no `pause.xml` e tira as esperas de streaming e de LOD0. Pode também ter sido o mesmo crash da .38 (carros dos drivatars na chegada). O modelo de INI e os três aparelhos estão com 0, e o INI marca a opção como instável.
- **Tela esmaecida ao abrir sem conta no servidor público (PC):** o jogo parou de desenhar uns 10 s depois de abrir, com a GPU ociosa e as threads dormindo. Causa não achada. O `winedbg` com `timeout` matou o processo (no Windows o processo depurado morre junto com o depurador); da próxima vez, usar minidump.
- **Busca de clube por tag que não existe:** agora responde erro. Antes, um `clubDetails` vazio virava uma linha em branco (clube 0) e travava ao abrir.
- O PC agora joga no **servidor público**; o servidor local é só para desenvolvimento.
- Para copiar do Windows: usar `sftp`, porque o `scp` corta em 200 KB.

## Estado em 2026-10-15, tarde (hook fh2online.38, v0.1.4 Latest): leia primeiro

- **v0.1.4 é a Latest** (hook .38, SHA `f222edbc…`). Os servidores público e local anunciam essa versão; hooks .37 ou mais novos se atualizam sozinhos.
- **Atualização automática provada** no Deck: build de teste .36 → `.37` do GitHub.
- **Message Center:** o `azimuth_ui.c` reescreve o `MessageCenter.str` de cada idioma ao abrir (reconhecido pelo tamanho). O nome da equipe (0x229b) vira INFINITY, a linha "De:" também, e o título da lista (0x3232) vira INFINITY. Os textos mudam no mesmo lugar e nunca crescem. Testado nos 17 idiomas (`tests/ui_edits_test.py`, `FH2_RUN` = wine do Proton). **Ainda não visto no jogo**; o log mostra "UI: Message Center texts name INFINITY".
- **Nomes de 1 a 20 letras** (servidor e hook). Os campos de gamertag do jogo têm 15 caracteres (`max_chars` 15 no schema): o jogo recebe `Store::game_tag` (Nome#12345 se couber, senão só o nome cortado em 15), e o hook usa o mesmo como nome do jogador.
- **Erro ao criar perfil, entrar ou adicionar amigo:** popup com "Try again" e "Cancel".
- **Travamento no PC** ao criar o perfil "DominicToretto" (14 letras, acima do limite antigo de 9) logo depois do teclado virtual, na 0.9.9. Não houve pedido de `register`. Causa não confirmada: ou o teclado da 0.9.9 ignora o limite de tamanho, ou o popup aberto ao fechar o teclado trava. Com o limite de 20 o nome passa direto; falta ver se um nome válido cria o perfil sem travar.
- **Placares:** `view` 0 Global, 1 Perto de mim, 2 Amigos, 3 Clube, 4 Amigos e clube, 5 depois do evento. Posição sempre a real no servidor. Os recordes de radar e zona vêm do save (`db/save_bests.rs`).
- **PC Linux:** Azimuth 0.9.9 + hook .37 + INI 0.2.74, servidor local `127.0.0.1:8081`. Backup em `Forza Horizon 2/fh2online-backup-az097-20261015-110137`. **Windows (diablo):** 0.9.9 + build de teste .36 (vai se atualizar sozinho). Ver memória `windows-pc-diablo`.
- **Atenção:** um instalador rodado a partir de uma release *pré-lançamento* baixa o DLL e o `update.sh` da Latest (`releases/latest/download`). Por isso a Latest tem que ser sempre a versão completa.

## Estado em 2026-10-15, meio-dia (hook fh2online.37, atualização automática): leia primeiro

- **Hook .37** (`5cd49b5`), na release **`v0.1.3`, publicada como pré-lançamento** (os instaladores, que usam `releases/latest`, ainda entregam a v0.1.2). SHA `f791ffe4…`.
  - **Atualização automática** (`adapters/azimuth_update.c`): o `/info` do servidor tem `latestHook`/`latestRelease`, vindos de `server/responses/infinity/latest.json`, que o `tools/package_release.sh --release vX.Y.Z` escreve.
    - Se o build anunciado for maior, o hook baixa `azimuth-tls.dll` e `.sha256` **do GitHub** (`anarqz/ForzaInfinity`, nunca do servidor do jogo).
    - Confere o SHA-256, o `MZ` e o `+fh2online.N`, grava `.new`, renomeia o atual para `.previous` e põe o novo no lugar.
    - Desliga com `[Network] AutoUpdate=0`.
  - Na tela INFINITY, o PROFILE virou um bloco 1x1 com o **UPDATE** embaixo (I7 em 0,1). O primeiro clique em INFINITY da sessão mostra a atualização. A linha da pausa avisa "UPDATE INSTALLED | RESTART THE GAME".
  - `ForzaInfinity-Update.sh`/`.cmd` (de `release/update.*`) ficam na pasta do Azimuth; os desinstaladores removem.
- **Ordem de uma release daqui em diante:** `package_release.sh --release vX.Y.Z` → `gh release create` → commit do `latest.json` → `infra/deploy.sh`. Se o servidor anunciar antes de a release existir, o download falha (e o menu manda atualizar à mão).
- **Falta testar no Deck** (estava fora da rede): instalar `scratchpad/selftest.dll` (o código do .37 com `MOD_VERSION` .36), abrir o jogo e ver no log "UPDATE: installed 0.2.74+fh2online.37" e o `azimuth-tls.dll` com o SHA `f791ffe4…`. Depois, o Alex confere os blocos e o popup. Só então promover a v0.1.3 a Latest.
- Landing: textos do servidor aberto (Rivais "Race the whole server's ghosts", Storefront para todos, abertura).

## Estado em 2026-10-15, manhã (hook fh2online.36, Azimuth 0.9.9 ligado): leia primeiro

- **Hook .36:** suporta a **0.9.9** (a mais recente) e a 0.9.7. A 0.9.8 continua de fora.
  - `azimuth_offsets` devolve `offsets_099`.
  - `azimuth_cloud.c` (`saves_folder`): na 0.9.9 usa `data\titles\1A96545B\saves`; o nome alternativo `title-1a96545b` também é procurado. Se nenhum existir, cai em `data\saves`.
  - 1A96545B é o TitleId do FH2 no `AppxManifest.xml`. A 0.9.9 cria a pasta e migra os saves na primeira abertura, deixando uma cópia em `data\saves-backup-<sessão>` (log: "title folder: the saves are moved from … to …").
  - O pad continua só na 0.9.7.
- **Testado no Deck com a 0.9.9** (o Alex confirmou "Deck está super funcional"): versão .36, menu INFINITY, `Anarqz#08723` no servidor público, "CLOUD: save in sync with the server" (o save foi achado na pasta nova) e 39 de 39 `o.xtsw` com 200.
- **Instaladores:**
  - aceitam 0.9.7 e 0.9.9;
  - quando o `azimuth-tls.dll` atual tem menos de 64 KB (o original que uma atualização do Azimuth devolve), ele vira o novo `.stock`. O original da 0.9.9 é diferente do da 0.9.7;
  - o `install.ps1` decodifica o INI baixado da release (o GitHub manda `octet-stream`, e o `.Content` vem em bytes).
- **Modelo `release/FH2_ExtraOptions.ini`:** refeito a partir do da 0.2.74 (o anterior ainda era o da 0.2.34), com Autodrive `Cinematic`/`InEvents` e `[Network]`. `DisableAzimuthSaveChecksums=0` e `EnableMediaOverrides=0`, como testado. O parser de verdade (`settings_read`, rodado no Wine) aceita o modelo novo, o antigo e o do upstream.
- **Release `v0.1.2` (Latest)** com o .36. O teste no Windows ainda falta (o Alex está instalando o SSH nele).
- **Servidor "mundo aberto"** (pedido do Alex; no ar no público às 11:49 UTC, com backup `data/pre-open-world-20261015T114836.sqlite` na VM):
  - `Ctx::can_see` vale para qualquer conta não banida: tempos, fantasmas (`DownloadFile`), pinturas, storefront, nomes, perfil e presença de um jogador;
  - placares são o servidor inteiro, com a posição real; `isClubBoard` mostra só o clube. O servidor imprime o `view` de cada pedido ("scoreboard view …") para descobrirmos qual é o filtro de amigos;
  - drivatars (`GetDrivatarsForCache` e grid) vêm de todo mundo: amigos primeiro, depois os outros embaralhados por pedido (`Store::drivatar_owners`);
  - rival (`GetRival`): o amigo mais próximo, senão qualquer jogador; fantasma de qualquer um;
  - clubes: todos aparecem na busca (os ocultos, privacidade 3, não), `clubXPRank` real e convite para qualquer jogador;
  - continuam só entre amigos: lista de amigos e presença em grupo, mensagens, desafios, Meus Rivais e convites sugeridos;
  - contas banidas ficam fora dos placares, dos fantasmas e dos drivatars;
  - teste `forza::tests::open_to_the_whole_server`;
  - o texto do menu INFINITY no hook ainda diz "ghost races between friends" (mudar no próximo hook).

## Estado em 2026-10-15, noite (hook fh2online.35, Azimuth 0.9.9 mapeado): leia primeiro

- **Público:** release `v0.1.1` (latest) com o hook **.34** = ExtraOptions **0.2.74** + FH2Online. Testado no Deck (Azimuth 0.9.7): menu INFINITY, conta e as edições de fluxo do upstream convivem. Landing em `https://anarqz.github.io/ForzaInfinity/`, com o card de compartilhamento (`landing/og/`).
- **Azimuth 0.9.8 e 0.9.9 saíram** (itch.io, Almost There Games). Por enquanto o ForzaInfinity **exige a 0.9.7**: os instaladores leem a versão do `azimuth.exe` (texto UTF-16 "0.x.y") e recusam outra (`FORCE=1` / `-Force`); README, INSTALL.md e a landing dizem isso.
- **O que muda na 0.9.9:**
  - O `azimuth-tls.dll` tem a mesma interface: exports, TLS de 336/16 e só KERNEL32.
  - O ExtraOptions 0.2.74 já trata 0.9.6–0.9.9 (`azimuth_host_build`, pela data de compilação do PE).
  - As nossas partes eram 0.9.7 fixas: rede, conta e o controle de testes.
  - A 0.9.9 move os saves para uma pasta por jogo: `<pasta do jogo>\saves\player-1`, com backup em `saves-backup-…` e o log "title folder: this game's saves are in …". O nome da pasta é montado em tempo de execução (estrutura em `DAT_14031a9c0`/`+0x88`, juntada por `FUN_140081dc0` em `FUN_140187040`).
  - Atualizar o Azimuth por cima sobrescreve o nosso `azimuth-tls.dll`: é preciso rodar o instalador de novo.
- **Hook .35** (`9cdbd4d`, commitado, **não publicado nem instalado**):
  - Os endereços do `azimuth.exe` usados pela rede e pela conta vêm de `adapters/azimuth_offsets.c` por versão. Os da 0.9.9 saíram do `tools/azimuth_map.py` (casamento de funções pelo Capstone, com os operandos de endereço apagados): todas iguais instrução por instrução.
  - **A 0.9.9 continua desligada** (`azimuth_offsets` devolve 0 para ela) até o `azimuth_cloud.c` seguir a pasta nova dos saves. Na 0.9.7 nada muda, mas o código foi reorganizado, então o .35 precisa de teste.
- **Próximo:**
  1. Testar o .35 na 0.9.7 (PC e Deck).
  2. Rodar a 0.9.9 uma vez numa pasta nova, com cópia dos saves, e ler no `data\logs\azimuth.log` a pasta do jogo.
  3. Fazer o `azimuth_cloud.c` usar essa pasta na 0.9.9, ligar a 0.9.9 e aceitá-la nos instaladores.
  4. Release só depois do teste.

## Estado em 2026-10-15, madrugada (servidor público no GCP, fila de 150 vagas)

- **Servidor público:** `https://infinity.anarqz.dev`, banco vazio. Detalhes e operação em `infra/README.md`.
  - GCP: projeto `forza-infinity-online` (org 14mb.io, faturamento em BRL, alerta de orçamento de R$ 30). VM e2-micro em us-east1, sem portas abertas: SSH só pelo IAP.
  - Cloudflare: Tunnel até `127.0.0.1:8081`, regra de limite de taxa nas rotas de conta e save, e cache de 15 s no `/stats`.
  - O banco fica num disco próprio (`infinity-data`, com `prevent_destroy`), com snapshot diário (7 dias) e `.backup` diário no bucket (30 dias).
  - Terraform em `infra/` (`tf.sh` usa o login do gcloud e as credenciais da Cloudflare do `.env` da raiz, fora do git). `deploy.sh` compila aqui e instala pelo IAP.
- **Servidor atrás de proxy:** `--behind-proxy` (o proxy local não conta no limite de conexões por IP), IP do cliente pelo `CF-Connecting-IP`, `--registration open|closed`. As respostas podem ter `Cache-Control` próprio (antes era sempre `no-store`).
- **Fila** (`server/src/admission.rs`, `--max-players`, padrão 150, 0 = sem limite):
  - Vaga por conta, liberada depois de 5 min sem pedidos. Fila por ordem de chegada; quem não pergunta por 90 s sai dela.
  - Sem vaga, todo pedido do jogo recebe 503, como sem conta, e o jogo fica offline.
  - `GET /infinity/v1/queue` (com token) devolve a posição, quantos esperam, online, capacidade, previsão e intervalo para perguntar de novo. `/info` e `/stats` trazem `online`, `queued`, `capacity` e `full`.
  - Testes: 3000 contas para 150 vagas; só a cabeça da fila entra.
  - **Ainda sem efeito no jogo:** o hook .33 não conhece a fila; quem espera só vê o jogo offline. O hook .34 deve mostrar a posição. **Pendente:** descobrir se o jogo volta a ficar online sozinho (refaz o `Logon`) quando a vaga sai, ou se precisa reiniciar. As capturas não respondem: todos os 15 `Logon` registrados deram 200. Teste proposto: o servidor local com `--max-players 1`, a vaga ocupada pela conta do Deck, o PC na fila, e depois liberar.
- **Landing:** redesenhada (abertura de festival ao entardecer, avisos de marco, maquetes, números ao vivo, abas de instalação, FAQ). Mostra "N/150 online" ou "Servidor cheio · N na fila". Vai para `https://anarqz.github.io/ForzaInfinity/` (decidido: fica no GitHub Pages). Para publicar: push da branch `landing`, Pages com origem "GitHub Actions" e as variáveis `STATS_URL` e `SERVER_ADDRESS`.
- **Hook .33** instalado no PC e no Deck (SHA `068b67b8…`, backups `.fh2online32`).

## Estado em 2026-10-14, fim da noite (hook fh2online.33 compilado, não instalado)

Plano combinado: tudo na `main`; a branch `landing` publica a página. Release do GitHub e GitHub Pages só quando o Alex mandar.
- **Rotas:** a API de contas passou a `/infinity/v1/...`; `/fh2online/v1/...` continua aceito para os hooks antigos. `info` responde `ForzaInfinity`.
- **Perfil novo a partir do save offline** (hook .33 + servidor): "Create profile" pede só o nome; o hook envia a revisão mais recente de `data/saves/player-1/User_9000000000000` como primeiro save (`azimuth_cloud_upload_offline`); se falhar, apaga a conta recém-criada. Templates de carro e `tools/make_starter_saves.py` removidos.
- **Excluir perfil:** INFINITY › LEAVE INFINITY › Delete profile (dupla confirmação) → `POST /infinity/v1/delete` → `Store::delete_user` apaga tudo do jogador numa transação (inclusive Storefront, amizades nos dois sentidos, mensagens enviadas) e recalcula os contadores dos arquivos de outros. Testado numa cópia do banco: nenhum registro sobra.
- **Limites de save:** 512 KB por envio, 8 MB por arquivo e 16 MB descomprimido, bytes guardados nunca maiores que os declarados (`db/saves.rs`, teste `size_limits`); balde próprio no guard (5 de rajada, 1 a cada 30 s). O hook usa os mesmos limites.
- **`GET /infinity/v1/stats`:** totais do servidor (CORS aberto, cache de 30 s, `db/stats.rs`). "Online" = visto nos últimos 5 min (`RetrieveForUser` marca o `last_seen`).
- **Landing** (`landing/`, React + Vite, visual de painel do Xbox One sem marcas da Microsoft): números ao vivo, instalação em 3 passos, recursos, docs, EN/PT. Workflow do Pages em `.github/workflows/landing.yml` (variáveis `STATS_URL`, `SERVER_ADDRESS`). Para publicar, o servidor precisa de um endereço público com HTTPS.
- **Instaladores** (`release/`): `Install.cmd`/`install.ps1` e `install.sh` (com `curl … | bash`), desinstaladores, modelo do INI. `tools/package_release.sh [--server host:port]` monta `dist/`. `install.sh` e `install.ps1 -Quiet` testados em instalações falsas (o PowerShell 7 está em `~/tools/pwsh`); as janelas do Windows (seletor de pasta, caixa do servidor) não foram testadas.
- **Docs:** README novo (inglês), `docs/INSTALL.md`, `docs/SERVER.md`, `docs/DEVELOPMENT.md`, `landing/README.md`.

Pendente:
- Instalar o .33 no PC e no Deck (jogo fechado; o Deck estava fora da rede) e testar no jogo, com uma conta descartável: criar perfil a partir do save offline e excluir perfil.
- O jogo do PC parou de apresentar quadros às 19:30 UTC (`graphics watchdog`, GPU ociosa, sem erro no hook): compatível com a tela do KDE bloqueada (Wayland para de liberar quadros, v-sync segura o Present). Não é bug conhecido do projeto.

## Estado em 2026-10-14, noite (hook fh2online.32, PC Linux): leia primeiro

O projeto saiu do PC Windows "diablo" para um PC **Debian 12** (RTX 3070, `192.168.68.40`). Servidor e jogo rodam aqui; a tabela "Onde está tudo" abaixo é a do Windows.
- **Jogo:** `Forza Horizon 2/` na raiz do repo (fora do git): `azimuth.exe`, `data/` e `Anthem_1.0.0.64_x64__8wekyb3d8bbwe/`. Atalho não-Steam (appid 3861404636) com **Proton 11.0**; abre com `steam steam://rungameid/16584606628276338688`. Conta deste PC: `Anarqz#72178`.
- **Servidor:** `server/run.sh` (equivalente do `run.bat`): `0.0.0.0:8081`, log em `server/logs/`, `--gamedb` do jogo local. Firewall nftables: a 8081 é liberada para a LAN com `nft add rule` (não recarregar `/etc/nftables.conf`: ele começa com `flush ruleset` e apaga as regras do Docker).
- **Steam Deck:** `deck@192.168.68.93`, jogo em `/run/media/deck/SN01T/piShop/Forza Horizon 2`, hook .32, `Server=192.168.68.40:8081`, conta `alex#08723` (amigo de `Anarqz#72178`).
- **Ferramentas:** `~/tools` (JDK 21, Ghidra 12.1.4, Zig 0.15.2), Rust em `~/.cargo`. Projetos Ghidra em `~/re`: `FH2_RE` (azimuth.exe, análise completa) e `FH2_RE_game` (exe do jogo e `Microsoft.Xbox.Services.dll`, sem análise); `~/re/decomp.sh` decompila por endereço (achar o início da função pelo `.pdata`).
- **Sistema:** driver NVIDIA 580 do repositório da NVIDIA (o 535 do Debian não tem `VK_KHR_maintenance5`, exigido pelo DXVK do Proton 11); `xpad` ligado ao receptor 8BitDo Ultimate 2C `2dc8:310a` por regra udev (o kernel 6.1 não reconhece).

Feito e testado no jogo nesta sessão:
- **Hook .31:** save na nuvem no Proton. O Wine não tem XPRESS nem `RtlDecompressBufferEx`, então o hook tem codec próprio (`adapters/azimuth_codec.c`): decodifica XPRESS Huffman (saves antigos e templates) e envia no **método 2** (LZ com varints). Teste: `python tests/codec_test.py <zig> <bundles/sqlite>`. Perfil novo agora cai direto no mundo aberto.
- **Hook .32:** o gravador de clipes do Game DVR (`FUN_14054e560`) retorna na entrada; antes a tarefa PPL pedia `ApplicationClipCapture` e derrubava o jogo (`0xc0000409`) em momentos marcantes das corridas.
- **Servidor:**
  - placares de radar/zona/habilidade por evento (`GetRows`, `pivotId`);
  - `UpdateAll` mescla as seções (o jogo manda só o que mudou; antes apagava o resto e zerava as travas);
  - Meus Rivais como "TIME TO BEAT" do melhor tempo de cada amigo em cada rota (`notifications::sync_rivals`), só corridas por tempo;
  - nomes oficiais das pistas e palavras-chave lidos das tabelas LSB2 do jogo (`strtable.rs`, `stringtables/EN.zip`);
  - filtro de palavras do Xbox (`resultCode` numérico) para criar clube;
  - drivatars dos amigos com a pintura do carro: pintura registrada sem camadas, ou pintura base gerada da cor de fábrica (`base_livery`);
  - Storefront entre amigos (`forza/storefront.rs`, `db/storefront.rs`): compartilhar, RECOMMENDED, busca, baixar, curtidas, downloads (contados pelo carro que passa a usar o conteúdo), meus arquivos, denúncias com 3 strikes de jogadores diferentes.

Pendente:
- **Pinturas com camadas** de outros jogadores: o Azimuth para em `CreatePlacementTexture2D requires GPU-write/CPU-map/unknown placement semantics` (`azimuth.exe` `FUN_1400ea0b0` só aceita dois formatos). Por isso os drivatars levam só pinturas sem camadas. Uma queda no PC às 16:23 em `game+0x10ccd7a` (classe `Material`, recurso nulo) pode ter a mesma origem: o Supra do PC usa o design de 4 camadas baixado do Alex. Dumps em `Forza Horizon 2/data/logs/`.
- **Online ao vivo (V2):** ver M7 no `MILESTONES.md`. A V1 fica no assíncrono.

## Estado em 2026-10-14 (hook fh2online.28): leia primeiro

Funcionando e testado no jogo (PC):
- Conta Infinity (`Nome#12345`), token por PC, login em outro PC com nome + chave do site (o PC antigo é desconectado), save na nuvem (`azimuth_cloud.c`: upload a cada revisão nova, download no boot).
- Servidor endurecido (`guard.rs`: limites de conexão/tamanho/taxa; painel só localhost; identidade só pelo `X-Fh2-Token`; tudo restrito a amigos; sem rankings públicos). `server\run.bat` escuta em `0.0.0.0:8081` e espelha o log em `server\logs\`.
- `azimuth_ui.c` edita em memória o que o jogo lê do zip (`FUN_1411ecbd0`, desvio com trampolim):
  - Configurações: CRÉDITOS (original) + INFINITY em 1x1;
  - pausa: linha de status centralizada no topo;
  - tela INFINITY: a própria página de Configurações relida com outros tiles (PROFILE, ADD FRIEND, TEST CONNECTION, SITE KEY, SWITCH PROFILE, ABOUT, LEAVE INFINITY).
- Navegação para o INFINITY: a página só aceita as saídas que a classe registra (`FUN_140da7720`), então o tile usa `team_credits` e o `pause.xml` desvia por um estado `BoolSwitchSticky` com nome literal `Fh2InfinityOpen` (respondido pelo hook do slot `+0x20` da vtable `0x142d29a90`). Ida e volta passam pela cena com spinner, para forçar a página a ser relida. Foco inicial: slot `+0x498` da vtable de Configurações.
- Textos do Infinity (hook e mensagens do servidor visíveis no jogo) em inglês. O painel web segue em português.
- Teste das edições de XAML/fluxo: `python hook\tests\ui_edits_test.py <zig> <Mount>` (compila as funções reais do `azimuth_ui.c` e valida o XML).

- Perfil novo (fh2online.29, não testado no jogo ainda): "Create profile" pede o carro inicial (Camaro SS 69 = 289, Supra RZ 98 = 461, BMW Z4 11 = 1218; o servidor recusa outros) e o servidor grava como save na nuvem o template `server/templates/starter-<carro>.bundle`, gerado por `tools/make_starter_saves.py` a partir do save offline (depois do 1º campeonato, 50.000 CR, `IntroducedMultiplayer` ligado para pular o vídeo do online). O boot seguinte baixa esse save. Formato: `SaveState` XML; `PlayerDatabase` = `ssmc` + u32 tamanho + SQLite + zeros até 6.291.464; `ProfileData` com campos binários (Credits, MostValuableCar).
- Criação de perfil testada e funcionando no jogo (2026-10-14).
- Linha de status da pausa (fh2online.30): o jogo monta a página de pausa uma vez e a guarda, então o texto vem de uma string do jogo (a mais longa da tabela `KinectSplitscreen`, nunca exibida no PC) que uma thread reescreve a cada 5 s com o estado do servidor. Falta confirmar no jogo que o texto muda sem reiniciar.
- Travas de integridade no servidor (`server/src/forza/gates.rs`, `db/integrity.rs`): crescimento de créditos/ganhos (250 mil + 500 mil por hora), XP (150 mil + 400 mil/h), nível (3 + 10/h), recorde de velocidade acima de 1,3× o carro mais rápido da garagem, tempo de corrida impossível para o carro, carros inexistentes, muitos carros de uma vez, XP reportado acima de 400 mil/h. Um sync recusado não é gravado (o jogo recebe OK) e gera um strike, visível no perfil do painel (`integrity`).

Pendente:
- Testar o perfil novo no jogo: este PC já tem conta (uma por PC), então é preciso apagar a conta de teste `Alex#72178` no servidor e o `Token` no `FH2Online_Accounts.ini`, ou testar num PC/Deck sem conta.
- Steam Deck (`deck@192.168.68.93`, jogo em `/run/media/SN01T/piShop/Forza Horizon 2`, perfil `anarqz#08723`) ainda com fh2online.17.
- O perfil de teste `Alex#72178` tem uma cópia do progresso offline (do teste do save na nuvem); backup em `FH2\saves-backup-20261014-131022`.
- Regras de trabalho com o usuário: não controlar teclado, controle nem o jogo dele; cópias de arquivo, builds e reinício do servidor são permitidos.

## Objetivo

Fazer o Forza Horizon 2 (Xbox One, rodando no PC pelo Azimuth 0.9.7) voltar a ter serviços online, com um backend nosso em Rust.
- Não reimplementamos o Azimuth. Fazemos hook nele, com a mesma estratégia do ExtraOptions: o `azimuth-tls.dll` é substituído por um fork do ExtraOptions.
- Primeiro capturamos o que o jogo envia e espera. Depois respondemos a partir do servidor de testes.

## Onde está tudo (PC "diablo", usuário alcmo)

| Caminho | O que é |
|---|---|
| `Downloads\GAMES\FH2\azimuth-windows\azimuth.exe` | Azimuth 0.9.7 (TimeDateStamp `0x6acb70a6`, SizeOfImage `0x352000`) |
| `...\azimuth-windows\data\bin\` | DLL instalado (`azimuth-tls.dll`), `azimuth-tls.dll.stock` (original, 4,6 KB), `FH2_ExtraOptions.ini`, `FH2_ExtraOptions.log`, `FH2_Network.jsonl` |
| `...\azimuth-windows\data\logs\azimuth.log` | Log do próprio Azimuth (sessões, sem detalhe de rede) |
| `Downloads\GAMES\FH2\ForzaHorizon2\Mount\forza_durango_release_final.exe` | Jogo (60 MB, SHA-256 `3ec04c61…cb7f0a3`, que o Azimuth exige) |
| `Downloads\GAMES\FH2\ExtraOptionsRelease\` | Release original do ExtraOptions 0.2.34 (INI e Gamerpic de referência) |
| `Downloads\GAMES\FH2\ExtraOptions\` | Clone do upstream (RokkuDayo/ForzaHorizon2_ExtraOptions) |
| `Downloads\GAMES\FH2\FHPC\` | Projeto antigo (reimplementação do Azimuth). Abandonado, mas `docs\loader.md` documenta o boot do Azimuth |
| **`Downloads\GAMES\FH2\FH2Online\`** | **Este projeto** |
| `FH2Online\hook\` | Fork do ExtraOptions, branch `fh2online`, git completo. O módulo novo é `adapters\azimuth_net.c` |
| `FH2Online\server\` | `fh2-server` em Rust (só std + serde_json + base64), git próprio |
| `FH2Online\bin\` | DLLs compilados (`azimuth-tls-fh2online5.dll` é o atual) e `network-section.ini` |
| `FH2Online\captures\` | Capturas guardadas |
| `FH2Online\notes\` | Descompilações do Azimuth (`decomp\*.c`), JSONs do REA, scripts do daemon REA |
| `Downloads\Ghidra\` | `ghidra_12.1.4_PUBLIC`, `jdk21`, `zig\zig.exe` (0.15.2), `uv` (ghidra-mcp), .bats de setup |
| `C:\Users\alcmo\FH2_RE` | Projeto Ghidra com o `azimuth.exe` analisado. O CodeBrowser foi salvo com o GhidraMCPPlugin, e a ponte sobe com `Downloads\Ghidra\start-ghidra-mcp.bat` (HTTP :8080) |

## O que já está provado

1. **O hook HTTP funciona.**
   - O Azimuth implementa o próprio `CoCreateInstance` (RVA `0x6dbe0`). Para `CLSID_FreeThreadedXMLHTTP60`, ele devolvia um `OfflineRequest` (erro `0x800C0005`).
   - Nós escrevemos um jump de 14 bytes nesse ponto, depois de conferir o SHA-256 dos 32 primeiros bytes (`bdac0438…cbec`). O XHR passa a ser o nosso `IXMLHTTPRequest2`.
   - O MMDeviceEnumerator continua indo para a fábrica do Azimuth, em RVA `0x144400`. Qualquer outro CLSID recebe `REGDB_E_CLASSNOTREG` e vai para o log.
2. **O bloqueio online era o token.**
   - O `User.GetTokenAndSignatureAsync` do Azimuth (RVA `0x960b0`) devolve uma operação async já falha com `0x800704CF`. Sem token, nem a XSAPI nem o jogo montam a requisição.
   - Agora as 6 entradas de vtable do `User` (RVAs `.rdata` `0x24ff90`, `0x24ff98`, `0x24ffa0`, `0x250290`, `0x250298`, `0x2502a0`) apontam para thunks nossos. Eles devolvem um `IAsyncOperation<GetTokenAndSignatureResult>` já concluído.
   - O resultado tem dois getters de HSTRING: **getter 0 = Signature, getter 1 = Token**, confirmado por captura. As HSTRINGs são criadas com o `WindowsCreateString` do Azimuth (RVA `0x86110`).
   - A posição do parâmetro de saída por slot é e/g/f/e/g/f, para as assinaturas (method, url, headers, out), (…, bodyLen, body, out) e (…, body, out).
3. **O caminho jogo → hook → servidor Rust funciona** (`Mode=Proxy`, `Server=127.0.0.1:8081`). Alex rodou e o servidor recebeu tudo.
4. **Classes WinRT.**
   - O `GetActivationFactoryByPCWSTR` do Azimuth (RVA `0x6da50`, também com hook nosso para trace) tenta 7 provedores. Os RVAs são `0x143020`, `0x2cf50`, `0x1431e0`, `0x93bd0`, `0x144250`, `0x1454b0` e `0x14a310`.
   - Todas as 20 classes que o jogo pede são atendidas, inclusive `Windows.Networking.Connectivity.NetworkInformation`. A rede não é bloqueio.
5. **O usuário que o Azimuth inventa:** XUID `0x0009000000000000` (2533274790395904), hash `"1"`, `IsSignedIn = true`, `IsGuest = false`, gamertag vinda do `player_name` do `azimuth.ini` ("Player 1").

## Tráfego que o jogo gera hoje (até o menu, cerca de 1 minuto)

| Requisição | Situação |
|---|---|
| `POST https://gameservices.FH2.forzamotorsport.net/Services.FH2Release/o.xtsw`, com `X-ClassName: Forza.WebServices.User, Forza.WebServices` e `X-Method: Logon` | **Próximo alvo.** Corpo binário de 111 bytes. Repete enquanto não recebe sucesso |
| `GET sessiondirectory.xboxlive.com/serviceconfigs/788f0100-a4bc-46c3-b19b-d1001a96545b/sessiontemplates/defaultTemplate_qos/sessions?xuid=…&private=true&take=32` (contract 105) | Stub `{"results":[]}` |
| `GET achievements.xboxlive.com/users/xuid(…)/achievements?titleId=446059611&types=challenge&unlockedOnly=true&maxItems=20` (contract 2) | Stub com lista vazia + pagingInfo |
| `POST gameserverds.xboxlive.com/titles/1368890819/variants`, corpo `{"locale":"en-us","maxVariants":8,"publisherOnly":false}` | Stub `{"variants":[],"variantSchemas":[]}` |
| `GET music.xboxlive.com/1/user/music/profile` | Stub chutado |

Os stubs ficam em `server\responses\hosts\<host>\_default.json` e **ainda não foram testados com o jogo**.

Headers do Forza:
- `Content-Type: bin/xtsw`, `User-Agent: Xls Client`
- `X-TitleID: 446059611`, `X-UserId: 0x9000000000000`, `X-CorrelationId: <n>`
- `Authorization` e `Signature`, vindos do token falso
- O jogo manda `Host: 127.0.0.1` por conta própria

Corpo do `User.Logon`, hex, da captura `captures\2026-10-13_first-capture.jsonl`:

```
00 09 00 00 00 00 00 00   XUID u64 big-endian
00 00 00 08 "Player 1"    string: u32 BE length + bytes
00 … 00 16 … 00 06 … 00 00 00 03 00 00 00 18 <24 bytes> …
```

## Sessão 2 (2026-10-13, local no PC): esquema extraído e `User.Logon` implementado

- **`tools/extract_schema.py`** gera `server/schema.json` a partir do exe em menos de 1 s: 571 tipos, 30 classes, 201 métodos (stub + inferência pelo nome do tipo). Só precisa de `pefile`.
  - Python: `C:\Users\alcmo\AppData\Roaming\uv\python\cpython-3.12-windows-x86_64-none\python.exe` (gerenciado pelo uv; crie um venv com `Downloads\Ghidra\uv\uv.exe venv` e `uv pip install pefile`). O `python` do PATH é o alias da Store.
  - Tipos sem classe no exe (código morto): DLCMetadata, GiftPhoto, GiftReplay, LargeFileUpload, SharedCar.
- **Formato `bin/xtsw` todo documentado em `notes/xtsw-format.md`**, lido do próprio leitor do jogo (decompilações em `notes/decomp-game/`). Resumo: big-endian, posicional, string/blob/array com u32 de tamanho; resposta = `[u32 status][u32 code][u32 msgLen]` + `_Results` (status 0 = sucesso).
- O Logon de 111 bytes da captura bate byte a byte com o esquema (teste em `server/src/xtsw.rs`).
- **Servidor:**
  - `src/xtsw.rs` faz o codec guiado pelo `schema.json` (embutido no binário com `include_str!`).
  - `src/forza.rs` tem os handlers embutidos. O primeiro é o `Forza.WebServices.User.Logon`, que ecoa XUID/gamertag/nível e grava `data/users/<xuid>.json`.
  - Toda requisição Forza tem os `_Parameters` decodificados em `request_xtsw` na captura, e a resposta enviada vai em `response_xtsw`.
  - Os specs aceitam `"xtsw": {...}`, que vai mesclado sobre o handler embutido (campos ausentes = zero), e `"xtsw_error": {code, message}`.
  - As pastas agora são `responses/forza/<Classe>/` para Forza.WebServices e `responses/xls/<Classe>/` para Xls.WebServices. Nas regras de `routes.json`, `class` é o nome completo (`Forza.WebServices.User`).
- **`tools/xtsw.py`** é um decodificador independente. `--replay <captura>` reenvia as requisições Forza ao servidor e decodifica requisição e resposta. Foi testado ponta a ponta: Logon → HTTP 200, 267 bytes, decodificado sem sobra.
- O Ghidra headless funciona no exe do jogo sem análise completa: `C:\Users\alcmo\FH2_RE_game` + `notes/ghidra-scripts/Decomp.java`, que decompila ~1200 funções em segundos.

**Teste 1 com o jogo (2026-10-13 19:32):**
- O Logon passou. O jogo seguiu para `OnlineProfile.UpdateAllTables`/`UpdateAll`, `Storefront.*`, `User.GetCompareStatsForRelatedUsers` e `User.GetEconomySettings`, todos ainda 503.
- Depois houve um crash em `game+0x41fd28`, ao ler 0x0 (dump em `azimuth-windows\data\logs\20261013T223229-*.dmp`).
- Causa: o XSAPI copia o corpo de **toda** resposta para um `Windows.Storage.Streams.DataWriter` criado com `ActivateInstance` (`WriteBytes` e depois `DetachBuffer`). Esse código é o `FUN_1800378b0`/`FUN_180038b30` em `Microsoft.Xbox.Services.dll`.
  - Se o buffer sai vazio, `FUN_180023f40` não cria resultado: a operação assíncrona termina "Completed" com resultado nulo.
  - O código de challenges do jogo (`FUN_140415030`) não checa nulo e quebra. Com 503 não quebrava, porque a operação termina em Error.
  - O Azimuth não tem `IDataWriter` próprio: a classe sai de uma tabela genérica. Isso explica por que nenhum JSON do Xbox Live chegava ao jogo (o `DataReader` nunca foi ativado).
- Correção no hook (`fh2online.6`, commit `1e99ad7`): um autoteste do `DataWriter` do Azimuth roda na primeira ativação. Se falhar, o hook entrega o seu próprio `DataWriter`, que gera o resultado num `Buffer` do Azimuth. O log mostra `WINRT: DataWriter self-test: ...`. **Ainda não foi testado com o jogo.**
- Teste 2 (`.6`): confirmou `Azimuth's writer loses the bytes`. Com o writer substituído, o `DataReader` passou a ser ativado, mas o crash continuou: o `DataReader` do Azimuth também é genérico, e o `ReadString` sai vazio.
- `.7` (commit `fb55dab`): também substitui o `DataReader` (`IDataReaderStatics.FromBuffer` e um leitor completo). As HSTRINGs são criadas com o `WindowsCreateString` do Azimuth (RVA `0x86110`). O log mostra `WINRT: DataReader self-test: ...`.
- Teste 3 (`.7`): o filtro do `DataReader` não disparou porque o IID estava errado (`...e3 49 3c 3c`; o certo é `...e3 49 31 3c`). Corrigido no `.8` (commit `e045df7`, SHA-256 `e8b9bafb…0079`). **Ainda não foi testado com o jogo.** Lição: conferir os IIDs pelas constantes do próprio binário, não de memória.
- O projeto Ghidra `C:\Users\alcmo\FH2_RE_game` agora também tem `Microsoft.Xbox.Services.dll` (sem análise).

- **Teste 4 (`.8`): funcionou.** Os dois autotestes trocaram o writer e o reader, e o Alex fez uma corrida inteira sem crash. Os stubs JSON do Xbox Live (achievements, variants, qosservers, sessiondirectory, music) agora chegam ao jogo.
- Métodos Forza pedidos nessa sessão (`server-1791932302627.jsonl`, todos 503 menos o Logon):
  - `Storefront`: GetLiveriesForDrivatar (44x, em loop), GetFileKeywords, GetFeaturedLiveriesForRandomCars;
  - `User`: GetEconomySettings, GetCompareStatsForRelatedUsers, GetUserData, GetOwnedProducts;
  - `OnlineProfile`: UpdateAll, UpdateAllTables;
  - `CarClub`: GetRecommendedClubs, GetClubKeywords;
  - `Scoreboard`: GetBestScore, SubmitScore, SubmitXpScore, GetRowsAnthem, GetRival;
  - `Notifications.RetrieveForUser`, `GameplayLog.Write`, `ContentVersioning.CreateVersionedTuneFromStructs`, `Drivatar.GetDrivatarsForCache`, `DailyCreditBonus.GetPayout`;
  - no Xbox Live, `eds.xboxlive.com/media/.../fields` também respondeu 503.
- O `schema.json` foi gerado de novo (commit `ad724d3`, kind 0x13 = f64). Antes disso, o `SubmitScore` não decodificava.

## Sessão 3: perfil em SQLite + manager web ao vivo (commit `1231cca` no server)

- **Banco:** `server/data/fh2online.sqlite`, temporário (pode apagar: o jogo reenvia o perfil). Tabelas:
  - `users`;
  - `profile` (o último `UpdateAll`: créditos, nível, XP, histórico de corridas, stats);
  - `profile_tables` (`UpdateAllTables` em deltas: cada tabela é um mapa por id, com os `delete*` aplicados; a garagem é a `garageCars`);
  - `scores`, `xp`, `tunes`, `events` (o feed).
- **SQLite no toolchain GNU:** `rusqlite` com `bundled`, e o C compilado pelo `zig cc` via `server/.cargo/config.toml`. O `CRATE_CC_NO_DEFAULTS` é necessário porque o cc-rs passa `--target=x86_64-pc-windows-gnu`, que o zig não aceita.
- **Handlers com banco** (`forza.rs`):
  - Logon, que devolve como checkpoint o `transactionId` guardado;
  - Xls `User.GetUserData`;
  - `OnlineProfile.UpdateAll`/`UpdateAllTables`;
  - `Scoreboard.SubmitScore`/`SubmitXpScore`/`GetBestScore`/`GetRowsAnthem` (hipótese: scoreType 1 = tempo, menor é melhor);
  - `ContentVersioning.CreateVersionedTuneFromStructs`, `GameplayLog.Write`, Xls `Notifications.RetrieveForUser`, `DailyCreditBonus.GetPayout`;
  - listas vazias para storefront, clubes e drivatares;
  - `profile.xboxlive.com/users/batch/profile/settings`.
  - **Ficam 503 de propósito:** `User.GetEconomySettings` (zeros podem zerar recompensas; achar os padrões no exe), `Scoreboard.GetRival` e `Drivatar.GetDrivatarInformationForRace`.
- **Catálogo** (`gamedb.rs`): o `ForzaHorizon2\Mount\media\stripped\gamedbRC.slt` é SQLite puro. `Data_Car` + `List_CarMake` + `CarClasses` + `Tracks` dão nomes como "Toyota Supra RZ 1998", classe e PI. Os nomes oficiais (`_&<hash>`) estão nos `stringtables\*.zip` (`.str` em formato `LSB2`, compactado, ainda não lido). As miniaturas estão nos `RC*.zip` como `.swatchbin` (textura Xbox).
- **API e ao vivo:** `/api/users`, `/api/users/<xuid>`, `/api/catalog`, `/api/status`, `/ws` (WebSocket próprio, RFC 6455, que manda `request` e `event`) e `/ui/` (a build do React). Requisições com `X-Fh2-Original-Url` sempre vão para o caminho do jogo.
- **Interface** em `FH2Online/web`: React 19 + Vite 8 + TS 7.
  - `web\build.bat` gera o `web\dist`, servido em `http://127.0.0.1:8081/ui/`. O `web\dev.bat` usa a porta 5173 com proxy para o 8081.
  - O Node portátil está em `Downloads\Tools\node-v24.21.0-win-x64` (checksum conferido).
- **Validado** reenviando a sessão real (`tools/xtsw.py --replay`, que agora manda o `X-UserId` e tolera 503): perfil, garagem, score, XP e tune foram gravados, o WebSocket recebeu 82 requests e 9 eventos, e a interface foi conferida por screenshot (Edge headless).
- **Correções depois do primeiro teste do manager** (commit `1ec54ef`):
  - **WebSocket:** o GUID do RFC 6455 estava digitado errado (`…C5AB0DC11B65`; o certo é `…C85B11`), e todo navegador recusava o handshake. Os meus testes em Python e Node usavam o mesmo GUID errado e "passavam". Agora há um teste com o exemplo do próprio RFC.
  - **Crash `game+0x124f916`:** veio do handler de `profile.xboxlive.com` (`GameDisplayPicRaw: ""` vira Uri nula no XSAPI, e o jogo não checa). O código agora manda URLs absolutas, **mas o `responses/routes.json` continua respondendo 503 para esse host** até ser testado.
  - **Nível:** o perfil guarda o nível a partir de 0, e o jogo mostra `level + 1` (conferido: o mesmo snapshot tinha 120.866 CR, igual à tela, com nível 2 contra 3 no jogo). `compareStats.wristbandLevel` é na verdade o XP.
  - Os números do manager só mudam quando o jogo manda o `UpdateAll` (fim de corrida, menus); entre um envio e outro, ficam defasados em relação à tela.
- **Gamerpic** (commit `13cff6b`): `GameDisplayPicRaw` aponta para `http://gamerpic.fh2online/pic?xuid=…`, e o próprio servidor responde a imagem. A ordem é:
  1. `data/gamerpics/<xuid>.png`;
  2. o avatar público do Xbox 360 pelo gamertag (`avatar-ssl.xboxlive.com/avatar/<gt>/avatarpic-l.png`, que ainda funciona sem login; baixado com o `curl.exe` e guardado em cache);
  3. um identicon pelo XUID.
  - O "Player 1" do `azimuth.ini` é um gamertag real de outra pessoa.
  - O `routes.json` com 503 virou `routes.disabled.json`. Se o crash voltar, é só renomear de volta.
- **Transações** (commit `be5fa42`): o `User.RetrieveOnlineCredits` vai com `X-TransactionId`. Com 503, toda sessão deixava uma transação pendente, e o jogo repetia o `Xls Transaction.Reconcile` sem parar (30 vezes por sessão).
  - Agora o servidor responde `credit: 0` e guarda a resposta exata. O `Reconcile` devolve essa resposta em `WebRequestPayload`, o corpo inteiro com o envelope, que o cliente processa como uma resposta atrasada (`FUN_140efce10`). Para uma transação desconhecida, devolve o erro Xls 6 (`0x80510006`), e o cliente encerra a transação (`FUN_140f04c80`).
  - **Hipótese ainda não confirmada:** a transação pendente segura o `OnlineProfile.UpdateAll`. Na sessão das 21:05, o perfil só subiu no login, enquanto XP e pontuação subiram depois das corridas.
- **Confirmado no jogo** (sessão das 21:21): com o `Reconcile` respondido, o `OnlineProfile.UpdateAll` passou a subir depois das corridas, e o `profile.xboxlive.com` com a URL nova não derrubou mais o jogo.
- **Commit `0e397cb`:**
  - `UpdateLivery`/`CreateVersionedLivery` gravados na tabela `liveries`;
  - `GetRival` → erro Xls 1 ("sem rival");
  - `Storefront.GetCuratedContent`/`GetMyFileList` → listas vazias;
  - `Drivatar.GetDrivatarInformationForRace` → lista vazia, **mas o `routes.json` mantém 503 até ser testado**.
- **`GetEconomySettings` resolvido** (servidor commit `4d174bb`; hook `0.2.34+fh2online.10`, commit `0dc4dcd`):
  - O getter de `CreditsPerKm` nunca é chamado, porque o C++ lê o campo direto. Por isso a sonda passou para a entrada de `FUN_140561000` (`game+0x561000`): trampolim em `VirtualAlloc`, com 16 bytes sem RIP-relativo, que despeja o módulo uma vez no log.
  - Valores reais: CreditDestinationBonus 30000, Falloff 2000, XPPerKmOffline 800, XPPerKmOnline 640, CreditsPerKm 960, MinimumFinishlineXP 0, SubmitXpFrequency 300000 ms, Tombola 1.0, Promo 1.0, Tokens 0, Music 0, **UseServerValues 1** (o jogo usa o que o servidor manda), RetrievalAttemptInterval 5 s, MaxRetry 120 s, Backoff 2.0.
  - Cada propriedade ocupa um slot de 0x20 com o valor em +8. Ponteiros de tipo: int `0x142c91c20`, float `0x142c6f280`, bool `0x142c6f268`.
  - O servidor responde com esses mesmos valores, e dá para ajustar pelo spec `forza/User/GetEconomySettings.json` (`xtsw`).
  - As duas sondas continuam no hook. São inofensivas e podem sair numa próxima versão.
- **Drivatares e rival** (commit `240bb67`, ainda não testados no jogo):
  - `SaveDrivatarInformation` → tabela `drivatars`.
  - `GetDrivatarInformationForRace`: `playerData` traz o drivatar do próprio jogador, e `drivatarInfos` traz os drivatares dos **outros** jogadores. O carro é o mais corrido da garagem dele dentro do bucket ou, se não houver, o herói do bucket (`Data_Car_Buckets`).
  - `GetRival`: rival aleatório em torno do tempo do jogador. `ghostFileId` vai 0.
  - Testado com dois jogadores no servidor de teste, com o `tools/xtsw.py`, que agora também codifica (`encode`).
- **Primeiro teste no jogo do drivatar:** `GetDrivatarsForCache` e `GetDrivatarInformationForRace` → 200 com listas vazias, e o grid foi montado com a IA offline (pilotos sem nome), sem crash.
- **Fantasmas implementados** (commit `2ffbd6b`, testados fora do jogo):
  - O `SubmitScore` só pede o upload (`uploadFiles`) para um novo recorde pessoal no evento.
  - A tabela `ghosts` guarda um fantasma por jogador e evento (chave: tipo de placar, tipo de pontuação, evento e pista). O `UploadFile` aceita o fantasma só se ele ainda for o recorde, e substitui o anterior mantendo o mesmo id.
  - O `DownloadFile` responde pelo id (máximo de 512 KB, limite do leitor do jogo), e o `GetRowsAnthem` informa o `ghostFileId`.
  - Os pedidos agora aceitam blobs de até 32 MB (`MAX_BLOB_IN`).
  - **Visto no jogo (14/10/2026), depois que a tabela de permissões foi preenchida:**
    - Uma corrida de rival termina com `SubmitScore`, que devolve `uploadFiles: true`, seguido de `UploadFile`. O blob tem cerca de 27 KB: cabeçalho de 16 bytes (`<u32 ?, u32 tamanho descompactado, u32 tamanho compactado, u32 4096>`) e depois blocos zlib de 4 KB.
    - Na corrida seguinte, o jogo baixa o fantasma com `DownloadFile(id, 256000)`.
    - Um recorde novo substitui o fantasma e mantém o id 1.
  - **Rivais com fantasma:** quando o rival não tem fantasma próprio, o `GetRival` usa o fantasma gravado mais próximo na mesma rota. Vale tanto para o desafio quanto para o rival gerado, que nesse caso adota o tempo e o carro do fantasma, desde que a classe seja a mesma pedida em `pivotId` (`Store::nearest_ghost`).
  - **O carro-fantasma aparece na pista** (Alex confirmou num print). Ele é translúcido e difícil de ver nos prints reduzidos. Com a IA dirigindo, ele fica sobreposto ao carro do jogador, porque segue a mesma trajetória.
    - Estado no `CGhostCarComponent`: `+0x140` é o handle do carro "LiveGhost", `+0x148` é o tempo do fantasma (double) e `+0x151` fica em 1 quando o carro foi criado.
    - A criação passa por `FUN_14098ed40`, que faz o parse com `FUN_1403490b0` e cria o carro com `FUN_1413c8c10`.
- **Automação de testes sem ninguém no PC (hook fh2online.14):**
  - Controle virtual: `tools/pad.py A B DOWN wait:2000 hold:RT:3000 stick:LX:-32767:500 off`.
    - O hook troca o `XInputGetState` do Azimuth (ponteiro em `azimuth+0x319520`) por uma versão que lê a memória compartilhada `Local\FH2Online_Pad`.
    - Enquanto `active` estiver em 0, o controle de verdade funciona normalmente.
  - Captura de tela: `tools/shot.py saida.png [divisor]`.
  - Tecla: `tools/key.py F2`.
  - `[Autodrive] InEvents=1`: o F2 entrega o carro à IA nativa também dentro de eventos. Foi assim que o rival da rota 897 foi corrido e vencido sem ninguém no controle.
  - Caminho até o desafio: START, RB×4 (aba Rivais), A (Meus Rivais), A (Ir para), RIGHT, A (carro), A (iniciar) e então F2.
- **Fantasmas (como o protocolo funciona):** o protocolo já prevê. O `SubmitScore` devolve `uploadFiles`; se for true, o jogo chama `Scoreboard.UploadFile(scoreId, blob)`. As linhas de placar e de rival trazem `ghostFileId`, que o jogo baixa com `Scoreboard.DownloadFile(id, maxSize)`.
- **Histórico, antes da sonda: `GetEconomySettings` ficava 503.**
  - As propriedades ficam no `ForzaEconomyModule`: o registro está em `FUN_14053bc10`, os getters em `0x14054xxxx`, e os valores no objeto em `+0x208`… `+0x348` (passo de 0x20). Os nomes são `CreditDestinationBonus`, `CreditDestinationFalloff`, `XPPerKmOffline`, `XPPerKmOnline`, `CreditsPerKm`, `MinimumFinishlineXP`, `SubmitXpFrequency`, `TombolaPrizeScale`, `HorizonPromoBonusScale`, `TokensEnabled`, `XboxMusicEnabled`, além de `UseServerValues` (`+0x1e8`) e `DownloadedEconomyValues` (`+0x351`).
  - Os padrões não estão em XML (o `modules.xml` só lista o módulo) nem no gamedb: devem vir do construtor C++, que não foi localizado.
  - A varredura da memória do jogo rodando (`memscan2.py`) não achou a instância.
- **Menu online travado ("RECURSO BLOQUEADO… servidor de Forza não está disponível"): duas condições** (2026-10-13 23h):
  - O popup `FUN_140ddd630(menu, motivo)` usa o motivo 0 = `IDS_Popup_FeatureLocked_LSPDown_Message`. O menu principal (`FUN_140db4f60`) exige `CLobby+0xf0` (CLobby em `*0x1434d6348`) **e** o LSP no ar (`FUN_14126c4c0`: `sessionObj+0x74 == 2 && +0x1131`; sessionObj = `*0x14367a6d0`).
  - **LSP/permissões:** o PermissionsLoader fica embutido em `sessionObj+0x80` (estado em `+0x90`; 4 = carregado). O `FUN_14126c260(sess, área)` testa `+0x1131` e depois a máscara de áreas banidas (`loader+0x10ac`). O servidor agora responde `User.GetPermissionsTable` (commit `714549a`), com permissões vazias e máscara 0. Com o servidor no ar, o estado ficou OK (`+0x1131=1`, loader 4).
  - **Permissões por método:** a lista `permissions` não pode vir vazia. O `FUN_14126c2b0(sess, methodId)` procura o método na tabela (máximo de 0x100 entradas) e devolve o campo `access`; quando o método não está na tabela, devolve 4. Com acesso 0, o jogo libera; de 1 a 5, mostra um popup:
    - O acesso 4 dá "RECURSO INDISPONÍVEL / O servidor do FH2 não está disponível". Esse era o erro do "Ir para" em Meus Rivais (`FUN_140d894a0`, com o método em `page+0x464`).
    - Os rivais exigem os métodos 0x51, 0x54, 0x59 e 0x5b liberados (`FUN_140a41c90`). Sem isso aparece `PopupRivalsServerError` ("ERRO NO SERVIDOR") ao correr contra um rival.
    - O servidor agora devolve os ids de 0 a 255, todos com `access` 0 e `isEnabled` true.
  - **CLobby+0xf0 = "conectado ao Xbox Live"**, definido em `FUN_140a2b900` como `FUN_140a2fa40(CLobby+0xe8) == 2`. Esse estado é `NetworkInformation.GetInternetConnectionProfile().GetNetworkConnectivityLevel() == 4` (XboxLiveAccess, existe só no console). O Azimuth (provider 2) nunca devolve 4, então a flag fica em 0. Correção no hook `.11` (commit `02293df` no fork): shim de `INetworkInformationStatics` e `IConnectionProfile` que repassa tudo ao Azimuth e só sobe o nível para 4. O log registra `WINRT: connectivity level N ... reported as 4`.
  - O Azimuth também stuba como offline as APIs de multiplayer real: `Windows.Xbox.Networking` (SecureDeviceAssociation, QoS), `Windows.Xbox.Multiplayer` (party) e privilégios da loja ("Restricted"). Veja as strings em `azimuth.exe` perto de `0x267b00` e `0x2692a8`. Multiplayer de verdade vai exigir substituir essas APIs pelo hook.
- **Notificações / centro de mensagens (2026-10-14):**
  - `Xls.Notifications.RetrieveForUser`: cada `XlsNotification` tem um `notificationType` e a struct de parâmetros vem fatiada em sequência no `parameterBlock` (`cbParameters` bytes cada). A tabela tipo → struct está em `FUN_140f15ec0` e documentada em `server/src/db/notifications.rs`.
  - A categoria (aba) de uma notificação é `(type >> 8) & 0x7f`. As páginas do centro de mensagens (`MessageCenterSocial`, `MessageCenterRivals`, `MessageCenterCommunityNews`) só montam as abas com a lista de categorias que vem no **`User.Logon`** (`notificationCategories`, máximo 7), e não pelo `GetCategories`, que o jogo nunca chama. Sem categorias, a página fica em "Recuperando dados" e "Você não tem mensagens", enquanto os tiles do menu mostram as notificações normalmente.
  - O intervalo de busca é `notificationPollIntervalSeconds`, do Logon (0 = 300 s), mais o `nextRetrieveSeconds` de cada resposta. O jogo não chama `StartNewSession`: o `nonce` também vem do Logon.
  - Clubes: o tile fica travado até `ClubsIntroducible`/`IntroducedClubs` = true no `SaveState` (bloco `IESaveState`) do save do Azimuth (`tools/save_flags.py`). O ranking do clube é o `Scoreboard.GetLadderRows`.
- **Conquistas:** não passam pelo servidor. O FH2 desbloqueia via `Windows.Xbox.Achievements` local (Azimuth). Para mostrá-las, o hook precisa interceptar e repassar.

**Próximo:**
1. Alex roda o jogo com o servidor novo e vê o que vem depois do Logon (`captures/server-*.jsonl`, campo `request_xtsw`).
2. Para cada método novo, escrever o handler em `forza.rs`, ou um spec com `xtsw`. `responses/forza/_default.example.json` responde sucesso vazio para tudo, se quiser ver até onde o jogo vai.
3. Descobrir se `permissions` vazio bloqueia métodos, e o que `isProfileValid` dispara.

## Esquema Forza.WebServices: onde começar (sessão 1, já feito)

- O executável do jogo guarda o nome de cada campo logo antes do nome do tipo em `.rdata`:
  - `loginData` em `0x141fa7bf8`, `ForzaUserLogon_Parameters` em `0x141fa7c08`;
  - `responseData` em `0x141fa7c28`, `ForzaUserLogon_Results` em `0x141fa7c38`;
  - também `isVip` / `ForzaUserGetVipState_Results` e `credit` / `ForzaUserRetrieveOnlineCredits_Results`.
- 25 classes e cerca de 185 métodos. O nome da classe na requisição vem como `Forza.WebServices.<Classe>, Forza.WebServices`.
- O framework é o "Xls" da Turn 10. Também aparecem `Xls.WebServices.User`, `Xls.WebServices.Club` e caminhos `/Services.Anthem*/o.xtsw`.
- Pesquisa na internet em 2026-10-13: **não há documentação pública** do `bin/xtsw` nem projeto de servidor privado.
- Base de imagem do jogo: `0x140000000`, que o Azimuth mapeia nesse endereço mesmo. Seções: `.text` `0x140001000`, `.rdata` `0x141dd0000`, `.data` `0x142c47000`.

**Próximos passos:**
1. **Extrator de esquema.** Achar quem aponta para essas strings: ponteiros absolutos de 8 bytes em `.rdata` e `.data`, ou `lea` RIP-relativo em `.text`. Entender o descritor de tipo (lista de campos com tipo e offset) e gerar `schema.json` com todos os `_Parameters` e `_Results`.
2. **Formato.** Validar o Logon de 111 bytes contra o esquema. Achar o código que lê a resposta do `o.xtsw` (procurar xrefs de `bin/xtsw` em `0x14235e2c8` e de `Xls Client` em `0x14235e300`, ambos UTF-16) para descobrir o envelope e o status.
3. **Servidor.** Codec `xtsw` em Rust a partir do esquema, primeiro handler `User.Logon`, depois os demais. Persistência simples (perfil, créditos, VIP).
4. Analisar o executável inteiro no Ghidra headless aqui no PC, que tem hardware para isso. Na nuvem eram só 2 núcleos e 7 GB.
5. Mais adiante: P2P. O Azimuth simula Secure Device Associations e sockets offline (`Windows.Xbox.Networking.SecureDeviceAssociationTemplate`).

## Hook: detalhes

- Código em `hook\adapters\azimuth_net.c`, chamado por `runtime.c` (`azimuth_apply_network`) dentro de `#ifdef FH2_AZIMUTH`.
- `settings.c` aceita `[Network]` com `Mode=Off|Capture|Proxy` e `Server=host:porta`. O parser do INI é estrito: seção ou chave desconhecida faz recusar tudo.
- Versão atual: `0.2.34+fh2online.5`, SHA-256 `974a462f1ff3bdbfc85a96a5f5da878ed8e1a33981b7efa83fabe581458693f0`.
- No modo Capture, cada requisição vai para `FH2_Network.jsonl` e recebe a resposta "offline". No modo Proxy, ela é encaminhada por WinHTTP para `http://<Server>/<host original>/<path>`, com os headers `X-Fh2-Original-Url` e `X-Fh2-Request-Id`.
- `FH2_ExtraOptions.log` mostra o que aconteceu:
  - `NETWORK:` para o hook HTTP;
  - `WINRT: activate "<classe>" provider=N -> hr` para as classes pedidas, sem repetir;
  - `XBL: User.GetTokenAndSignature slot N "<method>" "<url>"` para os pedidos de token;
  - `NETWORK: open #N` para as primeiras 64 requisições.
- Regras do código, herdadas do ExtraOptions:
  - C freestanding (`-ffreestanding -fno-builtin -Werror`), sem CRT.
  - Só importa Kernel32 (`kernel32.def`). Outros DLLs entram por `LoadLibraryExW(..., LOAD_LIBRARY_SEARCH_SYSTEM32)` e `GetProcAddress`.
  - Nada de arrays grandes na pilha, porque `___chkstk_ms` não existe aqui.
  - Arquivos com **CRLF**.
  - O TLS precisa continuar idêntico: template de 336 bytes zerados, alinhamento 16, sem callbacks, exports `azimuth_tls_index` e `azimuth_tls_attached`. Por isso o helper tem que ser C, e não Rust.

### Build (Windows)

```powershell
cd C:\Users\alcmo\Downloads\GAMES\FH2\FH2Online\hook
python build.py C:\Users\alcmo\Downloads\Ghidra\zig\zig.exe --azimuth --build-dir build --output build\azimuth-tls.dll
```

Antes, confira se o `build.py` roda no Windows; o original foi pensado para isso, mas ainda não testamos. Depois:
1. Verifique os imports, que devem ser só KERNEL32, e o TLS.
2. Feche o jogo, porque o DLL fica travado enquanto ele roda.
3. Copie para `azimuth-windows\data\bin\azimuth-tls.dll` **e também para `FH2Online\bin\azimuth-tls.dll`**, porque o `install.bat` reinstala o que estiver lá. Depois **confira o SHA-256 dos dois**. Nesta sessão uma cópia disse que funcionou e deixou o arquivo antigo.

### Servidor

```powershell
cd ...\FH2Online\server
cargo +stable-x86_64-pc-windows-gnu build --release   # ou build.bat
target\release\fh2-server.exe --listen 127.0.0.1:8081 --root .
```

- Grava cada requisição em `captures\server-*.jsonl`.
- Responde a partir de `responses\`, relido a cada requisição. A ordem de busca é:
  1. `routes.json`;
  2. `forza\<Classe>\<Metodo>.json`;
  3. `forza\<Classe>\_default.json`;
  4. `forza\_default.json`;
  5. `hosts\<host>\_default.json`;
  6. `_default.json`, que hoje responde 503.
- Formato de cada arquivo: `{status, headers, json | body | body_b64 | body_file, delay_ms}`.
- Não use tokio nem axum: no toolchain GNU eles exigem o `dlltool.exe`.

## Lições desta sessão

- Troque o DLL só com o jogo fechado (o Windows trava o arquivo em uso).
- O Azimuth fecha o processo quando uma classe WinRT não existe. O nosso trace devolve o erro em vez disso.
- O Azimuth não tem log detalhado. Instrumentar pelo nosso DLL foi o que funcionou.
- A ordem de vtable e de getters foi descoberta mandando valores diferentes em cada um e lendo a captura. Funciona bem para tirar dúvida de ABI.
