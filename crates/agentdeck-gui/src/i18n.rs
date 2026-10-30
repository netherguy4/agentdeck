//! EN/RU strings via fluent. The language comes from the system locale;
//! tests (and only tests) override it with [`set_language`].

use std::sync::OnceLock;

use i18n_embed::LanguageLoader;
use i18n_embed::fluent::FluentLanguageLoader;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

static LOADER: OnceLock<FluentLanguageLoader> = OnceLock::new();

/// Process-wide loader, initialized once from the system locale.
pub fn loader() -> &'static FluentLanguageLoader {
    LOADER.get_or_init(|| {
        // Domain matches the file stem fl! derives from the crate name.
        let loader: FluentLanguageLoader =
            FluentLanguageLoader::new("agentdeck_gui", "en".parse().unwrap());
        let available = ["en", "ru"]
            .iter()
            .map(|tag| tag.parse().unwrap())
            .collect::<Vec<unic_langid::LanguageIdentifier>>();
        loader
            .load_languages(&Localizations, &available)
            .expect("i18n assets are embedded at compile time");
        select_system_language(&loader);
        loader
    })
}

/// Switch the active language (tests use it to render RU snapshots).
pub fn set_language(tag: &str) {
    let id: unic_langid::LanguageIdentifier = tag.parse().expect("test language tag must parse");
    i18n_embed::select(loader(), &Localizations, &[id])
        .expect("embedded languages always include the test tag");
}

/// First `LANG`/`LC_ALL`/`LANGUAGE` language goes to the loader; anything but
/// Russian falls back to English (the only two embedded languages).
fn select_system_language(loader: &FluentLanguageLoader) {
    let raw = ["LC_ALL", "LANGUAGE", "LANG"]
        .iter()
        .find_map(std::env::var_os)
        .unwrap_or_default()
        .to_string_lossy()
        .replace('_', "-");
    let tag = raw.split(['.', '@', ':']).next().unwrap_or("en");
    let short = tag.split('-').next().unwrap_or("en");
    let id: unic_langid::LanguageIdentifier = match short {
        "ru" => "ru".parse().unwrap(),
        _ => "en".parse().unwrap(),
    };
    let _ = i18n_embed::select(loader, &Localizations, &[id]);
}
