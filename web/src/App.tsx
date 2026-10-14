import { useCallback, useEffect, useRef, useState } from "react";
import {
  Achievements,
  Card,
  Empty,
  Feed,
  Garage,
  Hero,
  Numbers,
  Scores,
  StatsTable,
  Traffic,
  XpChart,
} from "./components";
import { Friends, Players } from "./social";
import { getJson, useLive, useNow, type Catalog, type FeedEvent, type Profile, type TrafficItem, type UserSummary } from "./lib";

const STAT_TABS = [
  ["playerRaceHistory", "Corridas"],
  ["compareStats", "Mundo aberto"],
  ["miscStats", "Diversos"],
  ["drivatarStats", "Drivatar"],
  ["assists", "Assistências"],
] as const;

function formatTime(seconds: number) {
  const ms = Math.round(seconds * 1000);
  return `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}.${String(ms % 1000).padStart(3, "0")}`;
}

export default function App() {
  const [users, setUsers] = useState<UserSummary[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [profile, setProfile] = useState<Profile | null>(null);
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [traffic, setTraffic] = useState<TrafficItem[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<(typeof STAT_TABS)[number][0]>("playerRaceHistory");
  const now = useNow(5000);
  const selectedRef = useRef(selected);
  selectedRef.current = selected;

  const loadUsers = useCallback(async () => {
    try {
      const list = await getJson<UserSummary[]>("/api/users");
      setUsers(list);
      setSelected((s) => s ?? list[0]?.xuid ?? null);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const loadProfile = useCallback(async (xuid: string) => {
    try {
      setProfile(await getJson<Profile>(`/api/users/${xuid}`));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  // Many events arrive in bursts (a race end = profile + garage + score + xp): refetch once.
  const refetch = useRef<number | undefined>(undefined);
  const scheduleRefetch = useCallback(() => {
    window.clearTimeout(refetch.current);
    refetch.current = window.setTimeout(() => {
      if (selectedRef.current) loadProfile(selectedRef.current);
      loadUsers();
    }, 350);
  }, [loadProfile, loadUsers]);

  const live = useLive({
    onHello: (list) => {
      setUsers(list);
      setSelected((s) => s ?? list[0]?.xuid ?? null);
    },
    onEvent: (e: FeedEvent) => {
      if (selectedRef.current && e.xuid !== selectedRef.current && e.xuid !== "0") return;
      // show it at once; the refetch brings the numbers
      setProfile((p) => (p && !p.events.some((x) => x.id === e.id) ? { ...p, events: [e, ...p.events] } : p));
      if (!selectedRef.current) loadUsers();
      scheduleRefetch();
    },
    onRequest: (r) => setTraffic((t) => [r, ...t].slice(0, 50)),
    onUsers: (list) => {
      setUsers(list);
      setSelected((s) => (s && list.some((u) => u.xuid === s) ? s : list[0]?.xuid ?? null));
      scheduleRefetch();
    },
  });

  useEffect(() => {
    loadUsers();
    getJson<Catalog>("/api/catalog").then(setCatalog).catch(() => setCatalog(null));
  }, [loadUsers]);

  useEffect(() => {
    if (selected) loadProfile(selected);
  }, [selected, loadProfile]);

  return (
    <div className="app">
      <nav className="topbar">
        <div className="brand">
          <span className="logo">FH2</span>
          <span>
            <b>Online</b> Manager
          </span>
        </div>
        <div className="topbar-right">
          {users.length > 1 && (
            <select value={selected ?? ""} onChange={(e) => setSelected(e.target.value)}>
              {users.map((u) => (
                <option key={u.xuid} value={u.xuid}>
                  {u.gamertag || u.xuid}
                </option>
              ))}
            </select>
          )}
          <span className={`live live-${live}`}>
            <span className="dot" />
            {live === "live" ? "ao vivo" : live === "connecting" ? "conectando" : "offline"}
          </span>
        </div>
      </nav>

      {error && (
        <div className="error" onClick={() => setError(null)}>
          {error}
        </div>
      )}

      {!profile ? (
        <main className="welcome">
          <h1>Nenhum jogador ainda</h1>
          <p>
            Abra o jogo com o hook em <code>Mode=Proxy</code> apontando para este servidor. O perfil aparece aqui assim que o{" "}
            <code>User.Logon</code> passar.
          </p>
        </main>
      ) : (
        <main className="grid">
          <div className="span-all">
            <Hero p={profile} now={now} live={live === "live"} />
          </div>

          <div className="span-all">
            <Numbers p={profile} />
          </div>

          <div className="col-main">
            <Card title="Garagem" icon="🚗" extra={<span className="pill">{profile.garage.length} carro(s)</span>}>
              <Garage p={profile} catalog={catalog} />
            </Card>

            <Card title="XP por corrida" icon="⚡">
              <XpChart p={profile} />
            </Card>

            <Card title="Pontuações" icon="🏁" extra={<span className="pill">{profile.scores.length}</span>}>
              <Scores p={profile} now={now} catalog={catalog} />
            </Card>

            <Card title="Fantasmas" icon="👻" extra={<span className="pill">{profile.ghosts.length}</span>}>
              {profile.ghosts.length ? (
                <table className="table">
                  <thead>
                    <tr>
                      <th>Evento</th>
                      <th>Carro</th>
                      <th className="num">Tempo</th>
                      <th className="num">Tamanho</th>
                    </tr>
                  </thead>
                  <tbody>
                    {profile.ghosts.map((g) => (
                      <tr key={g.id}>
                        <td>{g.track?.name ?? (g.trackId >= 0 ? `Pista #${g.trackId}` : "Mundo aberto")}</td>
                        <td>{g.car?.name ?? `#${g.carId}`}</td>
                        <td className="num strong">{g.scoreType === 1 ? formatTime(g.score) : Math.round(g.score)}</td>
                        <td className="num muted">{Math.ceil(g.bytes / 1024)} KB</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              ) : (
                <Empty text="O fantasma do seu melhor tempo em cada evento aparece aqui (Scoreboard.UploadFile)." />
              )}
            </Card>

            <Card
              title="Estatísticas"
              icon="📊"
              extra={
                <div className="tabs">
                  {STAT_TABS.map(([key, name]) => (
                    <button key={key} className={tab === key ? "active" : ""} onClick={() => setTab(key)}>
                      {name}
                    </button>
                  ))}
                </div>
              }
            >
              <StatsTable stats={profile.profile?.[tab]?.[0] as Record<string, unknown> | undefined} catalog={catalog} />
            </Card>
          </div>

          <div className="col-side">
            <Card title="Amigos" icon="👥" extra={<span className="pill">{profile.friends.length}</span>}>
              <Friends me={profile.user.xuid} friends={profile.friends} users={users} now={now} onError={setError} />
            </Card>
            <Card title="Jogadores" icon="🎮" extra={<span className="pill">{users.length}</span>}>
              <Players users={users} selected={selected} now={now} onSelect={setSelected} onError={setError} />
            </Card>
            <Card title="Ao vivo" icon="📡" className="sticky">
              <Feed events={profile.events} now={now} />
            </Card>
            <Card title="Tráfego do jogo" icon="🛰️">
              <Traffic items={traffic} />
            </Card>
            <Card title="Conquistas" icon="🏆">
              <Achievements p={profile} />
            </Card>
            <Card title="Servidor" icon="🗄️">
              {profile.tunes || profile.user.profileTransaction != null ? (
                <dl className="kv">
                  <div>
                    <dt>Checkpoint do perfil</dt>
                    <dd>{profile.user.profileTransaction ?? "—"}</dd>
                  </div>
                  <div>
                    <dt>Drivatar</dt>
                    <dd>{profile.drivatar ? ` trechos,  envio(s)` : "ainda não enviado"}</dd>
                  </div>
                  <div>
                    <dt>Tunes salvos</dt>
                    <dd>{profile.tunes}</dd>
                  </div>
                  <div>
                    <dt>Tabelas do perfil</dt>
                    <dd>{Object.keys(profile.tables).length + (profile.garage.length ? 1 : 0)}</dd>
                  </div>
                </dl>
              ) : (
                <Empty text="Nada salvo além do login ainda." small />
              )}
            </Card>
          </div>
        </main>
      )}
    </div>
  );
}
