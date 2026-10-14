import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  ago,
  clock,
  fmt,
  fmt1,
  hours,
  km,
  kmh,
  label,
  useCountUp,
  type CarClass,
  type Catalog,
  type FeedEvent,
  type Profile,
  type Score,
  type TrafficItem,
} from "./lib";

// ---------------------------------------------------------------- building blocks

export function Card(props: { title: string; icon?: string; extra?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={`card ${props.className ?? ""}`}>
      <header className="card-head">
        <h2>
          {props.icon && <span className="card-icon">{props.icon}</span>}
          {props.title}
        </h2>
        {props.extra}
      </header>
      {props.children}
    </section>
  );
}

/** A number that counts up and flashes when it changes. */
export function Animated({ value, format = fmt }: { value: number; format?: (n: number) => string }) {
  const shown = useCountUp(value);
  const [flash, setFlash] = useState(false);
  const prev = useRef(value);
  useEffect(() => {
    if (prev.current !== value) {
      prev.current = value;
      setFlash(true);
      const t = window.setTimeout(() => setFlash(false), 1200);
      return () => window.clearTimeout(t);
    }
  }, [value]);
  return <span className={flash ? "flash" : undefined}>{format(shown)}</span>;
}

export function Tile(props: { label: string; value: number | null | undefined; format?: (n: number) => string; hint?: string; accent?: boolean }) {
  return (
    <div className={`tile ${props.accent ? "tile-accent" : ""}`}>
      <div className="tile-label">{props.label}</div>
      <div className="tile-value">
        {typeof props.value === "number" ? <Animated value={props.value} format={props.format} /> : "—"}
      </div>
      {props.hint && <div className="tile-hint">{props.hint}</div>}
    </div>
  );
}

export function ClassBadge({ cls, pi }: { cls: CarClass | undefined; pi: number | undefined }) {
  const color = cls?.color ?? "#888";
  return (
    <span className="class-badge" style={{ borderColor: color }}>
      <span className="class-letter" style={{ background: color }}>
        {cls?.name ?? "?"}
      </span>
      <span className="class-pi">{pi ?? "—"}</span>
    </span>
  );
}

// ---------------------------------------------------------------- hero

/** The same picture the game gets (manual file, Xbox avatar or placeholder), initials on error. */
function Avatar({ xuid, initials, version }: { xuid: string; initials: string; version: string }) {
  const [failed, setFailed] = useState(false);
  if (failed) return <div className="avatar">{initials}</div>;
  return (
    <img
      className="avatar avatar-img"
      src={`/api/gamerpic/${xuid}?v=${encodeURIComponent(version)}`}
      alt={initials}
      onError={() => setFailed(true)}
    />
  );
}

export function Hero({ p, now, live }: { p: Profile; now: number; live: boolean }) {
  const race = p.profile?.playerRaceHistory?.[0] ?? {};
  const history = p.profile?.playerHistory?.[0] ?? {};
  const online = now - p.user.lastSeen < 120_000;
  const initials = p.user.gamertag.slice(0, 2).toUpperCase() || "?";
  return (
    <section className="hero">
      <div className="hero-glow" />
      <Avatar xuid={p.user.xuid} initials={initials} version={p.user.gamertag} />
      <div className="hero-main">
        <div className="hero-top">
          <h1>{p.user.gamertag || "Jogador"}</h1>
          <span className={`presence ${online ? "on" : ""}`}>{online ? "no jogo" : `visto ${ago(p.user.lastSeen, now)}`}</span>
        </div>
        <div className="hero-sub">
          XUID {p.user.xuid} · {p.user.logons} login(s) · desde {new Date(p.user.firstSeen).toLocaleDateString("pt-BR")}
          {p.profileUpdated && ` · perfil sincronizado ${ago(p.profileUpdated, now)}`}
          {!live && " · sem conexão ao vivo"}
        </div>
      </div>
      <div className="hero-stats">
        <div>
          <div className="hero-label">Nível</div>
          {/* the profile stores the level from 0; the game shows level + 1 */}
          <div className="hero-value">{typeof race.level === "number" ? <Animated value={race.level + 1} /> : "—"}</div>
        </div>
        <div>
          <div className="hero-label">XP</div>
          <div className="hero-value">{typeof race.xp === "number" ? <Animated value={race.xp} /> : "—"}</div>
        </div>
        <div>
          <div className="hero-label">Créditos</div>
          <div className="hero-value credits">
            {typeof history.credits === "number" ? <Animated value={history.credits} /> : "—"}
            <small> CR</small>
          </div>
        </div>
      </div>
    </section>
  );
}

// ---------------------------------------------------------------- numbers

export function Numbers({ p }: { p: Profile }) {
  const r = p.profile?.playerRaceHistory?.[0] ?? {};
  const c = p.profile?.compareStats?.[0] ?? {};
  const n = (v: unknown) => (typeof v === "number" ? v : null);
  return (
    <div className="tiles">
      <Tile label="Corridas" value={n(r.numRaces)} accent />
      <Tile label="Vitórias" value={n(r.numVictories)} hint={pct(r.numVictories, r.numRaces)} />
      <Tile label="Pódios" value={n(r.numPodiums)} hint={pct(r.numPodiums, r.numRaces)} />
      <Tile label="Prêmios" value={n(r.totalWinnings)} hint="CR" />
      <Tile label="Distância" value={n(r.distanceDriven)} format={(v) => km(v)} />
      <Tile label="Tempo ao volante" value={n(r.timeDriven)} format={(v) => hours(v)} />
      <Tile label="Velocidade máxima" value={n(r.topSpeed)} format={(v) => kmh(v)} />
      <Tile label="Maior salto" value={n(r.longestJump)} format={(v) => `${fmt1(v)} m`} />
      <Tile label="Carros na garagem" value={n(c.carsInGarage) ?? p.garage.length} />
      <Tile label="Valor da garagem" value={n(c.garageValue)} hint="CR" />
      <Tile label="Estradas descobertas" value={n(c.roadsDiscovered)} />
      <Tile label="Skill tokens" value={n(r.numSkillTokens)} />
    </div>
  );
}

function pct(a: unknown, b: unknown) {
  return typeof a === "number" && typeof b === "number" && b > 0 ? `${Math.round((a / b) * 100)}%` : undefined;
}

// ---------------------------------------------------------------- garage

export function Garage({ p, catalog }: { p: Profile; catalog: Catalog | null }) {
  const cars = [...p.garage].sort((a, b) => (b.numRaces ?? 0) - (a.numRaces ?? 0));
  if (!cars.length)
    return <Empty text="A garagem aparece quando o jogo sincronizar o perfil (OnlineProfile.UpdateAllTables)." />;
  return (
    <div className="garage">
      {cars.map((g) => {
        const cls = catalog?.classes[String(g.classID ?? g.car?.classId)];
        const pi = Math.round((g.perfIndex ?? 0) * 1000) || g.car?.pi;
        return (
          <article className="car" key={g.id} style={{ ["--class" as string]: cls?.color ?? "#888" }}>
            <div className="car-stripe" />
            <div className="car-head">
              <div>
                <div className="car-make">{g.car ? `${g.car.make} · ${g.car.year}` : "Carro"}</div>
                <div className="car-name">{g.car?.model ?? `#${g.carId}`}</div>
              </div>
              <ClassBadge cls={cls} pi={pi} />
            </div>
            <div className="car-ratings">
              <Rating name="Velocidade" v={g.speedRating} />
              <Rating name="Aceleração" v={g.accelerationRating} />
              <Rating name="Largada" v={g.launchRating} />
              <Rating name="Frenagem" v={g.brakingRating} />
              <Rating name="Controle" v={g.handlingRating} />
            </div>
            <div className="car-stats">
              <span><b>{fmt(g.numRaces)}</b> corridas</span>
              <span><b>{fmt(g.numVictories)}</b> vitórias</span>
              <span><b>{km(g.distanceDriven)}</b></span>
              <span><b>{fmt(g.totalWinnings)}</b> CR</span>
            </div>
            {g.originalOwner && <div className="car-owner">Dono original: {g.originalOwner}</div>}
          </article>
        );
      })}
    </div>
  );
}

function Rating({ name, v }: { name: string; v: number }) {
  const w = Math.max(0, Math.min(10, v ?? 0)) * 10;
  return (
    <div className="rating">
      <span>{name}</span>
      <div className="bar">
        <div style={{ width: `${w}%` }} />
      </div>
      <b>{fmt1(v)}</b>
    </div>
  );
}

// ---------------------------------------------------------------- scores + xp

export function Scores({ p, now, catalog }: { p: Profile; now: number; catalog: Catalog | null }) {
  if (!p.scores.length) return <Empty text="Nenhuma pontuação enviada ainda (Scoreboard.SubmitScore)." />;
  return (
    <table className="table">
      <thead>
        <tr>
          <th>Quando</th>
          <th>Carro</th>
          <th>Pista / evento</th>
          <th className="num">Pontuação</th>
        </tr>
      </thead>
      <tbody>
        {p.scores.slice(0, 25).map((s) => (
          <tr key={s.scoreId}>
            <td className="muted">{ago(s.time, now)}</td>
            <td>
              <ClassBadge cls={catalog?.classes[String(s.carClassId)]} pi={s.car?.pi} /> {s.car?.name ?? `#${s.carId}`}
            </td>
            <td>
              {s.track?.name ?? (s.trackId >= 0 ? `Pista #${s.trackId}` : "Mundo aberto")}
              <span className="muted"> · quadro {s.scoreboardType}, tipo {s.scoreType}</span>
            </td>
            <td className="num strong">
              {formatScore(s)} {s.isClean && <span className="clean" title="Corrida limpa">✓</span>}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function formatScore(s: Score) {
  if (s.scoreType === 1) {
    const ms = Math.round(s.score * 1000);
    return `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}.${String(ms % 1000).padStart(3, "0")}`;
  }
  return fmt(s.score);
}

export function XpChart({ p }: { p: Profile }) {
  const pts = p.xp;
  if (pts.length === 0) return <Empty text="Cada corrida envia o XP ganho (Scoreboard.SubmitXpScore)." />;
  if (pts.length === 1)
    return (
      <div className="xp-summary">
        <div><span className="muted">Primeira corrida</span> <b>+{fmt(pts[0].xp)}</b></div>
        <div className="muted">O gráfico aparece a partir da segunda corrida.</div>
      </div>
    );
  const W = 600, H = 160, pad = 8;
  const max = Math.max(...pts.map((x) => x.xp), 1);
  const step = pts.length > 1 ? (W - pad * 2) / (pts.length - 1) : 0;
  const xy = pts.map((x, i) => [pad + i * step, H - pad - (x.xp / max) * (H - pad * 2)] as const);
  const line = xy.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
  const area = `${line} L${xy[xy.length - 1][0].toFixed(1)},${H - pad} L${pad},${H - pad} Z`;
  const total = pts.reduce((a, x) => a + x.xp, 0);
  return (
    <div>
      <div className="xp-summary">
        <div><span className="muted">Última corrida</span> <b>+{fmt(pts[pts.length - 1].xp)}</b></div>
        <div><span className="muted">Soma ({pts.length})</span> <b>{fmt(total)}</b></div>
        <div><span className="muted">Melhor</span> <b>{fmt(max)}</b></div>
      </div>
      <svg className="chart" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
        <defs>
          <linearGradient id="xpfill" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="var(--pink)" stopOpacity="0.45" />
            <stop offset="1" stopColor="var(--pink)" stopOpacity="0" />
          </linearGradient>
        </defs>
        <path d={area} fill="url(#xpfill)" />
        <path d={line} fill="none" stroke="url(#stroke)" strokeWidth="2.5" vectorEffect="non-scaling-stroke" />
        <linearGradient id="stroke" x1="0" x2="1">
          <stop offset="0" stopColor="var(--pink)" />
          <stop offset="1" stopColor="var(--orange)" />
        </linearGradient>
        {xy.map(([x, y], i) => (
          <circle key={i} cx={x} cy={y} r="3" fill="var(--orange)" vectorEffect="non-scaling-stroke" />
        ))}
      </svg>
    </div>
  );
}

// ---------------------------------------------------------------- feed

const ICONS: Record<string, string> = {
  login: "🔑",
  profile: "👤",
  garage: "🚗",
  score: "🏁",
  xp: "⚡",
  tune: "🔧",
  gameplay: "📝",
  achievement: "🏆",
  transaction: "🔁",
  livery: "🎨",
  drivatar: "🤖",
  rival: "⏱️",
  ghost: "👻",
};

export function Feed({ events, now }: { events: FeedEvent[]; now: number }) {
  if (!events.length) return <Empty text="Os eventos aparecem aqui em tempo real enquanto você joga." />;
  return (
    <ol className="feed">
      {events.slice(0, 60).map((e) => (
        <li key={e.id} className={`feed-item kind-${e.kind}`}>
          <span className="feed-icon">{ICONS[e.kind] ?? "•"}</span>
          <div>
            <div className="feed-title">{e.title}</div>
            <div className="feed-time" title={new Date(e.time).toLocaleString("pt-BR")}>
              {ago(e.time, now)}
            </div>
          </div>
        </li>
      ))}
    </ol>
  );
}

export function Traffic({ items }: { items: TrafficItem[] }) {
  if (!items.length) return <Empty text="Nenhuma requisição desde que a página abriu." small />;
  return (
    <ol className="traffic">
      {items.slice(0, 14).map((t) => (
        <li key={t.seq}>
          <span className="muted">{clock(t.time)}</span>
          <span className="traffic-label">{t.label}</span>
          <span className={`status s${String(t.status)[0]}`}>{t.status}</span>
        </li>
      ))}
    </ol>
  );
}

// ---------------------------------------------------------------- achievements + raw stats

export function Achievements({ p }: { p: Profile }) {
  if (!p.achievements.length)
    return (
      <Empty text="As conquistas do Xbox são desbloqueadas localmente pelo Azimuth e ainda não chegam ao servidor. Próximo passo: o hook repassar os desbloqueios." />
    );
  return (
    <ul className="achievements">
      {p.achievements.map((a) => (
        <li key={a.id} className={a.unlocked ? "unlocked" : ""}>🏆 {a.name}</li>
      ))}
    </ul>
  );
}

/** Units of the profile stats: distances in meters, speeds in m/s, times in seconds. */
function statValue(key: string, v: number, catalog: Catalog | null): string {
  const k = key.toLowerCase();
  if (k === "level") return fmt(v + 1); // stored from 0, shown like the game
  if (k.endsWith("car") && catalog?.cars[String(v)]) return catalog.cars[String(v)].name;
  if (k.includes("distance")) return km(v);
  if (k.includes("speed") && !k.startsWith("num") && !k.includes("speeds")) return kmh(v);
  if (k.startsWith("time") || k.endsWith("timedriven")) return hours(v);
  if (k.includes("jump") || k === "longestdrift") return `${fmt1(v)} m`;
  return Number.isInteger(v) ? fmt(v) : fmt1(v);
}

export function StatsTable({ stats, catalog }: { stats: Record<string, unknown> | undefined; catalog: Catalog | null }) {
  if (!stats) return <Empty text="Sem dados ainda." small />;
  const entries = Object.entries(stats).filter(([, v]) => typeof v === "number" || typeof v === "boolean");
  return (
    <dl className="kv">
      {entries.map(([k, v]) => (
        <div key={k}>
          <dt>{label(k)}</dt>
          <dd>{typeof v === "number" ? statValue(k, v, catalog) : v ? "sim" : "não"}</dd>
        </div>
      ))}
    </dl>
  );
}

export function Empty({ text, small }: { text: string; small?: boolean }) {
  return <p className={`empty ${small ? "small" : ""}`}>{text}</p>;
}
