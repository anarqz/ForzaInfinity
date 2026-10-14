import { useEffect, useRef, useState } from "react";

// ---------------------------------------------------------------- types (api.rs / db.rs)

export interface Presence {
  online: boolean;
  /** rich presence id the game posted (RP_FREEROAM...) */
  rich: string;
  updated: number;
}

export interface UserSummary {
  xuid: string;
  gamertag: string;
  firstSeen: number;
  lastSeen: number;
  logons: number;
  friends: number;
  presence: Presence;
}

export interface Friend {
  xuid: string;
  gamertag: string;
  favorite: boolean;
  /** follows back */
  mutual: boolean;
  presence: Presence;
}

export interface CatalogCar {
  id: number;
  name: string;
  make: string;
  model: string;
  year: number;
  classId: number;
  pi: number;
}

export interface CarClass {
  id: number;
  name: string;
  color: string;
  maxPi: number;
}

export interface Track {
  id: number;
  name: string;
  lengthM: number;
  reverse: boolean;
}

export interface Catalog {
  cars: Record<string, CatalogCar>;
  classes: Record<string, CarClass>;
  tracks: Record<string, Track>;
}

/** One row of OnlineProfile.UpdateAllTables garageCars (+ catalog entry). */
export interface GarageCar {
  id: number;
  carId: number;
  perfIndex: number;
  classID: number;
  distanceDriven: number;
  timeDriven: number;
  totalWinnings: number;
  numVictories: number;
  numPodiums: number;
  numRaces: number;
  originalOwner: string;
  simPeakPower: number;
  speedRating: number;
  accelerationRating: number;
  launchRating: number;
  brakingRating: number;
  handlingRating: number;
  car: CatalogCar | null;
  [key: string]: unknown;
}

export interface Score {
  scoreId: string;
  time: number;
  scoreType: number;
  scoreboardType: number;
  gameScoreboardId: number;
  trackId: number;
  track: Track | null;
  carId: number;
  car: CatalogCar | null;
  carClassId: number;
  score: number;
  isClean: boolean;
}

export interface FeedEvent {
  id: number;
  xuid: string;
  time: number;
  kind: string;
  title: string;
  detail: unknown;
}

export interface TrafficItem {
  seq: number;
  time: number;
  xuid: string;
  label: string;
  status: number;
  source: string | null;
}

type Stats = Record<string, number | boolean | string>;

export interface Profile {
  user: {
    xuid: string;
    gamertag: string;
    firstSeen: number;
    lastSeen: number;
    logons: number;
    careerLevel: number | null;
    badgeId: number | null;
    titleId: number | null;
    profileTransaction: number | null;
  };
  profile: null | {
    playerHistory?: Stats[];
    playerRaceHistory?: Stats[];
    compareStats?: Stats[];
    miscStats?: Stats[];
    drivatarStats?: Stats[];
    assists?: Stats[];
    [table: string]: unknown;
  };
  profileUpdated: number | null;
  garage: GarageCar[];
  tables: Record<string, Stats[]>;
  scores: Score[];
  xp: { time: number; xp: number }[];
  tunes: number;
  events: FeedEvent[];
  achievements: { id: string; name: string; unlocked: boolean; time?: number }[];
  ghosts: { id: number; scoreType: number; trackId: number; track: Track | null; score: number; carId: number; car: CatalogCar | null; time: number; bytes: number }[];
  drivatar: null | { updated: number; saves: number; segments: number; curves: number };
  friends: Friend[];
  presence: Presence;
}

export interface ClubMember {
  xuid: string;
  gamertag: string;
  /** 1 leader, 2 co-leader, 3 member */
  role: number;
  joined: number;
  presence: Presence;
}

export interface Club {
  id: number;
  tag: string;
  name: string;
  description: string;
  /** 0 open, 1 open to friends, 2 invite only, 3 private */
  privacy: number;
  region: number;
  attitude: number;
  interest: number;
  xp: number;
  created: number;
  members: ClubMember[];
  invites: { id: number; xuid: string; gamertag: string; sender: string; senderGamertag: string; created: number }[];
  requests: { xuid: string; gamertag: string; created: number }[];
  notices: { id: number; xuid: string; gamertag: string; message: string; created: number }[];
}

export interface Keyword {
  id: number;
  /** 0 region, 1 attitude, 2 interest */
  category: number;
  text: string;
}

export const ROLES: Record<number, string> = { 1: "Líder", 2: "Vice-líder", 3: "Membro" };
export const PRIVACY = ["Aberto", "Aberto para amigos", "Só por convite", "Privado"];

/** The text Xbox shows for a rich presence id (same as forza::rich_presence_text). */
export function richText(p: Presence): string {
  if (!p.online) return p.updated ? "offline" : "nunca jogou";
  const known: Record<string, string> = { RP_SPLASH: "Na tela inicial", RP_FREEROAM: "Explorando o sul da Europa" };
  return known[p.rich] ?? (p.rich ? p.rich.replace(/^RP_/, "").replace(/_/g, " ").toLowerCase() : "Jogando");
}

// ---------------------------------------------------------------- formatting

const nf = new Intl.NumberFormat("pt-BR");
const nf1 = new Intl.NumberFormat("pt-BR", { maximumFractionDigits: 1 });

export const fmt = (n: unknown) => (typeof n === "number" && isFinite(n) ? nf.format(Math.round(n)) : "—");
export const fmt1 = (n: unknown) => (typeof n === "number" && isFinite(n) ? nf1.format(n) : "—");
/** Distances come in meters. */
export const km = (m: unknown) => (typeof m === "number" ? `${nf1.format(m / 1000)} km` : "—");
/** Speeds come in m/s. */
export const kmh = (ms: unknown) => (typeof ms === "number" ? `${nf.format(Math.round(ms * 3.6))} km/h` : "—");
export const hours = (s: unknown) =>
  typeof s === "number" ? (s < 3600 ? `${Math.round(s / 60)} min` : `${nf1.format(s / 3600)} h`) : "—";

export function ago(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 10) return "agora";
  if (s < 60) return `há ${s} s`;
  const m = Math.round(s / 60);
  if (m < 60) return `há ${m} min`;
  const h = Math.round(m / 60);
  if (h < 24) return `há ${h} h`;
  return new Date(ms).toLocaleDateString("pt-BR", { day: "2-digit", month: "short" });
}

export const clock = (ms: number) =>
  new Date(ms).toLocaleTimeString("pt-BR", { hour: "2-digit", minute: "2-digit", second: "2-digit" });

/** "numPerfectDrifts" -> "num perfect drifts" -> label via dictionary or spaced words. */
const LABELS: Record<string, string> = {
  level: "Nível",
  xp: "XP",
  totalWinnings: "Prêmios totais",
  onlineWinnings: "Prêmios online",
  distanceDriven: "Distância",
  timeDriven: "Tempo dirigindo",
  topSpeed: "Velocidade máxima",
  longestJump: "Maior salto",
  longestDrift: "Maior drift",
  numVictories: "Vitórias",
  numPodiums: "Pódios",
  numRaces: "Corridas",
  numCreditsSpent: "Créditos gastos",
  numCreditsEarned: "Créditos ganhos",
  numSkillTokens: "Skill tokens",
  highestSkillScore: "Maior pontuação de skill",
  credits: "Créditos",
  carsInGarage: "Carros na garagem",
  garageValue: "Valor da garagem",
  barnFinds: "Barn finds",
  speedTrapsTriggered: "Radares",
  speedZonesTriggered: "Zonas de velocidade",
  roadsDiscovered: "Estradas descobertas",
  destinationsVisited: "Destinos visitados",
  showcasesWon: "Showcases vencidos",
  photosTaken: "Fotos tiradas",
  totalCollisions: "Colisões",
  wristbandLevel: "Pulseira",
};

export function label(key: string): string {
  if (LABELS[key]) return LABELS[key];
  const words = key
    .replace(/^num(?=[A-Z])/, "")
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/([A-Z])([A-Z][a-z])/g, "$1 $2")
    .toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

// ---------------------------------------------------------------- data hooks

export async function getJson<T>(url: string): Promise<T> {
  const r = await fetch(url, { cache: "no-store" });
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  return r.json() as Promise<T>;
}

/** POST / PUT / DELETE on the manager API; throws with the server's error text. */
export async function sendJson<T>(method: string, url: string, body?: unknown): Promise<T> {
  const r = await fetch(url, {
    method,
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error((data as { error?: string }).error ?? `HTTP ${r.status}`);
  return data as T;
}

export type LiveState = "connecting" | "live" | "offline";

interface LiveHandlers {
  onEvent: (e: FeedEvent) => void;
  onRequest: (r: TrafficItem) => void;
  onHello: (users: UserSummary[]) => void;
  /** players list changed (friends, presence, created / removed) */
  onUsers?: (users: UserSummary[]) => void;
  /** clubs changed */
  onClubs?: () => void;
}

/** WebSocket to /ws with automatic reconnect. */
export function useLive(handlers: LiveHandlers): LiveState {
  const [state, setState] = useState<LiveState>("connecting");
  const ref = useRef(handlers);
  ref.current = handlers;

  useEffect(() => {
    let socket: WebSocket | null = null;
    let timer: number | undefined;
    let delay = 500;
    let stopped = false;

    const connect = () => {
      const url = `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`;
      socket = new WebSocket(url);
      setState("connecting");
      socket.onopen = () => {
        delay = 500;
        setState("live");
      };
      socket.onmessage = (m) => {
        const msg = JSON.parse(m.data as string);
        if (msg.type === "event") ref.current.onEvent(msg.event);
        else if (msg.type === "request") ref.current.onRequest(msg);
        else if (msg.type === "hello") ref.current.onHello(msg.users);
        else if (msg.type === "users") ref.current.onUsers?.(msg.users);
        else if (msg.type === "clubs") ref.current.onClubs?.();
      };
      socket.onclose = () => {
        setState("offline");
        if (!stopped) {
          timer = window.setTimeout(connect, delay);
          delay = Math.min(delay * 2, 8000);
        }
      };
    };
    connect();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
      socket?.close();
    };
  }, []);
  return state;
}

/** Re-renders every `ms` so relative times ("há 3 min") stay fresh. */
export function useNow(ms = 15000): number {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), ms);
    return () => window.clearInterval(t);
  }, [ms]);
  return now;
}

/** Animates from the previous value to the new one (count-up). */
export function useCountUp(value: number, duration = 700): number {
  const [shown, setShown] = useState(value);
  const from = useRef(value);
  useEffect(() => {
    const start = performance.now();
    const a = from.current;
    let frame = 0;
    const tick = (t: number) => {
      const k = Math.min(1, (t - start) / duration);
      const eased = 1 - Math.pow(1 - k, 3);
      setShown(a + (value - a) * eased);
      if (k < 1) frame = requestAnimationFrame(tick);
      else from.current = value;
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [value, duration]);
  return shown;
}
