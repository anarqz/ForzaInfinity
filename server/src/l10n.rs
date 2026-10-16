//! The account endpoints' messages in the player's language: Portuguese (Brazil) when the hook
//! says the game runs in it (X-Fh2-Lang: pt-BR), English otherwise. Messages are written in
//! English as before and translated last: `tr` finds the English template (`{}` where a name
//! or a number goes) and fills the Portuguese one with the same values, in order.

/// (English, Portuguese); `{}` marks the same values in both.
const TEXTS: &[(&str, &str)] = &[
    ("Account suspended on this server.", "Conta suspensa neste servidor."),
    ("The server's save comes first: restart the game to get it, then play on.", "O save do servidor vem primeiro: reinicie o jogo para recebê-lo e continue jogando."),
    ("New season! Your XP, level, credits, records and garage (all but your first car) started again. Your career, liveries and creations are kept. See you on the leaderboards!", "Nova temporada! Seu XP, nível, créditos, recordes e garagem (menos o seu primeiro carro) começaram de novo. Sua carreira, pinturas e criações continuam. Nos vemos nos placares!"),
    ("Moderation found progress on this profile that could not be real and reset it: XP, level, credits, records and the garage (all but your first car). Your career and liveries are kept.", "A moderação encontrou neste perfil um progresso que não poderia ser real e o zerou: XP, nível, créditos, recordes e a garagem (menos o seu primeiro carro). Sua carreira e suas pinturas continuam."),
    ("Your save had progress this server could not accept, so the server's save was restored. Restart the game to get it.", "Seu save tinha um progresso que este servidor não pôde aceitar, então o save do servidor foi restaurado. Reinicie o jogo para recebê-lo."),
    ("A new season started: XP, level, credits, records and the garage (all but your first car) start again; your career, liveries and creations stay. Restart the game to get it.", "Uma nova temporada começou: XP, nível, créditos, recordes e a garagem (menos o seu primeiro carro) começam de novo; sua carreira, pinturas e criações continuam. Reinicie o jogo para recebê-la."),
    ("Moderation reset this profile's progress. Restart the game to get it; your liveries were kept.", "A moderação zerou o progresso deste perfil. Reinicie o jogo para recebê-lo; suas pinturas foram mantidas."),
    (
        "This game's car database is not the original one: only the game's own cars play online on this server. Put the original gamedbRC.slt back (or reinstall the game) and restart.",
        "O banco de carros deste jogo não é o original: neste servidor, só os carros do próprio jogo jogam online. Coloque o gamedbRC.slt original de volta (ou reinstale o jogo) e reinicie.",
    ),
    ("Unknown token.", "Token desconhecido."),
    ("This server is not taking new profiles right now.", "Este servidor não está aceitando perfis novos no momento."),
    ("Could not delete the account.", "Não foi possível excluir a conta."),
    ("Could not make a key.", "Não foi possível gerar a chave."),
    ("That is your own profile.", "Esse é o seu próprio perfil."),
    ("That player has no profile on this server.", "Esse jogador não tem perfil neste servidor."),
    ("Too many friend requests. Try again later.", "Pedidos de amizade demais. Tente de novo mais tarde."),
    ("There is no profile {} on this server.", "Não existe o perfil {} neste servidor."),
    ("You and {} are already friends and rivals.", "Você e {} já são amigos e rivais."),
    ("You and {} are now friends and rivals!", "Você e {} agora são amigos e rivais!"),
    (
        "Request sent to {}. Once {} adds you too, you become friends and rivals.",
        "Pedido enviado para {}. Quando {} adicionar você também, vocês viram amigos e rivais.",
    ),
    ("{} is no longer your friend.", "{} não é mais seu amigo."),
    ("Thanks. Your report on {} was sent to moderation.", "Obrigado. Sua denúncia contra {} foi enviada à moderação."),
    ("Too many reports today. Try again tomorrow.", "Denúncias demais hoje. Tente de novo amanhã."),
    (
        "Only players whose profile is a day old and has played online can report.",
        "Só pode denunciar quem tem o perfil há pelo menos um dia e já jogou online.",
    ),
    ("You already reported {}.", "Você já denunciou {}."),
    ("{} was not your friend.", "{} não era seu amigo."),
    ("Wrong profile or key.", "Perfil ou chave errados."),
    ("No cloud save yet.", "Ainda não há save na nuvem."),
    ("The name must have 1 to 20 letters or digits and start with a letter.", "O nome precisa ter de 1 a 20 letras ou números e começar com uma letra."),
    ("This PC already has the account {} on this server.", "Este PC já tem a conta {} neste servidor."),
    (
        "This PC already has {} profiles on this server ({}). Play one of them with Switch profile, or delete one there first.",
        "Este PC já tem {} perfis neste servidor ({}). Jogue com um deles em Trocar perfil, ou exclua um lá antes.",
    ),
    ("This profile was not created on this PC: sign in with it and its site key.", "Este perfil não foi criado neste PC: entre com ele e a chave do site dele."),
    ("{} already exists on this server.", "{} já existe neste servidor."),
    // player-hosted worlds (worlds.rs)
    ("There is no such world.", "Esse mundo não existe."),
    ("This world is closed.", "Este mundo está fechado."),
    ("You cannot join this world.", "Você não pode entrar neste mundo."),
    ("Only the host's friends can join this world.", "Só os amigos do anfitrião podem entrar neste mundo."),
    ("This world is full.", "Este mundo está cheio."),
    ("Too many wrong passwords. Try again later.", "Senhas erradas demais. Tente de novo mais tarde."),
    ("Too many worlds on this server right now. Try again later.", "Mundos demais neste servidor agora. Tente de novo mais tarde."),
    ("Free roam is off on this server.", "O free roam online está desligado neste servidor."),
    // Message Center notices (in the receiver's language)
    (
        "You and {} are now friends and rivals. Compare times in ghost races!",
        "Você e {} agora são amigos e rivais. Comparem os tempos nas corridas de fantasma!",
    ),
    (
        "{} wants to be your friend. Add {} in Settings > Infinity to become rivals.",
        "{} quer ser seu amigo. Adicione {} em Configurações > Infinity para virarem rivais.",
    ),
];

/// Whether a language tag (X-Fh2-Lang, or one stored for an account) is Portuguese.
pub fn is_portuguese(lang: &str) -> bool {
    lang.trim().to_ascii_lowercase().starts_with("pt")
}

/// `text` (English) in Portuguese when `pt`, else as it is.
pub fn tr(pt: bool, text: &str) -> String {
    if !pt {
        return text.to_owned();
    }
    for (english, portuguese) in TEXTS {
        if let Some(values) = matches(english, text) {
            let mut out = String::new();
            let mut values = values.into_iter();
            let mut parts = portuguese.split("{}").peekable();
            while let Some(part) = parts.next() {
                out.push_str(part);
                if parts.peek().is_some() {
                    out.push_str(values.next().unwrap_or_default());
                }
            }
            return out;
        }
    }
    text.to_owned()
}

/// The values `text` puts where `template` has `{}`, or None when it is not that template.
fn matches<'t>(template: &str, text: &'t str) -> Option<Vec<&'t str>> {
    let parts: Vec<&str> = template.split("{}").collect();
    let mut rest = text.strip_prefix(parts[0])?;
    let mut values = Vec::new();
    for (i, part) in parts.iter().enumerate().skip(1) {
        let last = i == parts.len() - 1;
        let at = if last {
            rest.len().checked_sub(part.len()).filter(|&a| rest[a..] == **part)?
        } else {
            rest.find(part).filter(|_| !part.is_empty())?
        };
        values.push(&rest[..at]);
        rest = &rest[at + part.len()..];
    }
    (rest.is_empty() && (parts.len() > 1 || text == template)).then_some(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates() {
        assert_eq!(tr(false, "Unknown token."), "Unknown token.");
        assert_eq!(tr(true, "Unknown token."), "Token desconhecido.");
        assert_eq!(tr(true, "You and Ana#11111 are now friends and rivals!"), "Você e Ana#11111 agora são amigos e rivais!");
        assert_eq!(
            tr(true, "Request sent to Bo#22222. Once Bo#22222 adds you too, you become friends and rivals."),
            "Pedido enviado para Bo#22222. Quando Bo#22222 adicionar você também, vocês viram amigos e rivais."
        );
        assert_eq!(tr(true, "Toretto#24890 already exists on this server."), "Toretto#24890 já existe neste servidor.");
        assert_eq!(tr(true, "Something new."), "Something new.");
        assert!(is_portuguese("pt-BR") && !is_portuguese("en-US"));
    }
}
