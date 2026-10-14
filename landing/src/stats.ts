import { useEffect, useState } from "react";
import { STATS_URL } from "./config";

/** GET /infinity/v1/stats: server-wide totals, nothing about any one player. */
export type Stats = {
  players: number;
  online: number;
  friendships: number;
  clubs: number;
  races: number;
  victories: number;
  kilometres: number;
  credits: number;
  speedTraps: number;
  cars: number;
  designs: number;
  sharedFiles: number;
  downloads: number;
  scores: number;
  ghosts: number;
  drivatars: number;
  /** Players waiting for a slot, the number of slots (0: no limit) and whether all are taken.
   *  Missing from servers before the waiting line. */
  queued?: number;
  capacity?: number;
  full?: boolean;
  updated: string;
};

export type StatsState = { stats: Stats | null; reachable: boolean | null };

/** The server's totals, refreshed every 30 s (the server caches them for as long). */
export function useStats(): StatsState {
  const [state, setState] = useState<StatsState>({ stats: null, reachable: null });
  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const reply = await fetch(STATS_URL, { cache: "no-store" });
        if (!reply.ok) throw new Error(String(reply.status));
        const stats = (await reply.json()) as Stats;
        if (alive) setState({ stats, reachable: true });
      } catch {
        if (alive) setState((s) => ({ stats: s.stats, reachable: false }));
      }
    };
    load();
    const timer = window.setInterval(load, 30_000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);
  return state;
}

export function formatNumber(n: number | undefined, locale: string): string {
  if (n === undefined) return "—";
  return new Intl.NumberFormat(locale, { notation: n >= 100_000 ? "compact" : "standard", maximumFractionDigits: 1 }).format(n);
}
