//! dedicated_server: the free roam world a player hosts. Reads server.cfg (next to the exe, or
//! --config PATH), binds UDP `[::]:<Port>` (IPv4 too) and runs the NerdCore relay (relay.rs).
//!
//! F0 uses Port and MaxPlayers; the rest of server.cfg is read and shown, and used by later
//! phases (the world list, access lists, UPnP).

use std::{
    path::PathBuf,
    process::exit,
    sync::atomic::AtomicBool,
    time::Duration,
};

use dedicated_server::{config::Config, net, relay};

fn usage() -> ! {
    println!("dedicated_server [--config server.cfg] [--port 5101] [--verbose] [--mirror SEGUNDOS]");
    println!("  --config   arquivo de configuração (padrão: server.cfg ao lado do executável)");
    println!("  --port     porta UDP (vence a do server.cfg)");
    println!("  --verbose  mostra cada mensagem do protocolo");
    println!("  --mirror   teste sozinho: o jogo vê a si mesmo como outro jogador, atrasado");
    exit(0)
}

fn main() {
    let mut config_path: Option<PathBuf> = None;
    let mut port: Option<u16> = None;
    let mut verbose = false;
    let mut mirror: Option<Duration> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next().unwrap_or_else(|| {
                eprintln!("{name} precisa de um valor");
                exit(2)
            })
        };
        match arg.as_str() {
            "--config" => config_path = Some(PathBuf::from(value("--config"))),
            "--port" => {
                port = Some(value("--port").parse().ok().filter(|p| *p != 0).unwrap_or_else(|| {
                    eprintln!("--port: número de 1 a 65535");
                    exit(2)
                }))
            }
            "--verbose" | "-v" => verbose = true,
            "--mirror" => {
                let s: f64 = value("--mirror").parse().ok().filter(|s: &f64| s.is_finite() && *s >= 0.0 && *s <= 60.0).unwrap_or_else(|| {
                    eprintln!("--mirror: segundos, de 0 a 60");
                    exit(2)
                });
                mirror = Some(Duration::from_secs_f64(s));
            }
            "-h" | "--help" => usage(),
            other => {
                eprintln!("argumento desconhecido: {other} (--help)");
                exit(2)
            }
        }
    }

    let config_path = config_path.unwrap_or_else(|| {
        let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
        dir.join("server.cfg")
    });
    let mut cfg = match Config::load(&config_path) {
        Ok(Some((cfg, warnings))) => {
            println!("Configuração: {}", config_path.display());
            for w in warnings {
                println!("  aviso: {w}");
            }
            cfg
        }
        Ok(None) => {
            println!("Configuração: {} não existe; usando os padrões", config_path.display());
            Config::default()
        }
        Err(e) => {
            eprintln!("não consegui ler {}: {e}", config_path.display());
            exit(1)
        }
    };
    if let Some(p) = port {
        cfg.port = p;
    }

    let (sock, dual) = match net::bind_dual(cfg.port) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("não consegui abrir a porta UDP {}: {e}", cfg.port);
            eprintln!("(outro programa, ou outro dedicated_server, já está usando a porta?)");
            exit(1)
        }
    };

    println!("ForzaInfinity: servidor de free roam (dedicated_server {})", env!("CARGO_PKG_VERSION"));
    println!("  porta:     UDP {} ({})", cfg.port, if dual { "IPv6 e IPv4" } else { "só IPv4: sem IPv6 nesta máquina" });
    println!("  jogadores: até {}", cfg.max_players);
    if !cfg.name.is_empty() {
        println!("  mundo:     {}", cfg.name);
    }
    if !cfg.message.is_empty() {
        println!("  mensagem:  {}", cfg.message);
    }
    println!(
        "  lista:     {}{} (anúncio ainda não implementado: {})",
        cfg.visibility.as_str(),
        if cfg.password.is_empty() { "" } else { ", com senha" },
        cfg.announce
    );
    if !cfg.allow.is_empty() || !cfg.ban.is_empty() {
        println!("  acesso:    {} liberado(s), {} banido(s) (ainda não aplicado)", cfg.allow.len(), cfg.ban.len());
    }
    if cfg.upnp {
        println!("  UPnP:      ligado (ainda não implementado: abra a porta UDP {} no roteador)", cfg.port);
    }
    if let Some(m) = mirror {
        println!("  espelho:   ligado, carro {:.1} s atrasado (jogador {})", m.as_secs_f64(), relay::MIRROR_INDEX);
    }
    println!("Pronto. Ctrl+C para fechar.");

    let settings = relay::Settings { max_players: cfg.max_players, mirror, verbose, ..Default::default() };
    let mut r = relay::Relay::new(sock, settings);
    let stop = AtomicBool::new(false);
    if let Err(e) = r.run(&stop) {
        eprintln!("erro no socket: {e}");
        exit(1);
    }
}
