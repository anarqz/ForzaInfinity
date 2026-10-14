# FH2Online

Projeto do Alex: devolver os serviços online do Forza Horizon 2, que roda no PC pelo Azimuth 0.9.7. Isso é feito com um hook no `azimuth-tls.dll` (fork do ExtraOptions) e um backend próprio em Rust.

**Antes de qualquer coisa, leia `HANDOFF.md`.** Ele tem o estado atual, os endereços, o que já está provado e os próximos passos.

## Convenções

- Responda ao Alex em português.
- `hook/` é C freestanding: só Kernel32, sem CRT, sem arrays grandes na pilha, arquivos com CRLF e `-Werror`. Não mexa no layout do TLS (veja HANDOFF.md).
- `server/` é Rust só com std (sem tokio/axum, que exigem `dlltool` no toolchain GNU).
- Troque o DLL instalado só com o jogo fechado, e confira o SHA-256 depois de copiar.
- Não apague o `azimuth-tls.dll.stock` (backup do original) em `azimuth-windows\data\bin`.
- Commits no fork vão para a branch `fh2online`, com remote `upstream`.
- Próxima tarefa: extrair o esquema `Forza.WebServices` do executável do jogo e implementar o `User.Logon` no servidor.
