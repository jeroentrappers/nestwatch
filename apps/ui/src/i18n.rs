// SPDX-License-Identifier: AGPL-3.0-or-later
//! Runtime translations of the Dioxus apps (product.md NFR-I18N-1/2).
//!
//! - English is the reference and lives in code: every string is declared
//!   once in [`crate::strings`] with its English text, which is also the
//!   fallback.
//! - Each other language has one file `apps/ui/i18n/<code>.txt`, embedded
//!   at build time, with lines `key = text` (keys are the function names
//!   in [`crate::strings`]). Plural strings have one line per plural
//!   category of that language: `key[one] = …`, `key[few] = …`,
//!   `key[many] = …`, `key[other] = …` (see [`Lang::plural_categories`]).
//!   Placeholders are named, `{name}`; `\n` is a line break, `\\` a
//!   backslash; lines starting with `#` are comments.
//! - The language follows the system ([`Lang::from_tags`] over the
//!   preferred languages), English when none is supported. The host calls
//!   [`set_language`] once at start; tests use [`with_language`], which
//!   only affects the calling thread.
//! - Local time: the host installs the UTC offset function with
//!   [`set_utc_offset`]; without one, times are shown in UTC.

use std::cell::Cell;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// A supported language (NFR-I18N-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    /// English (reference).
    En,
    /// Dutch.
    Nl,
    /// French.
    Fr,
    /// German.
    De,
    /// Spanish.
    Es,
    /// Italian.
    It,
    /// Portuguese (Brazil).
    PtBr,
    /// Polish.
    Pl,
    /// Swedish.
    Sv,
    /// Danish.
    Da,
    /// Norwegian Bokmål.
    Nb,
}

/// A CLDR plural category (cardinal, whole numbers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Plural {
    /// `one`.
    One,
    /// `few`.
    Few,
    /// `many`.
    Many,
    /// `other`.
    Other,
}

impl Plural {
    /// The category name used in translation files.
    pub fn name(self) -> &'static str {
        match self {
            Self::One => "one",
            Self::Few => "few",
            Self::Many => "many",
            Self::Other => "other",
        }
    }

    /// Parses a category name.
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "one" => Self::One,
            "few" => Self::Few,
            "many" => Self::Many,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

impl Lang {
    /// All languages, English first.
    pub const ALL: [Lang; 11] = [
        Lang::En,
        Lang::Nl,
        Lang::Fr,
        Lang::De,
        Lang::Es,
        Lang::It,
        Lang::PtBr,
        Lang::Pl,
        Lang::Sv,
        Lang::Da,
        Lang::Nb,
    ];

    /// BCP 47 tag (`pt-BR`).
    pub fn tag(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Nl => "nl",
            Self::Fr => "fr",
            Self::De => "de",
            Self::Es => "es",
            Self::It => "it",
            Self::PtBr => "pt-BR",
            Self::Pl => "pl",
            Self::Sv => "sv",
            Self::Da => "da",
            Self::Nb => "nb",
        }
    }

    /// The embedded translation file; `None` for English (in code).
    pub fn source(self) -> Option<&'static str> {
        Some(match self {
            Self::En => return None,
            Self::Nl => include_str!("../i18n/nl.txt"),
            Self::Fr => include_str!("../i18n/fr.txt"),
            Self::De => include_str!("../i18n/de.txt"),
            Self::Es => include_str!("../i18n/es.txt"),
            Self::It => include_str!("../i18n/it.txt"),
            Self::PtBr => include_str!("../i18n/pt-BR.txt"),
            Self::Pl => include_str!("../i18n/pl.txt"),
            Self::Sv => include_str!("../i18n/sv.txt"),
            Self::Da => include_str!("../i18n/da.txt"),
            Self::Nb => include_str!("../i18n/nb.txt"),
        })
    }

    /// The language of one BCP 47 tag (`de-AT`, `pt_BR.UTF-8`, `no`),
    /// `None` when unsupported. Any Portuguese maps to pt-BR and Norwegian
    /// (`no`, `nn`) to Bokmål.
    pub fn from_tag(tag: &str) -> Option<Lang> {
        let tag = tag.trim().split(['.', '@']).next().unwrap_or("");
        let primary = tag
            .split(['-', '_'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        Some(match primary.as_str() {
            "en" => Self::En,
            "nl" => Self::Nl,
            "fr" => Self::Fr,
            "de" => Self::De,
            "es" => Self::Es,
            "it" => Self::It,
            "pt" => Self::PtBr,
            "pl" => Self::Pl,
            "sv" => Self::Sv,
            "da" => Self::Da,
            "nb" | "no" | "nn" => Self::Nb,
            _ => return None,
        })
    }

    /// The first supported language of a preference list, else English.
    pub fn from_tags<I, S>(tags: I) -> Lang
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        tags.into_iter()
            .find_map(|t| Self::from_tag(t.as_ref()))
            .unwrap_or(Self::En)
    }

    /// The plural category of the whole number `n` (CLDR cardinal rules).
    pub fn plural(self, n: u64) -> Plural {
        match self {
            // French and Portuguese: 0 and 1 are singular.
            Self::Fr | Self::PtBr => {
                if n <= 1 {
                    Plural::One
                } else {
                    Plural::Other
                }
            }
            Self::Pl => {
                let (m10, m100) = (n % 10, n % 100);
                if n == 1 {
                    Plural::One
                } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                    Plural::Few
                } else {
                    Plural::Many
                }
            }
            _ => {
                if n == 1 {
                    Plural::One
                } else {
                    Plural::Other
                }
            }
        }
    }

    /// The categories a translation file must give for each plural string.
    pub fn plural_categories(self) -> &'static [Plural] {
        match self {
            Self::Pl => &[Plural::One, Plural::Few, Plural::Many],
            _ => &[Plural::One, Plural::Other],
        }
    }

    fn index(self) -> u8 {
        Self::ALL.iter().position(|l| *l == self).unwrap_or(0) as u8
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

thread_local! {
    static OVERRIDE: Cell<Option<Lang>> = const { Cell::new(None) };
}

/// Sets the app's language (once, at start, from the system).
pub fn set_language(lang: Lang) {
    CURRENT.store(lang.index(), Ordering::Relaxed);
}

/// The language in effect on this thread.
pub fn language() -> Lang {
    OVERRIDE
        .with(Cell::get)
        .unwrap_or_else(|| Lang::ALL[usize::from(CURRENT.load(Ordering::Relaxed)) % 11])
}

/// Runs `f` with `lang` in effect on this thread only (tests, previews).
pub fn with_language<R>(lang: Lang, f: impl FnOnce() -> R) -> R {
    let before = OVERRIDE.with(|o| o.replace(Some(lang)));
    struct Restore(Option<Lang>);
    impl Drop for Restore {
        fn drop(&mut self) {
            OVERRIDE.with(|o| o.set(self.0));
        }
    }
    let _restore = Restore(before);
    f()
}

/// One string of [`crate::strings`] with its English text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Key (the function name).
    pub key: &'static str,
    /// What it is for (the doc comment).
    pub doc: &'static str,
    /// Plural string.
    pub plural: bool,
    /// English text: `("", text)`, or `(category, text)` for plurals.
    pub forms: Vec<(&'static str, &'static str)>,
}

/// A parsed translation file.
#[derive(Debug, Default)]
pub struct Table {
    /// Plain and formatted strings.
    pub texts: HashMap<String, String>,
    /// Plural strings by category.
    pub plurals: HashMap<String, HashMap<Plural, String>>,
}

/// A problem in a translation file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based line.
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

fn unescape(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('\\') => out.push('\\'),
                Some(o) => {
                    out.push('\\');
                    out.push(o);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Parses a translation file; duplicate keys and malformed lines are errors.
pub fn parse(source: &str) -> Result<Table, Vec<ParseError>> {
    let mut t = Table::default();
    let mut errors = Vec::new();
    for (i, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let err = |m: String| ParseError {
            line: i + 1,
            message: m,
        };
        let Some((k, v)) = line.split_once('=') else {
            errors.push(err(format!("no `=` in {line:?}")));
            continue;
        };
        let (k, v) = (k.trim(), unescape(v.trim()));
        if let Some((key, cat)) = k.strip_suffix(']').and_then(|k| k.split_once('[')) {
            let Some(cat) = Plural::from_name(cat) else {
                errors.push(err(format!("unknown plural category in {k:?}")));
                continue;
            };
            if t.plurals
                .entry(key.to_owned())
                .or_default()
                .insert(cat, v)
                .is_some()
            {
                errors.push(err(format!("duplicate {k:?}")));
            }
        } else if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            errors.push(err(format!("bad key {k:?}")));
        } else if t.texts.insert(k.to_owned(), v).is_some() {
            errors.push(err(format!("duplicate {k:?}")));
        }
    }
    if errors.is_empty() {
        Ok(t)
    } else {
        Err(errors)
    }
}

/// The parsed table of `lang` (`None` for English or a broken file; the
/// unit tests keep the files valid).
pub fn table(lang: Lang) -> Option<&'static Table> {
    static TABLES: [OnceLock<Option<Table>>; 11] = [const { OnceLock::new() }; 11];
    TABLES[usize::from(lang.index())]
        .get_or_init(|| lang.source().and_then(|s| parse(s).ok()))
        .as_ref()
}

/// The names of the `{placeholders}` in a pattern, in order.
pub fn placeholders(pattern: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = pattern;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { break };
        let name = &after[..end];
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            out.push(name);
        }
        rest = &after[end + 1..];
    }
    out
}

/// Replaces `{name}` placeholders.
pub fn fill(pattern: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut rest = pattern;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match args.iter().find(|(n, _)| *n == name) {
                    Some((_, v)) => out.push_str(&v.to_string()),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A plain string in the current language.
pub fn text(key: &'static str, english: &'static str) -> &'static str {
    match table(language()) {
        Some(t) => t.texts.get(key).map_or(english, String::as_str),
        None => english,
    }
}

/// A string with placeholders in the current language.
pub fn format(key: &'static str, english: &'static str, args: &[(&str, &dyn Display)]) -> String {
    fill(text(key, english), args)
}

/// A plural string for `n` in the current language. `english` holds the
/// English `one` and `other` forms.
pub fn plural(
    key: &'static str,
    n: u64,
    english: &[(&'static str, &'static str)],
    args: &[(&str, &dyn Display)],
) -> String {
    let lang = language();
    let translated = table(lang)
        .and_then(|t| t.plurals.get(key))
        .and_then(|forms| {
            let cat = lang.plural(n);
            forms
                .get(&cat)
                .or_else(|| forms.get(&Plural::Other))
                .map(String::as_str)
        });
    let pattern = translated.unwrap_or_else(|| {
        let cat = Lang::En.plural(n).name();
        english
            .iter()
            .find(|(c, _)| *c == cat)
            .or_else(|| english.iter().find(|(c, _)| *c == "other"))
            .map_or("", |(_, p)| *p)
    });
    fill(pattern, args)
}

/// Local UTC offset in minutes at a Unix time in ms.
pub type UtcOffsetFn = fn(u64) -> i32;

static UTC_OFFSET: RwLock<Option<UtcOffsetFn>> = RwLock::new(None);

/// Installs the function giving the local UTC offset in minutes at a Unix
/// time in ms (the host's time zone).
pub fn set_utc_offset(f: UtcOffsetFn) {
    if let Ok(mut g) = UTC_OFFSET.write() {
        *g = Some(f);
    }
}

/// The local UTC offset in minutes at `unix_ms` (0 without a host function).
pub fn utc_offset_minutes(unix_ms: u64) -> i32 {
    UTC_OFFSET
        .read()
        .ok()
        .and_then(|g| *g)
        .map_or(0, |f| f(unix_ms))
}

/// The English strings in translation file format, with each string's
/// doc comment above it: the starting point for a new language
/// (`NESTWATCH_I18N_TEMPLATE=path cargo test -p nestwatch-ui -- --ignored
/// write_english_template`).
pub fn english_template() -> String {
    let mut out = String::from(
        "# Nestwatch strings. `key = text`; plural strings have one line per\n\
         # category of the language (`key[one]`, `key[other]`, Polish also\n\
         # `key[few]`, `key[many]` instead of `key[other]`). Keep {placeholders}.\n",
    );
    for r in crate::strings::reference() {
        out.push('\n');
        for line in r.doc.lines() {
            out.push_str(&format!("# {}\n", line.trim()));
        }
        for (cat, text) in &r.forms {
            let text = text.replace('\\', "\\\\").replace('\n', "\\n");
            if r.plural {
                out.push_str(&format!("{}[{cat}] = {text}\n", r.key));
            } else {
                out.push_str(&format!("{} = {text}\n", r.key));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn the_english_template_parses_with_every_key() {
        let t = parse(&english_template()).unwrap();
        let reference = crate::strings::reference();
        assert_eq!(t.texts.len() + t.plurals.len(), reference.len());
        assert_eq!(t.texts["close"], "Close");
        assert_eq!(t.plurals["wake_ups"][&Plural::Other], "{n} wake-ups");
    }

    #[test]
    #[ignore = "writes a file; run on demand"]
    fn write_english_template() {
        if let Some(p) = std::env::var_os("NESTWATCH_I18N_TEMPLATE") {
            std::fs::write(p, english_template()).unwrap();
        }
    }

    #[test]
    fn tags_pick_the_first_supported_language() {
        assert_eq!(Lang::from_tags(["xx", "de-AT", "fr"]), Lang::De);
        assert_eq!(Lang::from_tags(["pt-PT"]), Lang::PtBr);
        assert_eq!(Lang::from_tags(["nb_NO.UTF-8"]), Lang::Nb);
        assert_eq!(Lang::from_tags(["no"]), Lang::Nb);
        assert_eq!(Lang::from_tags(["ja", "zh"]), Lang::En);
        assert_eq!(Lang::from_tags(Vec::<String>::new()), Lang::En);
        assert_eq!(Lang::from_tag("C"), None);
    }

    #[test]
    fn plural_rules() {
        let cats = |l: Lang| [0, 1, 2, 5, 12, 22, 25, 101].map(|n| l.plural(n));
        use Plural::*;
        assert_eq!(
            cats(Lang::En),
            [Other, One, Other, Other, Other, Other, Other, Other]
        );
        assert_eq!(
            cats(Lang::Fr),
            [One, One, Other, Other, Other, Other, Other, Other]
        );
        assert_eq!(
            cats(Lang::Pl),
            [Many, One, Few, Many, Many, Few, Many, Many]
        );
        for l in Lang::ALL {
            for n in 0..200 {
                assert!(l.plural_categories().contains(&l.plural(n)), "{l:?} {n}");
            }
        }
    }

    #[test]
    fn parse_fill_and_errors() {
        let t = parse(
            "# c\n\na = Hello {name}\nb[one] = {n} thing\nb[other] = {n} things\nc = x\\ny\n",
        )
        .unwrap();
        assert_eq!(t.texts["a"], "Hello {name}");
        assert_eq!(t.texts["c"], "x\ny");
        assert_eq!(t.plurals["b"][&Plural::One], "{n} thing");
        assert_eq!(fill("Hi {name}, {x}", &[("name", &"Ann")]), "Hi Ann, {x}");
        assert_eq!(placeholders("{a} and {b_2} {"), vec!["a", "b_2"]);
        let e = parse("a = 1\na = 2\nnothing\nb[two] = x\n").unwrap_err();
        assert_eq!(e.len(), 3);
    }

    /// Every language has every string, with the same placeholders, and
    /// nothing else (NFR-I18N-2).
    #[test]
    fn every_language_is_complete_and_consistent() {
        let reference = crate::strings::reference();
        let mut problems = Vec::new();
        for lang in Lang::ALL.into_iter().skip(1) {
            let src = lang.source().unwrap_or_default();
            let t = match parse(src) {
                Ok(t) => t,
                Err(e) => {
                    problems.push(format!("{}: {e:?}", lang.tag()));
                    continue;
                }
            };
            let tag = lang.tag();
            for r in &reference {
                let english: BTreeSet<&str> =
                    r.forms.iter().flat_map(|(_, p)| placeholders(p)).collect();
                if !r.plural {
                    match t.texts.get(r.key) {
                        None => problems.push(format!("{tag}: missing {}", r.key)),
                        Some(v) => {
                            let got: BTreeSet<&str> = placeholders(v).into_iter().collect();
                            if got != english {
                                problems.push(format!(
                                    "{tag}: {} has {got:?}, English {english:?}",
                                    r.key
                                ));
                            }
                            if v.trim().is_empty() {
                                problems.push(format!("{tag}: {} is empty", r.key));
                            }
                        }
                    }
                    if t.plurals.contains_key(r.key) {
                        problems.push(format!("{tag}: {} is not a plural", r.key));
                    }
                    continue;
                }
                let Some(forms) = t.plurals.get(r.key) else {
                    problems.push(format!("{tag}: missing plural {}", r.key));
                    continue;
                };
                let want: BTreeSet<&str> =
                    lang.plural_categories().iter().map(|c| c.name()).collect();
                let have: BTreeSet<&str> = forms.keys().map(|c| c.name()).collect();
                if want != have {
                    problems.push(format!(
                        "{tag}: {} has forms {have:?}, needs {want:?}",
                        r.key
                    ));
                }
                let mut union = BTreeSet::new();
                for (cat, v) in forms {
                    let got: BTreeSet<&str> = placeholders(v).into_iter().collect();
                    if !got.is_subset(&english) {
                        problems.push(format!("{tag}: {}[{}] has {got:?}", r.key, cat.name()));
                    }
                    union.extend(got);
                }
                if union != english {
                    problems.push(format!(
                        "{tag}: {} uses {union:?}, English {english:?}",
                        r.key
                    ));
                }
                if t.texts.contains_key(r.key) {
                    problems.push(format!("{tag}: {} needs plural forms", r.key));
                }
            }
            let known: BTreeSet<&str> = reference.iter().map(|r| r.key).collect();
            for k in t.texts.keys().chain(t.plurals.keys()) {
                if !known.contains(k.as_str()) {
                    problems.push(format!("{tag}: unknown key {k}"));
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn with_language_is_scoped_to_the_thread() {
        let fr = with_language(Lang::Fr, language);
        assert_eq!(fr, Lang::Fr);
        assert_eq!(language(), Lang::En);
    }
}
