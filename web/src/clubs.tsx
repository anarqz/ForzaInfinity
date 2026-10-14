import { useState, type FormEvent } from "react";
import { Empty } from "./components";
import { ago, PRIVACY, richText, ROLES, sendJson, type Club, type Keyword, type UserSummary } from "./lib";

type OnError = (e: string) => void;

function KeywordSelect(props: { keywords: Keyword[]; category: number; value: number; onChange: (v: number) => void }) {
  return (
    <select value={props.value} onChange={(e) => props.onChange(Number(e.target.value))}>
      <option value={0}>{["Região", "Atitude", "Interesse"][props.category]}: —</option>
      {props.keywords
        .filter((k) => k.category === props.category)
        .map((k) => (
          <option key={k.id} value={k.id}>
            {k.text}
          </option>
        ))}
    </select>
  );
}

function CreateClub(props: { users: UserSummary[]; clubs: Club[]; keywords: Keyword[]; onError: OnError }) {
  const free = props.users.filter((u) => !props.clubs.some((c) => c.members.some((m) => m.xuid === u.xuid)));
  const [owner, setOwner] = useState("");
  const [tag, setTag] = useState("");
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [privacy, setPrivacy] = useState(0);
  const [kw, setKw] = useState({ region: 0, attitude: 0, interest: 0 });
  const submit = (e: FormEvent) => {
    e.preventDefault();
    sendJson("POST", "/api/clubs", { owner, tag, name, description, privacy, ...kw })
      .then(() => {
        setTag("");
        setName("");
        setDescription("");
      })
      .catch((err: Error) => props.onError(err.message));
  };
  return (
    <form className="club-create" onSubmit={submit}>
      <select value={owner} onChange={(e) => setOwner(e.target.value)}>
        <option value="">Líder (jogador sem clube)…</option>
        {free.map((u) => (
          <option key={u.xuid} value={u.xuid}>
            {u.gamertag || u.xuid}
          </option>
        ))}
      </select>
      <input value={tag} maxLength={4} placeholder="Tag" className="tag-input" onChange={(e) => setTag(e.target.value.toUpperCase())} />
      <input value={name} maxLength={24} placeholder="Nome do clube" onChange={(e) => setName(e.target.value)} />
      <input value={description} maxLength={200} placeholder="Descrição" className="wide" onChange={(e) => setDescription(e.target.value)} />
      <select value={privacy} onChange={(e) => setPrivacy(Number(e.target.value))}>
        {PRIVACY.map((p, i) => (
          <option key={p} value={i}>
            {p}
          </option>
        ))}
      </select>
      <KeywordSelect keywords={props.keywords} category={0} value={kw.region} onChange={(v) => setKw({ ...kw, region: v })} />
      <KeywordSelect keywords={props.keywords} category={1} value={kw.attitude} onChange={(v) => setKw({ ...kw, attitude: v })} />
      <KeywordSelect keywords={props.keywords} category={2} value={kw.interest} onChange={(v) => setKw({ ...kw, interest: v })} />
      <button className="btn" disabled={!owner || tag.length < 2 || !name.trim()}>
        Criar clube
      </button>
    </form>
  );
}

function ClubCard(props: { club: Club; users: UserSummary[]; keywords: Keyword[]; now: number; onError: OnError }) {
  const { club, users, keywords, now, onError } = props;
  const [invitee, setInvitee] = useState("");
  const [notice, setNotice] = useState("");
  const run = (p: Promise<unknown>) => p.catch((e: Error) => onError(e.message));
  const base = `/api/clubs/${club.id}`;
  const leader = club.members.find((m) => m.role === 1);
  const outside = users.filter((u) => !club.members.some((m) => m.xuid === u.xuid) && !club.invites.some((i) => i.xuid === u.xuid));
  const kwText = [club.region, club.attitude, club.interest]
    .map((id) => keywords.find((k) => k.id === id)?.text)
    .filter(Boolean)
    .join(" · ");

  return (
    <article className="club">
      <header className="club-head">
        <span className="club-tag">{club.tag}</span>
        <div className="club-title">
          <b>{club.name}</b>
          <span className="muted small">
            {club.members.length} membro(s) · criado {ago(club.created, now)}
            {kwText && ` · ${kwText}`}
          </span>
        </div>
        <select
          value={club.privacy}
          title="Privacidade"
          onChange={(e) => run(sendJson("PUT", base, { privacy: Number(e.target.value) }))}
        >
          {PRIVACY.map((p, i) => (
            <option key={p} value={i}>
              {p}
            </option>
          ))}
        </select>
        <button
          className="icon-btn danger"
          title="Apagar clube"
          onClick={() => confirm(`Apagar o clube [${club.tag}] ${club.name}?`) && run(sendJson("DELETE", base))}
        >
          ✕
        </button>
      </header>
      {club.description && <p className="club-desc">{club.description}</p>}

      <ul className="social-list">
        {club.members.map((m) => (
          <li key={m.xuid}>
            <img className="social-avatar" src={`/api/gamerpic/${m.xuid}.png`} width={26} height={26} alt="" />
            <div className="social-who">
              <b>
                <span className={`presence-dot ${m.presence.online ? "on" : ""}`} /> {m.gamertag || m.xuid}
              </b>
              <span className="muted small">{richText(m.presence)}</span>
            </div>
            <select value={m.role} onChange={(e) => run(sendJson("PUT", `${base}/members/${m.xuid}`, { role: Number(e.target.value) }))}>
              {Object.entries(ROLES).map(([v, t]) => (
                <option key={v} value={v}>
                  {t}
                </option>
              ))}
            </select>
            <button className="icon-btn danger" title="Remover do clube" onClick={() => run(sendJson("DELETE", `${base}/members/${m.xuid}`))}>
              ✕
            </button>
          </li>
        ))}
      </ul>

      {club.requests.length > 0 && (
        <div className="club-section">
          <h4>Pedidos para entrar</h4>
          {club.requests.map((r) => (
            <div key={r.xuid} className="club-row">
              <span>
                {r.gamertag || r.xuid} <span className="muted small">{ago(r.created, now)}</span>
              </span>
              <span>
                <button className="btn small" onClick={() => run(sendJson("POST", `${base}/members`, { xuid: r.xuid }))}>
                  Aprovar
                </button>
                <button className="icon-btn danger" title="Recusar" onClick={() => run(sendJson("DELETE", `${base}/requests/${r.xuid}`))}>
                  ✕
                </button>
              </span>
            </div>
          ))}
        </div>
      )}

      <div className="club-section">
        <h4>Convites</h4>
        {club.invites.map((i) => (
          <div key={i.id} className="club-row">
            <span>
              {i.gamertag || i.xuid}{" "}
              <span className="muted small">
                por {i.senderGamertag || "manager"} · {ago(i.created, now)}
              </span>
            </span>
            <span>
              <button className="btn small" title="Aceitar em nome do jogador" onClick={() => run(sendJson("POST", `${base}/members`, { xuid: i.xuid }))}>
                Aceitar
              </button>
              <button className="icon-btn danger" title="Cancelar convite" onClick={() => run(sendJson("DELETE", `${base}/invites/${i.id}`))}>
                ✕
              </button>
            </span>
          </div>
        ))}
        <form
          className="social-form"
          onSubmit={(e) => {
            e.preventDefault();
            run(sendJson("POST", `${base}/invites`, { xuid: invitee, sender: leader?.xuid ?? "0" }).then(() => setInvitee("")));
          }}
        >
          <select value={invitee} onChange={(e) => setInvitee(e.target.value)}>
            <option value="">{outside.length ? "Convidar jogador…" : "Ninguém para convidar"}</option>
            {outside.map((u) => (
              <option key={u.xuid} value={u.xuid}>
                {u.gamertag || u.xuid}
              </option>
            ))}
          </select>
          <button className="btn" disabled={!invitee}>
            Convidar
          </button>
        </form>
      </div>

      <div className="club-section">
        <h4>Mural</h4>
        {club.notices.map((n) => (
          <div key={n.id} className="club-row notice">
            <span>
              <b>{n.gamertag || "manager"}:</b> {n.message} <span className="muted small">{ago(n.created, now)}</span>
            </span>
            <button className="icon-btn danger" title="Apagar aviso" onClick={() => run(sendJson("DELETE", `${base}/notices/${n.id}`))}>
              ✕
            </button>
          </div>
        ))}
        <form
          className="social-form"
          onSubmit={(e) => {
            e.preventDefault();
            run(sendJson("POST", `${base}/notices`, { xuid: leader?.xuid ?? "0", message: notice }).then(() => setNotice("")));
          }}
        >
          <input value={notice} maxLength={200} placeholder={`Aviso como ${leader?.gamertag ?? "manager"}`} onChange={(e) => setNotice(e.target.value)} />
          <button className="btn" disabled={!notice.trim()}>
            Postar
          </button>
        </form>
      </div>
    </article>
  );
}

export function Clubs(props: { clubs: Club[]; keywords: Keyword[]; users: UserSummary[]; now: number; onError: OnError }) {
  return (
    <div className="clubs">
      {props.clubs.length ? (
        props.clubs.map((c) => <ClubCard key={c.id} club={c} users={props.users} keywords={props.keywords} now={props.now} onError={props.onError} />)
      ) : (
        <Empty text="Nenhum clube ainda. Crie um aqui ou pelo jogo (Xls Club.CreateClub)." small />
      )}
      <CreateClub users={props.users} clubs={props.clubs} keywords={props.keywords} onError={props.onError} />
    </div>
  );
}
