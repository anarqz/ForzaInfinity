import { useState, type FormEvent } from "react";
import { Empty } from "./components";
import { ago, sendJson, type Friend, type InboxItem } from "./lib";

const ICONS: Record<number, string> = {
  0x0200: "✉️", 0x0700: "🙋", 0x0701: "📨", 0x0702: "⚠️", 0x0703: "🚪", 0x0704: "⬇️", 0x0705: "✅",
  0x0706: "👋", 0x0707: "🆕", 0x0708: "⬆️", 0x0709: "📌", 0x8403: "⏱️", 0x8500: "💰",
};

/** The player's in-game inbox (Xls.Notifications): what the game shows in its message center. */
export function Inbox(props: { me: string; items: InboxItem[]; friends: Friend[]; now: number; onError: (e: string) => void }) {
  const { me, items, friends, now, onError } = props;
  const [message, setMessage] = useState("");
  const [rival, setRival] = useState("");
  const run = (p: Promise<unknown>) => p.catch((e: Error) => onError(e.message));
  const send = (e: FormEvent) => {
    e.preventDefault();
    run(sendJson("POST", `/api/users/${me}/notifications`, { kind: "message", message }).then(() => setMessage("")));
  };

  return (
    <div className="social">
      {items.length ? (
        <ul className="inbox">
          {items.map((n) => (
            <li key={n.id} className={n.read ? "read" : ""}>
              <span className="inbox-icon">{ICONS[n.type] ?? "🔔"}</span>
              <span className="inbox-text">
                {n.text}
                <span className="muted small">
                  {" "}
                  · {ago(n.sent, now)}
                  {n.read ? " · lida" : ""}
                </span>
              </span>
              <button className="icon-btn danger" title="Apagar" onClick={() => run(sendJson("DELETE", `/api/notifications/${n.id}`))}>
                ✕
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <Empty text="Nada na caixa de entrada. Desafios de rivais, convites e avisos de clube chegam aqui e no jogo." small />
      )}
      <form className="social-form" onSubmit={send}>
        <input value={message} maxLength={200} placeholder="Mensagem para o jogo" onChange={(e) => setMessage(e.target.value)} />
        <button className="btn" disabled={!message.trim()}>
          Enviar
        </button>
      </form>
      <form
        className="social-form"
        onSubmit={(e) => {
          e.preventDefault();
          run(sendJson("POST", `/api/users/${me}/notifications`, { kind: "rival", from: rival }));
        }}
      >
        <select value={rival} onChange={(e) => setRival(e.target.value)} disabled={!friends.length}>
          <option value="">{friends.length ? "Desafio de teste de…" : "Sem amigos para desafiar"}</option>
          {friends.map((f) => (
            <option key={f.xuid} value={f.xuid}>
              {f.gamertag || f.xuid}
            </option>
          ))}
        </select>
        <button className="btn" disabled={!rival} title="O amigo 'bate' seu último tempo por 1 a 3%">
          Desafiar
        </button>
      </form>
    </div>
  );
}
