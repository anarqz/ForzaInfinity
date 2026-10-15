import { useEffect, useRef, useState, type ReactNode } from "react";
import { DOWNLOAD_ZIP, INSTALL_PS1, INSTALL_SH, REPO_URL, SERVER_ADDRESS } from "./config";
import { initialLang, initialPlatform, TEXTS, type Lang, type Platform, type Texts } from "./i18n";
import { formatNumber, useStats, type Stats } from "./stats";

export function App() {
  const [lang, setLang] = useState<Lang>(initialLang);
  const t = TEXTS[lang];
  const { stats, reachable } = useStats();
  const scrolled = useScrolled();
  useReveal();

  const changeLang = (next: Lang) => {
    setLang(next);
    try {
      localStorage.setItem("lang", next);
    } catch {
      /* storage blocked: the choice lasts this visit */
    }
    document.documentElement.lang = next === "pt" ? "pt-BR" : "en";
  };

  return (
    <>
      <header className={scrolled ? "bar solid" : "bar"}>
        <a className="brand" href="#top">
          <BrandMark />
          <span>
            FORZA<b>INFINITY</b>
          </span>
        </a>
        <nav className="links" aria-label="Sections">
          <a href="#features">{t.nav.features}</a>
          <a href="#live">{t.nav.stats}</a>
          <a href="#install">{t.nav.install}</a>
          <a href="#faq">{t.nav.faq}</a>
        </nav>
        <div className="bar-end">
          <div className="lang" role="group" aria-label="Language">
            {(["en", "pt"] as const).map((l) => (
              <button key={l} className={l === lang ? "on" : ""} onClick={() => changeLang(l)} aria-pressed={l === lang}>
                {l.toUpperCase()}
              </button>
            ))}
          </div>
          <a className="pill-button small" href="#install">
            {t.nav.install}
          </a>
        </div>
      </header>

      <main id="top">
        <Hero t={t} stats={stats} reachable={reachable} />
        <Features t={t} />
        <Live t={t} stats={stats} reachable={reachable} />
        <Install t={t} />
        <Faq t={t} />
        <section className="finale" data-reveal>
          <h2>{lang === "pt" ? "Te vejo no festival." : "See you at the festival."}</h2>
          <a className="pill-button" href="#install">
            {t.hero.cta}
          </a>
        </section>
      </main>

      <footer className="footer">
        <p>{t.footer}</p>
        <a href={REPO_URL}>GitHub</a>
      </footer>
    </>
  );
}

// ---------------------------------------------------------------- hooks

function useScrolled(): boolean {
  const [scrolled, setScrolled] = useState(false);
  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 40);
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => window.removeEventListener("scroll", onScroll);
  }, []);
  return scrolled;
}

/** Elements with data-reveal fade and rise in the first time they scroll into view. The mark is
 *  an attribute React does not own (a class would be reset on the next render); elements made
 *  after a render (a language switch remounts some) are picked up on that render. */
function useReveal() {
  useEffect(() => {
    const observer = new IntersectionObserver(
      (entries) =>
        entries.forEach((e) => {
          if (e.isIntersecting) {
            e.target.setAttribute("data-in", "");
            observer.unobserve(e.target);
          }
        }),
      { threshold: 0.12 },
    );
    document.querySelectorAll("[data-reveal]:not([data-in])").forEach((el) => observer.observe(el));
    return () => observer.disconnect();
  });
}

/** Counts up to `target` (from the last value shown). */
function useCountUp(target?: number): number {
  const [value, setValue] = useState(0);
  const from = useRef(0);
  useEffect(() => {
    if (target === undefined) return;
    const start = performance.now();
    const begin = from.current;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let frame = 0;
    const step = (now: number) => {
      const k = reduce ? 1 : Math.min(1, (now - start) / 1200);
      setValue(Math.round(begin + (target - begin) * (1 - Math.pow(1 - k, 3))));
      if (k < 1) frame = requestAnimationFrame(step);
      else from.current = target;
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  }, [target]);
  return value;
}

// ----------------------------------------------------------------- hero

function Hero({ t, stats, reachable }: { t: Texts; stats: Stats | null; reachable: boolean | null }) {
  return (
    <section className="hero">
      <RoadScene />
      <div className="hero-content">
        <LivePill t={t} stats={stats} reachable={reachable} />
        <p className="kicker">{t.hero.kicker}</p>
        <h1>
          {t.hero.title[0]}
          <br />
          <span className="glow">{t.hero.title[1]}</span>
        </h1>
        <p className="lead">{t.hero.body}</p>
        <div className="hero-actions">
          <a className="pill-button" href="#install">
            <IconDownload /> {t.hero.cta}
          </a>
          <a className="pill-button ghost" href="#features">
            {t.hero.cta2}
          </a>
        </div>
        <p className="platforms">{t.hero.platforms}</p>
      </div>
      <Toasts t={t} stats={stats} />
      <a className="scroll-cue" href="#features" aria-label={t.hero.cta2}>
        <span />
      </a>
    </section>
  );
}

function LivePill({ t, stats, reachable }: { t: Texts; stats: Stats | null; reachable: boolean | null }) {
  const full = reachable && stats?.full;
  const label =
    reachable === null
      ? t.server.checking
      : !reachable
        ? t.server.offline
        : full
          ? `${t.server.full} · ${formatNumber(stats?.queued ?? 0, t.locale)} ${t.server.inLine}`
          : `${occupancy(stats, t.locale)} ${t.server.online}`;
  return (
    <span className={`live-pill ${full ? "full" : reachable ? "up" : reachable === false ? "down" : ""}`}>
      <i aria-hidden="true" />
      {label}
    </span>
  );
}

/** "87" or, on a server with a player limit, "87/150". */
function occupancy(stats: Stats | null, locale: string): string {
  const online = formatNumber(stats?.online ?? 0, locale);
  return stats?.capacity ? `${online}/${formatNumber(stats.capacity, locale)}` : online;
}

/** "Festival milestone" toasts with the server's real totals, one after another. */
function Toasts({ t, stats }: { t: Texts; stats: Stats | null }) {
  const keys = ["races", "kilometres", "designs", "players", "speedTraps", "downloads"] as const;
  const shown = stats ? keys.filter((k) => stats[k] > 0) : [];
  const [index, setIndex] = useState(0);
  useEffect(() => {
    if (shown.length < 2) return;
    const timer = window.setInterval(() => setIndex((i) => (i + 1) % shown.length), 4200);
    return () => window.clearInterval(timer);
  }, [shown.length]);
  if (!stats || shown.length === 0) return null;
  const key = shown[index % shown.length];
  return (
    <div className="toast-zone" aria-live="polite">
      <div className="toast" key={`${key}-${index}`}>
        <span className="toast-badge">
          <IconTrophy />
        </span>
        <span className="toast-text">
          <small>{t.toast.label}</small>
          <strong>{t.toast.lines[key](formatNumber(stats[key], t.locale))}</strong>
        </span>
      </div>
    </div>
  );
}

/** A dusk sky, hills and a road racing to the horizon (CSS animates its lane marks). */
function RoadScene() {
  return (
    <div className="scene" aria-hidden="true">
      <div className="sky" />
      <div className="sun" />
      <svg className="hills" viewBox="0 0 1440 320" preserveAspectRatio="none">
        <path d="M0 230 C180 150 320 200 470 170 C640 135 760 210 930 175 C1100 140 1260 190 1440 150 V320 H0Z" fill="#14301a" />
        <path d="M0 270 C220 220 380 260 560 240 C760 215 900 270 1080 245 C1240 225 1340 250 1440 235 V320 H0Z" fill="#0b1d0f" />
      </svg>
      <div className="ground">
        <div className="road" />
      </div>
      <div className="vignette" />
    </div>
  );
}

// ------------------------------------------------------------- features

function Features({ t }: { t: Texts }) {
  const f = t.features;
  return (
    <section id="features" className="section">
      <h2 className="section-title" data-reveal>
        {f.title}
      </h2>
      <Feature tag={f.rivals.tag} title={f.rivals.title} body={f.rivals.body} visual={<RivalCard t={t} />} />
      <Feature tag={f.crew.tag} title={f.crew.title} body={f.crew.body} visual={<FriendsCard t={t} />} flip />
      <Feature tag={f.paint.tag} title={f.paint.title} body={f.paint.body} visual={<DesignCard t={t} />} />
      <Feature tag={f.cloud.tag} title={f.cloud.title} body={f.cloud.body} visual={<CloudCard t={t} />} flip />
      <div className="more-grid">
        {f.more.map((m, i) => (
          <div key={m.title} className="glass small-card" data-reveal style={{ transitionDelay: `${i * 70}ms` }}>
            <span className="small-icon">{[<IconTrophy />, <IconCar />, <IconCheck />, <IconLock />][i]}</span>
            <h3>{m.title}</h3>
            <p>{m.body}</p>
          </div>
        ))}
      </div>
    </section>
  );
}

function Feature(props: { tag: string; title: string; body: string; visual: ReactNode; flip?: boolean }) {
  return (
    <div className={props.flip ? "feature flip" : "feature"}>
      <div className="feature-text" data-reveal>
        <span className="tag">{props.tag}</span>
        <h3>{props.title}</h3>
        <p>{props.body}</p>
      </div>
      <div className="feature-visual" data-reveal>
        {props.visual}
      </div>
    </div>
  );
}

function RivalCard({ t }: { t: Texts }) {
  const c = t.features.rivals.card;
  return (
    <div className="mock rival">
      <div className="mock-head">
        <span>{c.head}</span>
        <span className="mock-dot" />
      </div>
      <div className="rival-route">{c.route}</div>
      <div className="rival-row">
        <Avatar letter="K" hue={150} />
        <div>
          <strong>Kira#20417</strong>
          <small>Toyota Supra RZ 1998</small>
        </div>
        <span className="rival-time">2:39.20</span>
      </div>
      <div className="ghost-track">
        <span className="ghost-car" />
        <span className="you-car" />
      </div>
      <button className="mock-button" tabIndex={-1}>
        {c.go}
      </button>
    </div>
  );
}

function FriendsCard({ t }: { t: Texts }) {
  const c = t.features.crew.card;
  const friends = [
    { name: "Kira#20417", state: c.states[0], on: true, hue: 150 },
    { name: "Duke#88301", state: c.states[1], on: true, hue: 30 },
    { name: "Nova#41022", state: c.states[2], on: false, hue: 270 },
  ];
  return (
    <div className="mock friends">
      <div className="mock-head">
        <span>{c.head}</span>
        <span className="club-tag">
          {c.club} · <b>APEX</b>
        </span>
      </div>
      {friends.map((f) => (
        <div key={f.name} className="friend-row">
          <Avatar letter={f.name[0]} hue={f.hue} />
          <div>
            <strong>{f.name}</strong>
            <small className={f.on ? "on" : ""}>{f.state}</small>
          </div>
          <span className={f.on ? "presence on" : "presence"} />
        </div>
      ))}
    </div>
  );
}

function DesignCard({ t }: { t: Texts }) {
  const c = t.features.paint.card;
  return (
    <div className="mock design">
      <div className="mock-head">
        <span>{c.head}</span>
      </div>
      <div className="livery">
        <CarSilhouette />
      </div>
      <div className="design-meta">
        <div>
          <strong>{c.title}</strong>
          <small>Duke#88301 · RACING · RETRO</small>
        </div>
        <div className="design-stats">
          <span>
            <IconHeart /> 128 <small>{c.likes}</small>
          </span>
          <span>
            <IconDownload /> 342 <small>{c.downloads}</small>
          </span>
        </div>
      </div>
    </div>
  );
}

function CloudCard({ t }: { t: Texts }) {
  const c = t.features.cloud.card;
  return (
    <div className="mock cloud">
      <div className="device">
        <IconMonitor />
        <span>{c.from}</span>
      </div>
      <div className="sync-line">
        <span className="pulse" />
        <span className="sync-label">
          <IconCloud /> {c.synced}
        </span>
      </div>
      <div className="device">
        <IconHandheld />
        <span>{c.to}</span>
      </div>
    </div>
  );
}

function Avatar({ letter, hue }: { letter: string; hue: number }) {
  return (
    <span className="avatar" style={{ background: `linear-gradient(135deg, hsl(${hue} 70% 45%), hsl(${hue + 40} 70% 30%))` }}>
      {letter}
    </span>
  );
}

// ----------------------------------------------------------------- live

type StatKey = keyof Texts["stat"];
const BENTO: { key: StatKey; size?: "wide" | "tall" | "big" }[] = [
  { key: "players", size: "big" },
  { key: "races", size: "wide" },
  { key: "kilometres" },
  { key: "designs" },
  { key: "credits", size: "wide" },
  { key: "sharedFiles" },
  { key: "downloads" },
  { key: "clubs" },
  { key: "friendships" },
  { key: "speedTraps" },
  { key: "ghosts" },
  { key: "scores", size: "wide" },
  { key: "cars", size: "wide" },
];

function Live({ t, stats, reachable }: { t: Texts; stats: Stats | null; reachable: boolean | null }) {
  return (
    <section id="live" className="section">
      <h2 className="section-title" data-reveal>
        {t.live.title}
      </h2>
      <p className="section-note" data-reveal>
        {reachable === false ? t.live.unavailable : t.live.note}
      </p>
      <div className="bento">
        {BENTO.map(({ key, size }, i) => (
          <StatCard key={key} label={t.stat[key]} value={stats?.[key]} locale={t.locale} size={size} delay={i * 40}>
            {key === "players" && stats ? (
              <>
                <span className={stats.full ? "online-line full" : "online-line"}>
                  <i /> {occupancy(stats, t.locale)} {t.stat.online}
                  {stats.queued ? ` · ${formatNumber(stats.queued, t.locale)} ${t.server.inLine}` : ""}
                </span>
                {stats.capacity ? (
                  <span className="slots" aria-hidden="true">
                    <span style={{ width: `${Math.min(100, (stats.online / stats.capacity) * 100)}%` }} />
                  </span>
                ) : null}
              </>
            ) : null}
          </StatCard>
        ))}
      </div>
    </section>
  );
}

function StatCard(props: { label: string; value?: number; locale: string; size?: string; delay: number; children?: ReactNode }) {
  const shown = useCountUp(props.value);
  return (
    <div className={`glass stat ${props.size ?? ""}`} data-reveal style={{ transitionDelay: `${props.delay}ms` }}>
      <span className="stat-value">{props.value === undefined ? "—" : formatNumber(shown, props.locale)}</span>
      <span className="stat-label">{props.label}</span>
      {props.children}
    </div>
  );
}

// -------------------------------------------------------------- install

function Install({ t }: { t: Texts }) {
  const [platform, setPlatform] = useState<Platform>(initialPlatform);
  const command = platform === "windows" ? INSTALL_PS1 : INSTALL_SH;
  return (
    <section id="install" className="section">
      <h2 className="section-title" data-reveal>
        {t.install.title}
      </h2>
      <p className="section-note" data-reveal>
        {t.install.note}
      </p>
      <p className="requires" data-reveal>
        <IconLock /> {t.install.requires}
      </p>
      <div className="install glass" data-reveal>
        <div className="tabs" role="tablist">
          {(["windows", "deck", "linux"] as const).map((p) => (
            <button key={p} role="tab" aria-selected={p === platform} className={p === platform ? "on" : ""} onClick={() => setPlatform(p)}>
              {p === "windows" ? <IconMonitor /> : p === "deck" ? <IconHandheld /> : <IconTerminal />}
              {t.install.tabs[p]}
            </button>
          ))}
        </div>
        <ol className="steps">
          {t.install.steps[platform].map((s, i) => (
            <li key={`${platform}-${i}`}>
              <span className="step-n">{i + 1}</span>
              <span>{s}</span>
            </li>
          ))}
        </ol>
        <div className="install-actions">
          <a className="pill-button" href={DOWNLOAD_ZIP}>
            <IconDownload /> {t.install.download}
          </a>
          <CopyCommand command={command} t={t} />
        </div>
        <p className="server-hint">{t.install.server(SERVER_ADDRESS)}</p>
      </div>
    </section>
  );
}

function CopyCommand({ command, t }: { command: string; t: Texts }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      /* no clipboard access: the command stays selectable */
    }
  };
  return (
    <div className="command">
      <code>{command}</code>
      <button onClick={copy}>{copied ? t.install.copied : t.install.copy}</button>
    </div>
  );
}

// ------------------------------------------------------------------ faq

function Faq({ t }: { t: Texts }) {
  return (
    <section id="faq" className="section narrow">
      <h2 className="section-title" data-reveal>
        {t.faq.title}
      </h2>
      <div className="faq">
        {t.faq.items.map((item) => (
          <details key={item.q} className="glass qa" data-reveal>
            <summary>{item.q}</summary>
            <p>{item.a}</p>
          </details>
        ))}
      </div>
      <a className="text-link" href={`${REPO_URL}#readme`}>
        {t.faq.more} →
      </a>
    </section>
  );
}

// ---------------------------------------------------------- art, icons

function BrandMark() {
  return (
    <svg className="brand-mark" viewBox="0 0 32 32" aria-hidden="true">
      <defs>
        <linearGradient id="bm" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#5ce65c" />
          <stop offset="1" stopColor="#107c10" />
        </linearGradient>
      </defs>
      <circle cx="16" cy="16" r="15" fill="url(#bm)" />
      <path d="M9.5 16c2.2-4.4 4.4-4.4 6.5 0s4.3 4.4 6.5 0" stroke="#fff" strokeWidth="2.6" fill="none" strokeLinecap="round" />
    </svg>
  );
}

function CarSilhouette() {
  return (
    <svg viewBox="0 0 320 130" className="car-art" aria-hidden="true">
      <defs>
        <linearGradient id="paint" x1="0" y1="0" x2="1" y2="0">
          <stop offset="0" stopColor="#ff4f7b" />
          <stop offset=".5" stopColor="#ffb547" />
          <stop offset="1" stopColor="#3ddc84" />
        </linearGradient>
      </defs>
      <path
        d="M18 92 C22 74 44 66 74 62 L118 38 C134 30 176 28 204 34 L252 58 C280 62 300 70 304 86 L306 98 H18 Z"
        fill="url(#paint)"
      />
      <path d="M126 44 L120 60 H196 L186 40 C168 36 144 36 126 44Z" fill="#0d1a12" opacity=".75" />
      <path d="M200 42 L210 60 H240 L214 46Z" fill="#0d1a12" opacity=".75" />
      <path d="M30 84 H296" stroke="#ffffff" strokeOpacity=".5" strokeWidth="3" strokeDasharray="22 10" />
      <circle cx="82" cy="98" r="18" fill="#111" stroke="#2a2a2a" strokeWidth="6" />
      <circle cx="244" cy="98" r="18" fill="#111" stroke="#2a2a2a" strokeWidth="6" />
    </svg>
  );
}

const icon = (path: ReactNode) => (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    {path}
  </svg>
);
function IconDownload() {
  return icon(<><path d="M12 4v11" /><path d="M7 11l5 5 5-5" /><path d="M5 20h14" /></>);
}
function IconTrophy() {
  return icon(<><path d="M8 4h8v5a4 4 0 0 1-8 0V4Z" /><path d="M8 6H5a3 3 0 0 0 3 3M16 6h3a3 3 0 0 1-3 3" /><path d="M12 13v4M9 20h6" /></>);
}
function IconCar() {
  return icon(<><path d="M4 15l1.5-5A2 2 0 0 1 7.4 8.6h9.2a2 2 0 0 1 1.9 1.4L20 15" /><rect x="3" y="15" width="18" height="4" rx="1" /><circle cx="7.5" cy="19" r="1.4" /><circle cx="16.5" cy="19" r="1.4" /></>);
}
function IconCheck() {
  return icon(<><circle cx="12" cy="12" r="8" /><path d="M8.5 12.5l2.5 2.5 4.5-5" /></>);
}
function IconLock() {
  return icon(<><rect x="5" y="11" width="14" height="9" rx="2" /><path d="M8 11V8a4 4 0 0 1 8 0v3" /></>);
}
function IconHeart() {
  return icon(<path d="M12 20s-7-4.4-7-10a4 4 0 0 1 7-2.6A4 4 0 0 1 19 10c0 5.6-7 10-7 10Z" />);
}
function IconCloud() {
  return icon(<path d="M7 18h10a4 4 0 0 0 .4-8A6 6 0 0 0 6 9.5 4.3 4.3 0 0 0 7 18Z" />);
}
function IconMonitor() {
  return icon(<><rect x="3" y="4" width="18" height="12" rx="2" /><path d="M8 20h8M12 16v4" /></>);
}
function IconHandheld() {
  return icon(<><rect x="2" y="7" width="20" height="10" rx="4" /><rect x="8" y="9" width="8" height="6" rx="1" /><circle cx="5.5" cy="12" r="1" /><circle cx="18.5" cy="12" r="1" /></>);
}
function IconTerminal() {
  return icon(<><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M7 9l3 3-3 3M12 15h5" /></>);
}
