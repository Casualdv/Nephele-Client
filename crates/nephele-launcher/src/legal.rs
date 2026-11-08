//! The text of the in-app legal screens, and the source for the README's
//! legal section. Keeping it in code means the launcher can show it with no
//! network, no prepared resources, and no extra file to lose.

pub const TITLE: &str = "LEGAL";

pub const NON_AFFILIATION: &[&str] = &[
    "Nephele is not an official Minecraft product. It is not approved by, \
     endorsed by, or associated with Mojang Synergies AB or Microsoft \
     Corporation.",
    "\"Minecraft\" is a trademark of Mojang Synergies AB. It is used here only \
     to describe what this client is compatible with.",
    "Nephele ships no Minecraft code, textures, sounds, data or game files. \
     Every resource is built on your machine from the copy of Minecraft: Java \
     Edition 1.8.9 that you already own.",
];

pub const TERMS: &[&str] = &[
    "Nephele's own source code is licensed under MIT OR Apache-2.0. The licence \
     text is in the repository.",
    "You must own a legitimate copy of Minecraft: Java Edition 1.8.9 to use \
     this client. Nephele never downloads the game for you.",
    "The software is provided \"as is\", without warranty of any kind. You are \
     responsible for following the rules of any server you join and for any \
     account action that results from how you use it.",
    "Nephele does not bypass account authentication, server anti-cheat or \
     Mojang's terms. Offline-mode play is for local testing and private \
     servers you are allowed to use.",
];

pub const PRIVACY: &[&str] = &[
    "No telemetry. Nothing about your play is collected or sent anywhere by \
     default.",
    "Your Microsoft session token and server list are stored only on this \
     machine, under your user data folder. They are never uploaded.",
    "A crash report is written to a local file. It is uploaded only if you \
     explicitly turn crash uploads on in the launcher, and only when the build \
     is configured with an upload endpoint.",
    "An update check is made only by builds that carry an update signing key. \
     It sends no account information and downloads nothing.",
    "Joining an online server sends the normal Minecraft login and play \
     packets that any 1.8.9 client sends. That traffic goes to the server you \
     chose, not to Nephele.",
];

pub const BLOCKED_SIGNING: &str = "This build is unsigned, so automatic updates are disabled. \
     The update signing key in the release pipeline is empty until a code-signing \
     certificate exists.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_legal_text_covers_the_required_ground() {
        let non_affiliation = NON_AFFILIATION.join(" ");
        assert!(non_affiliation.contains("not an official Minecraft product"));
        assert!(non_affiliation.contains("Mojang"));
        assert!(non_affiliation.contains("Microsoft"));

        let terms = TERMS.join(" ");
        assert!(terms.contains("MIT OR Apache-2.0"));
        assert!(terms.contains("as is"));

        let privacy = PRIVACY.join(" ");
        assert!(privacy.contains("No telemetry"));
        assert!(privacy.contains("only if you explicitly turn crash uploads on"));
        assert!(privacy.contains("never uploaded"));
    }

    #[test]
    fn the_signing_notice_is_explicit_about_the_blocker() {
        assert!(BLOCKED_SIGNING.contains("unsigned"));
        assert!(BLOCKED_SIGNING.contains("code-signing"));
    }
}
