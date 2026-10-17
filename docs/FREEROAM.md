# Free roam online

ForzaInfinity brings back Forza Horizon 2's online free roam and races: 8 drivers by default (up
to 12, `MaxPlayers` in `server.cfg`) in the same festival, on worlds that **players host on
their own PC**. HARDCORE and NOOB players have their own worlds: one never sees the other's. No ForzaInfinity server carries
anyone's game traffic: the players' consoles connect to each other directly, and the server only
lists the worlds and introduces the players.

## Hosting a world

- **Settings › INFINITY › HOST WORLD**: a public world. It shows in FIND WORLD for everyone, and
  your friends' FREE ROAM ONLINE goes straight to it.
- **The game's PRIVATE ONLINE SESSION › PRIVATE FREE ROAM**: an unlisted world. Only players who
  pick JOIN WORLD on your player card reach it.
- To close it: Start › HORIZON SOLO (or close the game: the world closes with it).

The world runs `dedicated_server.exe` (in the game's `data/bin`) hidden, with its settings in
`server.cfg` next to it (name, message, password, player limit, port). On Windows, the first
time, the firewall asks about it: allow it on **private networks**.

## Joining

- **FREE ROAM ONLINE** joins a world hosted by a friend (players who added each other).
- **Settings › INFINITY › FIND WORLD** shows the other public worlds one by one, with a warning:
  a world of someone you do not know can be unsafe. Prefer playing with people you know.
- **JOIN WORLD** on a player's card joins the world they are in.
- Everyone in a world needs the same car database (the same game files): a changed one cannot
  join, and is dropped if it changes during the session.

## Races

Start an event in an online world (drive to its marker and accept it) and everyone else in the
world gets an invite for 15 seconds. Whoever joins lines up on the same grid, sees the others on
the HUD and in the standings, and gets the results, XP and credits at the end; each player's
time goes to the route's leaderboard and rivals, as in an offline race. Players who stay in free
roam can watch the race go by. The world's host runs the race (`[Events] Races=1` in
`server.cfg`, on by default); a world whose host turned races off sends you back to free roam
with a notice.

## How players reach each other

When you join a world on another network, ForzaInfinity:

1. asks two public STUN servers (Google's and Cloudflare's) what your address on the Internet is;
2. trades addresses with the host through the ForzaInfinity server;
3. has both sides send packets to each other at once (**hole punching**), which opens both
   routers for this connection without any setting;
4. if the host's router has **UPnP**, the host's world also asks it to open UDP 5101 by itself.

No port has to be opened by hand on most home networks.

## Settings › INFINITY › TEST CONNECTION

The **FREE ROAM** part says what your network allows:

| It says | What it means |
|---|---|
| **NAT: moderate (cone)** | The usual home network. You join most worlds and most players join yours. |
| **NAT: strict (symmetric)** | You join worlds whose host is moderate or open. Two strict networks cannot meet. See the fixes below. |
| **No answer from the STUN servers** | The network blocks UDP (a firewall, a school or work network). Free roam with players outside it will not work there. |
| **IPv6: yes** | Your provider gives you IPv6, which usually connects without any NAT at all. |

While you host, **YOUR WORLD** adds the address players reach it at, whether the router's
UPnP opened its port, and whether your router sits behind another router or the operator's
CGNAT.

## Fixes, easiest first

1. **Turn on UPnP in your router** (often under *NAT forwarding*, *Advanced* or *Gaming*). Mesh
   systems (Deco, eero, Google/Nest Wifi) usually have it in their app.
2. **Turn on IPv6** in the router, or ask your operator for it.
3. **Two routers** (the operator's modem plus your own): put the operator's modem in **bridge**
   mode, so that only one of them does NAT.
4. **CGNAT** (the operator shares one public address among many customers; TEST CONNECTION's
   *behind another router or the operator's CGNAT*): ask the operator for a **public IP**. In
   Brazil it is often free on request.
5. **Forward UDP 5101** by hand to the PC that hosts (the port is `Port=` in `server.cfg`).
6. **Windows firewall**: allow `dedicated_server.exe` (and `azimuth.exe`) on private networks.
   *Windows Security › Firewall & network protection › Allow an app through firewall*.

## For the curious

- `[FreeRoam] Direct=1` in `FH2_ExtraOptions.ini` turns the joining side's proxy off (the game
  then sends straight to the world's address), for troubleshooting only.
- What a world found about its network is in `data/bin/dedicated_server.net`; its log is
  `dedicated_server.log`; the game's side is in `FH2_ExtraOptions.log` (`PROXY:` and `WORLD:`
  lines). **ForzaInfinity-Report** collects them.
