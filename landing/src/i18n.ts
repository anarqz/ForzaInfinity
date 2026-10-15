export type Lang = "en" | "pt";
export type Platform = "windows" | "deck" | "linux";

const en = {
  locale: "en-US",
  nav: { features: "Features", stats: "Live", install: "Install", faq: "FAQ" },
  server: { online: "online now", offline: "server offline", checking: "connecting", full: "Server full", inLine: "in line" },
  hero: {
    kicker: "Forza Horizon 2 · on PC",
    title: ["Back on", "the road."],
    body:
      "ForzaInfinity brings Forza Horizon 2's online side back to the PC version on Azimuth. Friends, car clubs, rival ghosts, leaderboards, shared liveries and a cloud save that follows you from your PC to the Steam Deck.",
    cta: "Get ForzaInfinity",
    cta2: "See what's back",
    platforms: "Windows · Steam Deck · Linux · Azimuth 0.9.9",
  },
  toast: {
    label: "Festival milestone",
    lines: {
      races: (n: string) => `${n} races finished`,
      kilometres: (n: string) => `${n} km driven together`,
      designs: (n: string) => `${n} liveries painted`,
      players: (n: string) => `${n} drivers on the server`,
      speedTraps: (n: string) => `${n} speed traps smashed`,
      downloads: (n: string) => `${n} designs downloaded`,
    } as Record<string, (n: string) => string>,
  },
  live: {
    title: "Live from the festival",
    note: "Every number below is the whole server, refreshed every 30 seconds.",
    unavailable: "The server is offline right now: these are the last numbers we saw.",
  },
  stat: {
    players: "drivers",
    online: "online now",
    races: "races finished",
    kilometres: "km driven",
    designs: "liveries painted",
    sharedFiles: "files shared",
    downloads: "downloads",
    clubs: "car clubs",
    friendships: "friendships",
    speedTraps: "speed traps",
    ghosts: "ghosts recorded",
    scores: "scores posted",
    cars: "cars in garages",
    credits: "credits won",
  },
  features: {
    title: "Everything that made Horizon social",
    rivals: {
      tag: "Rivals",
      title: "Chase your friends' ghosts.",
      body:
        "Every friend's best time on each route shows up as a time to beat. Go to the race and you are on track against their ghost, in their car.",
      card: { head: "TIME TO BEAT", route: "Nice Aérodrome Cross Country", go: "GO TO" },
    },
    crew: {
      tag: "Friends & clubs",
      title: "Your crew, back together.",
      body:
        "Add friends by Name#12345 straight from the game, see what they are up to, start a car club with a tag, roles and a notice board.",
      card: { head: "FRIENDS", states: ["In free roam", "In the paint shop", "Offline"], club: "CAR CLUB" },
    },
    paint: {
      tag: "Storefront",
      title: "Paint it. Share it.",
      body:
        "Share your designs, vinyl groups and tunes. Your friends find them on Recommended or by search, download them in one press and like them.",
      card: { head: "RECOMMENDED DESIGNS", title: "Sunset Runner", likes: "likes", downloads: "downloads" },
    },
    cloud: {
      tag: "Cloud save",
      title: "Your progress, wherever you play.",
      body:
        "Your profile starts from your offline save and lives on the server. Pick it up on another PC or on the Steam Deck.",
      card: { synced: "Save synced", from: "PC", to: "Steam Deck" },
    },
    more: [
      { title: "Leaderboards", body: "Speed traps, zones and skill chains among friends." },
      { title: "Drivatars", body: "Friends race you and park at car meets, in their colours." },
      { title: "Fair play", body: "Impossible profile jumps are refused; reports take files down." },
      { title: "Private by design", body: "No public lists: only you, your friends and your club." },
    ],
  },
  install: {
    title: "Up and running in a minute",
    note: "The installer keeps Azimuth's original file as a backup and only touches its data/bin folder.",
    requires:
      "Requires Azimuth 0.9.9 (the latest) or 0.9.7; 0.9.8 is not supported (the game would play offline). The installer checks the version for you. Updating Azimuth puts its own file back: run the installer again after each update.",
    tabs: { windows: "Windows", deck: "Steam Deck", linux: "Linux" } as Record<Platform, string>,
    steps: {
      windows: [
        "Download ForzaInfinity.zip and unzip it.",
        "Double click Install.cmd. It finds Azimuth and asks for the server address.",
        "Start the game: Settings › INFINITY › Create profile.",
      ],
      deck: [
        "Add azimuth.exe to Steam as a non-Steam game and force Proton 11.",
        "In desktop mode, open Konsole and run the command below.",
        "Back in game mode: Settings › INFINITY › Create profile.",
      ],
      linux: [
        "Run Azimuth through Steam with Proton 11 (Vulkan driver with VK_KHR_maintenance5).",
        "Run the command below, or ./install.sh from the zip.",
        "Start the game: Settings › INFINITY › Create profile.",
      ],
    } as Record<Platform, string[]>,
    download: "Download .zip",
    copy: "Copy",
    copied: "Copied",
    server: (address: string) => (address ? `Server: ${address}` : "Ask whoever runs your server for its address (host:port)."),
  },
  faq: {
    title: "Questions",
    items: [
      {
        q: "How does it work?",
        a: "Azimuth runs the Xbox One version of Forza Horizon 2 on PC with its online services switched off. ForzaInfinity swaps Azimuth's azimuth-tls.dll for a hook that sends the game's own online requests to a ForzaInfinity server. The game files are never modified.",
      },
      {
        q: "What do I need?",
        a: "Your own copy of Forza Horizon 2 on Azimuth 0.9.9 or 0.9.7 (not 0.9.8), a server to connect to, and an offline save: play until the game saves once before creating your profile.",
      },
      {
        q: "Will I lose my progress?",
        a: "No. Your online profile starts from your newest offline save, and the offline save stays untouched on your PC.",
      },
      {
        q: "Can I run my own server?",
        a: "Yes: it is one Rust program with SQLite. Build it with cargo, start run.sh (or run.bat) and open TCP port 8081 to your players.",
      },
      {
        q: "What about my privacy?",
        a: "There are no public lists. You see your own data, your friends' and your club's. This page only shows server totals. Delete profile removes everything you have on the server.",
      },
      {
        q: "Is live multiplayer back?",
        a: "Not yet. Online Road Trip and online free roam need a replacement for the original dedicated servers; it is planned for a later version.",
      },
      {
        q: "How do I uninstall?",
        a: "Run Uninstall.cmd (Windows) or uninstall.sh (Linux / Steam Deck): it puts Azimuth's original file back and leaves your saves alone.",
      },
    ],
    more: "Full documentation on GitHub",
  },
  footer:
    "ForzaInfinity is a fan project. It is not affiliated with, endorsed or sponsored by Microsoft, Xbox, Playground Games or Turn 10 Studios. Forza Horizon and Xbox are trademarks of their owners. You need your own copy of the game.",
};

export type Texts = typeof en;

const pt: Texts = {
  locale: "pt-BR",
  nav: { features: "Recursos", stats: "Ao vivo", install: "Instalar", faq: "Dúvidas" },
  server: { online: "online agora", offline: "servidor fora do ar", checking: "conectando", full: "Servidor cheio", inLine: "na fila" },
  hero: {
    kicker: "Forza Horizon 2 · no PC",
    title: ["De volta", "à estrada."],
    body:
      "O ForzaInfinity traz de volta o lado online do Forza Horizon 2 para a versão de PC no Azimuth. Amigos, clubes de carros, fantasmas dos rivais, placares, pinturas compartilhadas e um save na nuvem que vai do seu PC ao Steam Deck.",
    cta: "Baixar o ForzaInfinity",
    cta2: "Ver o que voltou",
    platforms: "Windows · Steam Deck · Linux · Azimuth 0.9.9",
  },
  toast: {
    label: "Marco do festival",
    lines: {
      races: (n: string) => `${n} corridas concluídas`,
      kilometres: (n: string) => `${n} km rodados juntos`,
      designs: (n: string) => `${n} pinturas criadas`,
      players: (n: string) => `${n} pilotos no servidor`,
      speedTraps: (n: string) => `${n} radares estourados`,
      downloads: (n: string) => `${n} designs baixados`,
    },
  },
  live: {
    title: "Ao vivo do festival",
    note: "Cada número abaixo é do servidor inteiro, atualizado a cada 30 segundos.",
    unavailable: "O servidor está fora do ar agora: estes são os últimos números que vimos.",
  },
  stat: {
    players: "pilotos",
    online: "online agora",
    races: "corridas concluídas",
    kilometres: "km rodados",
    designs: "pinturas criadas",
    sharedFiles: "arquivos compartilhados",
    downloads: "downloads",
    clubs: "clubes de carros",
    friendships: "amizades",
    speedTraps: "radares",
    ghosts: "fantasmas gravados",
    scores: "marcas registradas",
    cars: "carros nas garagens",
    credits: "créditos ganhos",
  },
  features: {
    title: "Tudo o que fazia do Horizon um jogo social",
    rivals: {
      tag: "Rivais",
      title: "Persiga o fantasma dos amigos.",
      body:
        "O melhor tempo de cada amigo em cada rota aparece como tempo a bater. Vá até a corrida e você está na pista contra o fantasma dele, no carro dele.",
      card: { head: "TEMPO A BATER", route: "Nice Aérodrome Cross Country", go: "IR PARA" },
    },
    crew: {
      tag: "Amigos e clubes",
      title: "Sua turma, junta de novo.",
      body:
        "Adicione amigos pelo Nome#12345 direto no jogo, veja o que estão fazendo e monte um clube com tag, cargos e mural.",
      card: { head: "AMIGOS", states: ["No mundo aberto", "Na oficina de pintura", "Offline"], club: "CLUBE" },
    },
    paint: {
      tag: "Storefront",
      title: "Pinte. Compartilhe.",
      body:
        "Compartilhe seus designs, grupos de vinil e ajustes. Seus amigos acham no Recomendados ou na busca, baixam com um botão e curtem.",
      card: { head: "DESIGNS RECOMENDADOS", title: "Sunset Runner", likes: "curtidas", downloads: "downloads" },
    },
    cloud: {
      tag: "Save na nuvem",
      title: "Seu progresso, onde você jogar.",
      body: "Seu perfil nasce do seu save offline e mora no servidor. Continue em outro PC ou no Steam Deck.",
      card: { synced: "Save sincronizado", from: "PC", to: "Steam Deck" },
    },
    more: [
      { title: "Placares", body: "Radares, zonas e combos de habilidade entre amigos." },
      { title: "Drivatars", body: "Os amigos correm contra você e aparecem nos encontros, nas cores deles." },
      { title: "Jogo limpo", body: "Saltos impossíveis no perfil são barrados; denúncias derrubam arquivos." },
      { title: "Privado por padrão", body: "Sem listas públicas: só você, seus amigos e seu clube." },
    ],
  },
  install: {
    title: "Rodando em um minuto",
    note: "O instalador guarda o arquivo original do Azimuth como backup e só mexe na pasta data/bin dele.",
    requires:
      "Requer o Azimuth 0.9.9 (o mais recente) ou o 0.9.7; o 0.9.8 não é suportado (o jogo ficaria offline). O instalador confere a versão para você. Atualizar o Azimuth devolve o arquivo original dele: rode o instalador de novo depois de cada atualização.",
    tabs: { windows: "Windows", deck: "Steam Deck", linux: "Linux" },
    steps: {
      windows: [
        "Baixe o ForzaInfinity.zip e descompacte.",
        "Dê dois cliques em Install.cmd. Ele acha o Azimuth e pergunta o endereço do servidor.",
        "Abra o jogo: Configurações › INFINITY › Create profile.",
      ],
      deck: [
        "Adicione o azimuth.exe à Steam como jogo não-Steam e force o Proton 11.",
        "No modo desktop, abra o Konsole e rode o comando abaixo.",
        "De volta ao modo de jogo: Configurações › INFINITY › Create profile.",
      ],
      linux: [
        "Rode o Azimuth pela Steam com Proton 11 (driver Vulkan com VK_KHR_maintenance5).",
        "Rode o comando abaixo, ou o ./install.sh do zip.",
        "Abra o jogo: Configurações › INFINITY › Create profile.",
      ],
    },
    download: "Baixar .zip",
    copy: "Copiar",
    copied: "Copiado",
    server: (address: string) =>
      address ? `Servidor: ${address}` : "Peça o endereço (host:porta) a quem cuida do seu servidor.",
  },
  faq: {
    title: "Dúvidas",
    items: [
      {
        q: "Como funciona?",
        a: "O Azimuth roda a versão de Xbox One do Forza Horizon 2 no PC com os serviços online desligados. O ForzaInfinity troca o azimuth-tls.dll do Azimuth por um hook que manda as requisições online do próprio jogo para um servidor ForzaInfinity. Os arquivos do jogo nunca são alterados.",
      },
      {
        q: "Do que eu preciso?",
        a: "Sua própria cópia do Forza Horizon 2 no Azimuth 0.9.9 ou 0.9.7 (não no 0.9.8), um servidor para se conectar e um save offline: jogue até o jogo salvar uma vez antes de criar o perfil.",
      },
      {
        q: "Vou perder meu progresso?",
        a: "Não. Seu perfil online começa do seu save offline mais recente, e o save offline continua intacto no seu PC.",
      },
      {
        q: "Posso rodar meu próprio servidor?",
        a: "Pode: é um único programa em Rust com SQLite. Compile com cargo, inicie o run.sh (ou run.bat) e libere a porta TCP 8081 para os jogadores.",
      },
      {
        q: "E a minha privacidade?",
        a: "Não há listas públicas. Você vê os seus dados, os dos seus amigos e os do seu clube. Esta página só mostra totais do servidor. Excluir o perfil apaga tudo o que você tem no servidor.",
      },
      {
        q: "O multiplayer ao vivo voltou?",
        a: "Ainda não. O Online Road Trip e o free roam online precisam substituir os servidores dedicados originais; está planejado para uma versão futura.",
      },
      {
        q: "Como desinstalo?",
        a: "Rode o Uninstall.cmd (Windows) ou o uninstall.sh (Linux / Steam Deck): ele devolve o arquivo original do Azimuth e não mexe nos seus saves.",
      },
    ],
    more: "Documentação completa no GitHub",
  },
  footer:
    "O ForzaInfinity é um projeto de fãs. Não é afiliado, endossado nem patrocinado pela Microsoft, Xbox, Playground Games ou Turn 10 Studios. Forza Horizon e Xbox são marcas de seus donos. Você precisa da sua própria cópia do jogo.",
};

export const TEXTS: Record<Lang, Texts> = { en, pt };

export function initialLang(): Lang {
  try {
    const saved = localStorage.getItem("lang");
    if (saved === "en" || saved === "pt") return saved;
  } catch {
    /* storage blocked: fall back to the browser language */
  }
  return navigator.language.toLowerCase().startsWith("pt") ? "pt" : "en";
}

export function initialPlatform(): Platform {
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("windows")) return "windows";
  if (ua.includes("steamdeck") || ua.includes("valve")) return "deck";
  if (ua.includes("linux")) return "linux";
  return "windows";
}
