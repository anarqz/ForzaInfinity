import { useState, type FormEvent } from "react";
import { Empty } from "./components";
import { ago, richText, sendJson, type Friend, type Presence, type UserSummary } from "./lib";

function Dot({ p }: { p: Presence }) {
  return <span className={`presence-dot ${p.online ? "on" : ""}`} title={p.online ? "online" : "offline"} />;
}

function Avatar({ xuid, size = 32 }: { xuid: string; size?: number }) {
  return <img className="social-avatar" src={`/api/gamerpic/${xuid}.png`} width={size} height={size} alt="" />;
}

/** Friends of the selected player: what the game gets from social / People / presence. */
export function Friends(props: { me: string; friends: Friend[]; users: UserSummary[]; now: number; onError: (e: string) => void }) {
  const { me, friends, users, now, onError } = props;
  const [pick, setPick] = useState("");
  const candidates = users.filter((u) => u.xuid !== me && !friends.some((f) => f.xuid === u.xuid));

  const run = (p: Promise<unknown>) => p.catch((e: Error) => onError(e.message));
  const add = (e: FormEvent) => {
    e.preventDefault();
    if (pick) run(sendJson("POST", `/api/users/${me}/friends`, { xuid: pick }).then(() => setPick("")));
  };

  return (
    <div className="social">
      {friends.length ? (
        <ul className="social-list">
          {friends.map((f) => (
            <li key={f.xuid}>
              <Avatar xuid={f.xuid} />
              <div className="social-who">
                <b>
                  <Dot p={f.presence} /> {f.gamertag || f.xuid}
                </b>
                <span className="muted small">
                  {richText(f.presence)}
                  {!f.presence.online && f.presence.updated ? ` · ${ago(f.presence.updated, now)}` : ""}
                  {!f.mutual && " · não segue de volta"}
                </span>
              </div>
              <button
                className={`icon-btn ${f.favorite ? "fav" : ""}`}
                title={f.favorite ? "Tirar dos favoritos" : "Favoritar"}
                onClick={() => run(sendJson("PUT", `/api/users/${me}/friends/${f.xuid}`, { favorite: !f.favorite }))}
              >
                ★
              </button>
              <button
                className="icon-btn danger"
                title="Desfazer amizade"
                onClick={() => run(sendJson("DELETE", `/api/users/${me}/friends/${f.xuid}`))}
              >
                ✕
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <Empty text="Sem amigos ainda. No jogo eles aparecem na lista de amigos, com presença." small />
      )}
      <form className="social-form" onSubmit={add}>
        <select value={pick} onChange={(e) => setPick(e.target.value)} disabled={!candidates.length}>
          <option value="">{candidates.length ? "Adicionar amigo…" : "Nenhum outro jogador"}</option>
          {candidates.map((u) => (
            <option key={u.xuid} value={u.xuid}>
              {u.gamertag || u.xuid}
            </option>
          ))}
        </select>
        <button className="btn" disabled={!pick}>
          Adicionar
        </button>
      </form>
    </div>
  );
}

/** Every player on the server: create test / friend accounts, remove them. */
export function Players(props: { users: UserSummary[]; selected: string | null; now: number; onSelect: (x: string) => void; onError: (e: string) => void }) {
  const { users, selected, now, onSelect, onError } = props;
  const [tag, setTag] = useState("");
  const create = (e: FormEvent) => {
    e.preventDefault();
    sendJson<{ xuid: string }>("POST", "/api/users", { gamertag: tag.trim() })
      .then(() => setTag(""))
      .catch((err: Error) => onError(err.message));
  };
  const remove = (u: UserSummary) => {
    if (confirm(`Remover ${u.gamertag || u.xuid} e todos os dados dele?`)) {
      sendJson("DELETE", `/api/users/${u.xuid}`).catch((err: Error) => onError(err.message));
    }
  };

  return (
    <div className="social">
      <ul className="social-list">
        {users.map((u) => (
          <li key={u.xuid} className={u.xuid === selected ? "selected" : ""} onClick={() => onSelect(u.xuid)}>
            <Avatar xuid={u.xuid} size={28} />
            <div className="social-who">
              <b>
                <Dot p={u.presence} /> {u.gamertag || u.xuid}
              </b>
              <span className="muted small">
                {u.logons ? `${richText(u.presence)} · visto ${ago(u.lastSeen, now)}` : "criado no manager"} · {u.friends} amigo(s)
              </span>
            </div>
            <button
              className="icon-btn danger"
              title="Remover jogador"
              onClick={(e) => {
                e.stopPropagation();
                remove(u);
              }}
            >
              ✕
            </button>
          </li>
        ))}
      </ul>
      <form className="social-form" onSubmit={create}>
        <input value={tag} maxLength={15} placeholder="Gamertag do novo jogador" onChange={(e) => setTag(e.target.value)} />
        <button className="btn" disabled={!tag.trim()}>
          Criar
        </button>
      </form>
    </div>
  );
}
