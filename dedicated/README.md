# dedicated_server

O servidor de free roam online do Forza Horizon 2 que um jogador roda no próprio PC (fase F0 do `PLANO_FREEROAM.md`).

No Xbox One, cada console se ligava a um servidor dedicado da Turn 10, que só repassava os objetos e os carros de cada console para os outros. O `dedicated_server` faz esse papel. Ele fala o protocolo do jogo (NerdCore, UDP) e não simula nada: o jogo de cada jogador continua dono do próprio carro.

É Rust só com a std, sem dependências. É uma versão corrigida do `relay.py` do fh2-mods (`tools/online/server`).

## Como rodar

```
dedicated_server [--config server.cfg] [--port 5101] [--verbose] [--mirror SEGUNDOS]
```

- **Sem argumentos:** lê o `server.cfg` que estiver ao lado do executável. Se o arquivo não existir, usa os padrões.
- **`--port`:** troca a porta do `server.cfg`.
- **`--verbose`:** mostra cada mensagem do protocolo.
- **`--mirror 3`:** para testar sozinho. O seu jogo recebe a si mesmo como outro jogador (o jogador 250), com o carro 3 segundos atrasado.

O console mostra quem entra (`jogador N entrou no mundo`), quem sai e por quê, e quem foi recusado.

Por enquanto (F0), o servidor ForzaInfinity manda todos os jogadores para um mundo só, o que ele recebeu em `--freeroam-world HOST:PORT`. Para um teste, o hook também aceita `[FreeRoam] World=HOST:PORT` no INI. A lista de mundos e o join pelo jogo chegam na F1.

## server.cfg

O `server.cfg` desta pasta é o exemplo comentado. Hoje valem estas chaves:

| Chave | Padrão | O que faz |
|---|---|---|
| `[World] MaxPlayers` | `8` | De 1 a 12. Quem chega com o mundo cheio é recusado. |
| `[Network] Port` | `5101` | Porta UDP do mundo. |

As outras chaves já são lidas e conferidas, mas só passam a valer nas próximas fases: `Name`, `Message`, `Visibility`, `Password` e `Announce` na F1, `Allow`/`Ban` na F2 e `UPnP` na F3. A senha nunca aparece no console.

## Portas e firewall

- O servidor escuta em **UDP 5101** (ou na `Port` do `server.cfg`), em IPv6 e IPv4 no mesmo socket.
- **Windows:** na primeira vez, aceite o aviso do Firewall do Windows para o `dedicated_server.exe` (redes privadas, e públicas se for o caso). Também dá para criar a regra à mão:
  `netsh advfirewall firewall add rule name="FH2 dedicated_server" dir=in action=allow protocol=UDP localport=5101`
- **Linux:** libere a porta no firewall, se houver um (por exemplo `sudo ufw allow 5101/udp`).
- **Pela internet:** encaminhe a porta UDP 5101 do roteador para o IP deste PC. O UPnP automático chega na F3. Na rede local não é preciso.
- **O seu próprio jogo**, no mesmo PC, entra por `127.0.0.1` (ou `::1`).

## Compilar

- **Linux:** `~/.cargo/bin/cargo build --release` e `cargo test`.
- **`.exe` a partir do Linux:** `./build-windows.sh`. Usa o zig como linker e precisa do `rustup target add x86_64-pc-windows-gnu`. A saída é `target/x86_64-pc-windows-gnu/release/dedicated_server.exe`.
- **No Windows:** `cargo build --release`.
