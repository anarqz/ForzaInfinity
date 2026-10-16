//! The account endpoints' messages in the player's language: Portuguese (Brazil) when the hook
//! says the game runs in it (X-Fh2-Lang: pt-BR), English otherwise. Messages are written in
//! English as before and translated last: `tr` finds the English template (`{}` where a name
//! or a number goes) and fills the Portuguese one with the same values, in order.

/// (English, Portuguese); `{}` marks the same values in both.
const TEXTS: &[(&str, &str)] = &[
    ("Account suspended on this server.", "Conta suspensa neste servidor."),
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
