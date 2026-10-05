//! The message catalogue at run time.
//!
//! Everything the application says to a human comes from here: the window, the tray, and the
//! notes `protonvpn-core` writes about itself. The strings live in `crates/protonvpn-core/i18n/`
//! as [Fluent](https://projectfluent.org/) resources — one directory per language, English first —
//! and `build.rs` turns the English ones into the methods called below. See
//! [`docs/i18n.md`](../../../docs/i18n.md) for the rules and `scripts/translate` for filling in a
//! language.
//!
//! Three properties are worth stating plainly, because they are why this is not a `match` over a
//! constant:
//!
//! * **A missing translation does not compile.** `build.rs` compares every locale against English
//!   and refuses to generate anything if one is short a message, a file or a variable.
//! * **A call cannot forget an argument.** The generated methods take exactly the `$variables`
//!   their own pattern uses — the signature comes from the message, not from the call site.
//! * **Context travels with the string.** Fluent puts a developer comment beside each message, in
//!   the same file, and that comment is what a translator reads instead of the screen.
//!
//! What this module does *not* translate is data: a server name, a country the CLI printed, a
//! command line. Those are facts about the outside world, and they are shown as they arrived.

use std::str::FromStr;
use std::time::Duration;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

use crate::model::{ConnectTarget, ConnectionStatus, RunnerStatus};

include!(concat!(env!("OUT_DIR"), "/locales.rs"));
include!(concat!(env!("OUT_DIR"), "/messages.rs"));

/// The catalogue for one language, with every other language loaded beside it.
///
/// All of them, not just the selected one: a language picker has to name each language *in that
/// language* — "Русский" stays "Русский" whatever the window is currently in — and the fallback to
/// English has to be available on the first miss rather than on the first reload.
///
/// Loading is a few tens of kilobytes of parsing, so an instance is cheap enough to build once per
/// thread — the engine, the tray and the window each have their own. That is deliberate rather than
/// an `Arc` shared three ways: the three run independently, and a lock per sentence would buy
/// nothing that a second parse does not. The *concurrent* bundle is what makes the instance `Send`
/// at all — Fluent's default memoizes plural rules in a `RefCell`, and a catalogue that cannot be
/// handed to a thread could not be handed to the engine.
pub struct I18n {
    locale: Locale,
    bundles: Vec<FluentBundle<FluentResource>>,
}

impl I18n {
    /// Every catalogue of every locale this build carries, in `Locale::ALL` order.
    pub fn new(locale: Locale) -> Self {
        Self {
            locale,
            bundles: Locale::ALL.iter().copied().map(bundle).collect(),
        }
    }

    /// The language this instance speaks.
    pub fn locale(&self) -> Locale {
        self.locale
    }

    /// Switch language. The bundles are already loaded, so this costs nothing and takes effect on
    /// the next line drawn.
    pub fn set_locale(&mut self, locale: Locale) {
        self.locale = locale;
    }

    /// The application's own name. The same words in every language — but still a catalogue entry,
    /// because "it is never translated" is a decision, not an accident.
    pub fn app_name(&self) -> String {
        self.chrome_app_name()
    }

    /// A language's name for itself, formatted in that language rather than in the current one.
    pub fn endonym(&self, locale: Locale) -> String {
        self.lookup(&[locale], "chrome-locale-name", &[])
            .unwrap_or_else(|| locale.id().to_string())
    }

    /// The connection status in words. `Unknown` is its own answer and never collapses into
    /// "disconnected" (`docs/architecture.md` §5).
    pub fn connection_label(&self, status: &ConnectionStatus) -> String {
        match status {
            ConnectionStatus::Unknown => self.status_unknown(),
            ConnectionStatus::Disconnected => self.status_disconnected(),
            ConnectionStatus::Connecting => self.status_connecting(),
            ConnectionStatus::Connected(_) => self.status_connected(),
            ConnectionStatus::Error(_) => self.status_error(),
        }
    }

    /// The collapsed console bar: working, idle, or how many commands are behind this one.
    pub fn runner_label(&self, status: &RunnerStatus) -> String {
        match status {
            RunnerStatus::Idle => self.status_runner_idle(),
            RunnerStatus::Running { argv, .. } => {
                self.status_runner_running(crate::pty::command_line(argv))
            }
            RunnerStatus::Queued { depth } => self.status_runner_queued(*depth as i64),
        }
    }

    /// The age of a piece of state, per the wording table of `docs/architecture.md` §7.
    ///
    /// Never the word "stale": the number is the whole message, and the user judges it.
    pub fn age_text(&self, age: Duration) -> String {
        let seconds = age.as_secs();
        if seconds < 10 {
            self.status_age_just_now()
        } else if seconds < 60 {
            self.status_age_seconds(seconds as i64)
        } else if seconds < 3600 {
            self.status_age_minutes((seconds / 60) as i64)
        } else {
            self.status_age_hours((seconds / 3600) as i64)
        }
    }

    /// What a connect was asked for, for the "Connecting…" line and the console's own labels.
    /// Never claims more than the intent — a country the CLI has not answered about yet is a
    /// country, not a server.
    pub fn target_label(&self, target: &ConnectTarget) -> String {
        if let Some(server) = &target.server {
            server.clone()
        } else if let Some(city) = &target.city {
            city.clone()
        } else if let Some(country) = &target.country {
            country.clone()
        } else if target.secure_core {
            self.status_target_secure_core()
        } else if target.tor {
            self.status_target_tor()
        } else if target.p2p {
            self.status_target_p2p()
        } else if target.random {
            self.status_target_random()
        } else {
            self.status_target_fastest()
        }
    }

    /// Format in the current locale, falling back to the source language and then to the id.
    ///
    /// The fallback is unreachable: `build.rs` refuses to build a locale that is short a message.
    /// It stays because the alternative — an empty string where a sentence should be — is the one
    /// failure a user cannot even report.
    fn format<'a>(&self, id: &str, args: &[(&'static str, FluentValue<'a>)]) -> String {
        let chain: &[Locale] = if self.locale == Locale::SOURCE {
            &[self.locale]
        } else {
            &[self.locale, Locale::SOURCE]
        };
        self.lookup(chain, id, args)
            .unwrap_or_else(|| id.to_string())
    }

    fn lookup<'a>(
        &self,
        locales: &[Locale],
        id: &str,
        args: &[(&'static str, FluentValue<'a>)],
    ) -> Option<String> {
        let mut fluent = FluentArgs::new();
        for (name, value) in args {
            fluent.set(*name, value.clone());
        }

        for locale in locales {
            let bundle = &self.bundles[locale.index()];
            let Some(pattern) = bundle.get_message(id).and_then(|message| message.value()) else {
                continue;
            };
            let mut errors = Vec::new();
            let text = bundle.format_pattern(pattern, Some(&fluent), &mut errors);
            if errors.is_empty() {
                return Some(text.into_owned());
            }
        }
        None
    }
}

impl Locale {
    /// The desktop's preference, in the order gettext itself reads it.
    ///
    /// `LANGUAGE` first because it is the only one that holds a list, then the single-valued
    /// variables from most to least specific. A tag we do not carry falls back to its language
    /// (`de_AT` is German), and a language we do not carry falls back to English — `LANG=C` is not
    /// an error, it is "no preference".
    pub fn detect() -> Self {
        std::env::var("LANGUAGE")
            .ok()
            .and_then(|list| list.split(':').find_map(from_tag))
            .or_else(|| std::env::var("LC_ALL").ok().and_then(|tag| from_tag(&tag)))
            .or_else(|| {
                std::env::var("LC_MESSAGES")
                    .ok()
                    .and_then(|tag| from_tag(&tag))
            })
            .or_else(|| std::env::var("LANG").ok().and_then(|tag| from_tag(&tag)))
            .unwrap_or(Locale::SOURCE)
    }
}

/// `ru_RU.UTF-8` is Russian; `pt_BR` is `pt-br` if we carry it and Portuguese if we do not.
fn from_tag(tag: &str) -> Option<Locale> {
    let tag = tag.split(['.', '@']).next()?;
    if tag.is_empty() {
        return None;
    }
    let normalized = tag.replace('_', "-").to_ascii_lowercase();
    Locale::from_id(&normalized).or_else(|| normalized.split('-').next().and_then(Locale::from_id))
}

/// Parse one locale's resources into a bundle.
///
/// The `expect`s cannot fire: `build.rs` parsed these exact bytes successfully a moment before the
/// compiler saw them, and a resource that is broken here would be broken there first.
fn bundle(locale: Locale) -> FluentBundle<FluentResource> {
    let id = LanguageIdentifier::from_str(locale.id())
        .unwrap_or_else(|error| panic!("i18n: `{}` is not a language tag: {error}", locale.id()));
    let mut bundle = FluentBundle::new_concurrent(vec![id]);

    // Fluent wraps every placeable in Unicode bidi isolates by default, which is right for
    // mixed-direction text in a browser and wrong in a GUI: the marks end up in the label, in the
    // clipboard, and in the width the layout measured.
    bundle.set_use_isolating(false);

    // `NUMBER()` is not installed by default, and without it every plural selection that reads a
    // number formats as an error — which is exactly how a language silently stops declining its
    // nouns.
    bundle
        .add_function("NUMBER", fluent_bundle::builtins::NUMBER)
        .expect("NUMBER is not already defined");

    for (resource, text) in resources(locale) {
        let parsed = FluentResource::try_new(text.to_string()).unwrap_or_else(|(_, errors)| {
            panic!("i18n/{}/{resource}.ftl: {errors:?}", locale.id())
        });
        bundle
            .add_resource(parsed)
            .unwrap_or_else(|errors| panic!("i18n/{}/{resource}.ftl: {errors:?}", locale.id()));
    }
    bundle
}

#[cfg(test)]
mod tests {
    use super::*;

    fn english() -> I18n {
        I18n::new(Locale::SOURCE)
    }

    #[test]
    fn every_locale_carries_every_message() {
        for locale in Locale::ALL {
            let i18n = I18n::new(*locale);
            let bundle = &i18n.bundles[locale.index()];
            for id in MessageIds::ALL {
                assert!(
                    bundle.get_message(id).is_some(),
                    "i18n/{}: `{id}` is missing",
                    locale.id()
                );
            }
        }
    }

    #[test]
    fn a_missing_argument_is_not_a_crash() {
        // Unreachable through the generated accessors, which take exactly the message's own
        // variables — but `lookup` is also how `endonym` reaches a message, and a formatter that
        // panics on a mistake would take the window down with it.
        assert!(
            english()
                .lookup(&[Locale::SOURCE], "status-age-hours", &[])
                .is_none()
        );
    }

    #[test]
    fn a_language_names_itself_in_itself() {
        let i18n = english();
        assert_eq!(i18n.endonym(Locale::from_id("ru").unwrap()), "Русский");
        assert_eq!(i18n.endonym(Locale::SOURCE), "English");
        // Not in the current language: the picker has to be readable by someone who cannot read
        // the language they are currently stuck in.
        let russian = I18n::new(Locale::from_id("ru").unwrap());
        assert_eq!(russian.endonym(Locale::SOURCE), "English");
    }

    #[test]
    fn age_wording_matches_the_table() {
        let i18n = english();
        assert_eq!(i18n.age_text(Duration::from_secs(0)), "updated just now");
        assert_eq!(i18n.age_text(Duration::from_secs(9)), "updated just now");
        assert_eq!(i18n.age_text(Duration::from_secs(10)), "updated 10s ago");
        assert_eq!(i18n.age_text(Duration::from_secs(59)), "updated 59s ago");
        assert_eq!(i18n.age_text(Duration::from_secs(60)), "updated 1 min ago");
        assert_eq!(
            i18n.age_text(Duration::from_secs(180)),
            "updated 3 mins ago"
        );
        assert_eq!(
            i18n.age_text(Duration::from_secs(3600)),
            "updated 1 hour ago"
        );
        assert_eq!(
            i18n.age_text(Duration::from_secs(7200)),
            "updated 2 hours ago"
        );
    }

    #[test]
    fn age_never_says_stale() {
        for secs in [0, 5, 61, 7200, 86_400] {
            for locale in Locale::ALL {
                let text = I18n::new(*locale).age_text(Duration::from_secs(secs));
                assert!(!text.contains("stale"), "{text}");
                assert!(!text.contains("устар"), "{text}");
            }
        }
    }

    #[test]
    fn russian_counts_in_three_forms() {
        let i18n = I18n::new(Locale::from_id("ru").unwrap());
        assert_eq!(
            i18n.age_text(Duration::from_secs(60)),
            "обновлено 1 минуту назад"
        );
        assert_eq!(
            i18n.age_text(Duration::from_secs(180)),
            "обновлено 3 минуты назад"
        );
        assert_eq!(
            i18n.age_text(Duration::from_secs(300)),
            "обновлено 5 минут назад"
        );
    }

    #[test]
    fn a_status_is_never_invented_in_any_language() {
        for locale in Locale::ALL {
            let i18n = I18n::new(*locale);
            let unknown = i18n.connection_label(&ConnectionStatus::Unknown);
            let disconnected = i18n.connection_label(&ConnectionStatus::Disconnected);
            assert_ne!(unknown, disconnected, "{}", locale.id());
        }
    }

    #[test]
    fn a_locale_tag_falls_back_to_its_language() {
        assert_eq!(from_tag("ru_RU.UTF-8"), Locale::from_id("ru"));
        assert_eq!(from_tag("en_GB"), Locale::from_id("en"));
        assert_eq!(from_tag("C"), None);
        assert_eq!(from_tag("de_DE"), None);
        assert_eq!(from_tag(""), None);
    }
}
