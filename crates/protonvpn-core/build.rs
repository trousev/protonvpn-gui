//! The catalogue compiler: the translation lint, and the Rust the crate actually calls.
//!
//! Every locale under `i18n/` is read **before this crate is compiled**. English is the source of
//! truth: the messages it declares are the messages every other locale owes, in the same resources,
//! with the same `$variables`, and each with a developer comment beside it saying where it appears.
//! A locale that is missing one does not compile. That is deliberate: a test can be skipped, an
//! `#[ignore]`d test is skipped by default, and a silent fall back to English at run time is
//! exactly the failure that goes unnoticed for a year — a build script is the one place where
//! "incomplete translation" and "does not build" are the same statement.
//!
//! Two files are written into `OUT_DIR`:
//!
//! * `locales.rs` — the [`Locale`] enum, one variant per directory under `i18n/`, and the
//!   `include_str!` table that puts the catalogues into the binary,
//! * `messages.rs` — one method per message id, taking the arguments that message's pattern
//!   actually uses. A call that forgets an argument is a compile error, not a literal `{$name}`
//!   left on somebody's screen.
//!
//! `scripts/translate` fills in what this rejects.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::{env, fs, process};

use fluent_syntax::ast;
use fluent_syntax::parser;

/// The language the messages are written in. Not one locale among others: the catalogue every other
/// locale is measured against, and what the application falls back to if a bundle ever fails.
const SOURCE: &str = "en";

/// Names [`I18n`] uses for its own helpers. A generated method with one of these would be a
/// duplicate definition — and, worse, an ambiguity a reader would have to resolve by reading the
/// build script. The message id is what has to change.
const RESERVED: &[&str] = &[
    "new",
    "locale",
    "set_locale",
    "bundle",
    "format",
    "lookup",
    "endonym",
    "detect",
    "app_name",
    "connection_label",
    "runner_label",
    "target_label",
    "age_text",
];

/// The CLDR plural categories. A `select` whose every variant key is one of these selects a
/// **number**, and Russian is why it matters: it needs `one`, `few` and `many` where English needs
/// two forms, and `[few]` only ever matches an actual number, never the string "3".
const PLURAL_CATEGORIES: &[&str] = &["zero", "one", "two", "few", "many", "other"];

/// The only function a message may call. `NUMBER` is how a number reaches a plural selection and
/// how it is written in the locale's own digits; anything else would be a formatter this crate has
/// not installed, which fails at run time rather than here.
const FUNCTIONS: &[&str] = &["NUMBER"];

const KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use",
    "where", "while", "async", "await", "box",
];

/// One message id, and everything the checks and the code generator need to know about it.
type Catalogue = BTreeMap<String, Message>;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let catalogues = manifest.join("i18n");
    println!("cargo:rerun-if-changed={}", catalogues.display());

    // The "is this message used anywhere" check reads the whole workspace, so a rename that
    // orphans a message re-runs this script.
    let mut sources = Vec::new();
    if let Some(workspace) = manifest.parent().and_then(Path::parent) {
        for entry in fs::read_dir(workspace.join("crates")).into_iter().flatten() {
            let root = entry.expect("workspace entry").path().join("src");
            if root.is_dir() {
                println!("cargo:rerun-if-changed={}", root.display());
                collect_sources(&root, &mut sources);
            }
        }
    }

    let errors = build(&catalogues, &sources);
    if !errors.is_empty() {
        eprintln!();
        for error in &errors {
            eprintln!("error: {error}");
        }
        eprintln!("\n{} problem(s) in i18n/.", errors.len());
        eprintln!("Run `scripts/translate` to fill the gaps, or fix them where they are named.");
        process::exit(1);
    }
}

fn collect_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten() {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            collect_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// One message, as its locale declares it.
#[derive(Debug)]
struct Message {
    /// The resource stem it lives in — `chrome`, `status`, … — which is also the id's prefix.
    resource: String,
    /// The developer comment, one string per line. This is the context a translator reads.
    comment: Vec<String>,
    /// The pattern, rendered back to FTL-ish text for the generated documentation.
    text: String,
    /// The `$variables` the pattern uses, in the order they first appear.
    variables: Vec<Variable>,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Kind {
    Text,
    /// Used as `NUMBER($n)`, or selected over plural categories. An `i64` at the call site, so that
    /// a language can pick `[few]` for 3 and `[many]` for 5.
    Number,
}

#[derive(Debug)]
struct Variable {
    name: String,
    kind: Kind,
}

fn build(catalogues: &Path, sources: &[PathBuf]) -> Vec<String> {
    let mut errors = Vec::new();

    let locales = discover(catalogues);
    if !locales.iter().any(|locale| locale == SOURCE) {
        errors.push(format!(
            "`i18n/{SOURCE}` is missing: it is the source language"
        ));
        return errors;
    }

    // English first, in the sense that everything else is measured against it.
    let mut all: BTreeMap<String, Catalogue> = BTreeMap::new();
    for locale in &locales {
        if let Some(catalogue) = read_locale(&catalogues.join(locale), locale, &mut errors) {
            all.insert(locale.clone(), catalogue);
        }
    }
    let Some(source) = all.get(SOURCE) else {
        return errors;
    };

    // Checks only the source language can carry: an id becomes a method name, and its prefix is the
    // file it lives in, so that "where is this string" has exactly one answer.
    for (id, message) in source {
        if message.comment.is_empty() {
            errors.push(format!(
                "i18n/{SOURCE}: `{id}` has no comment. Every message needs a line saying where it \
                 appears and how much room it has — it is what a translator has instead of the \
                 screen."
            ));
        }
        let method = method_name(id);
        if RESERVED.contains(&method.as_str()) {
            errors.push(format!(
                "i18n/{SOURCE}: `{id}` would generate `I18n::{method}`, which already exists. \
                 Rename the message."
            ));
        }
        if KEYWORDS.contains(&method.as_str()) {
            errors.push(format!(
                "i18n/{SOURCE}: `{id}` becomes the Rust keyword `{method}`"
            ));
        }
    }

    // Every other locale owes English an exact counterpart, file for file, id for id.
    for locale in locales.iter().filter(|locale| *locale != SOURCE) {
        let Some(mine) = all.get(locale) else {
            continue;
        };

        for missing in resources_of(source).difference(&resources_of(mine)) {
            errors.push(format!(
                "i18n/{locale}: `{missing}.ftl` is missing. Every locale carries the same files, so \
                 that a gap is a missing file and not a message nobody noticed."
            ));
        }
        for extra in resources_of(mine).difference(&resources_of(source)) {
            errors.push(format!(
                "i18n/{locale}: `{extra}.ftl` has no `i18n/{SOURCE}` counterpart"
            ));
        }

        for (id, message) in source {
            let Some(translated) = mine.get(id) else {
                errors.push(format!("i18n/{locale}: `{id}` is untranslated"));
                continue;
            };
            if translated.comment.is_empty() {
                errors.push(format!(
                    "i18n/{locale}: `{id}` has no comment. The comment travels with the \
                     translation, so a later edit still knows where the string lives."
                ));
            }

            let wanted: BTreeSet<&str> = message
                .variables
                .iter()
                .map(|variable| variable.name.as_str())
                .collect();
            let got: BTreeSet<&str> = translated
                .variables
                .iter()
                .map(|variable| variable.name.as_str())
                .collect();
            if wanted != got {
                let missing: Vec<_> = wanted.difference(&got).collect();
                let extra: Vec<_> = got.difference(&wanted).collect();
                errors.push(format!(
                    "i18n/{locale}: `{id}` uses the wrong variables — missing {missing:?}, unknown \
                     {extra:?}. A translation substitutes the same names as its source."
                ));
            }
            for (want, have) in message.variables.iter().zip(&translated.variables) {
                if want.name == have.name && want.kind != have.kind {
                    errors.push(format!(
                        "i18n/{locale}: `{id}` uses `${}` as {:?} where {SOURCE} uses {:?} — a \
                         number and a string are selected over differently",
                        want.name, have.kind, want.kind
                    ));
                }
            }
        }

        for id in mine.keys() {
            if !source.contains_key(id) {
                errors.push(format!(
                    "i18n/{locale}: `{id}` does not exist in i18n/{SOURCE} — a translation cannot \
                     invent a message"
                ));
            }
        }
    }

    // A message nobody asks for is a message that will be translated forever for nothing.
    let haystack = sources
        .iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .collect::<Vec<_>>()
        .join("\n");
    for id in source.keys() {
        let method = method_name(id);
        // Either the generated accessor, or the id written out as a literal: `endonym` formats
        // through `I18n::lookup` rather than through an accessor, because it needs a different
        // locale's bundle than the one its receiver holds.
        if !haystack.contains(&method) && !haystack.contains(&format!("\"{id}\"")) {
            errors.push(format!(
                "i18n/{SOURCE}: `{id}` is used nowhere in the workspace. Delete it, or call \
                 `i18n.{method}()`."
            ));
        }
    }

    if !errors.is_empty() {
        return errors;
    }

    if let Err(error) = generate(&locales, source, &all) {
        errors.push(error);
    }
    errors
}

fn discover(catalogues: &Path) -> Vec<String> {
    let mut locales: Vec<String> = fs::read_dir(catalogues)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", catalogues.display()))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            path.is_dir()
                .then(|| path.file_name()?.to_str().map(str::to_string))?
        })
        .collect();
    locales.sort();
    locales
}

/// The set of `.ftl` stems a catalogue is built from.
fn resources_of(catalogue: &Catalogue) -> BTreeSet<String> {
    catalogue
        .values()
        .map(|message| message.resource.clone())
        .collect()
}

/// The method a message id turns into when it reaches Rust.
fn method_name(id: &str) -> String {
    id.replace('-', "_")
}

fn read_locale(dir: &Path, locale: &str, errors: &mut Vec<String>) -> Option<Catalogue> {
    let mut out = Catalogue::new();

    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            path.extension()
                .is_some_and(|ext| ext == "ftl")
                .then_some(path)
        })
        .collect();
    files.sort();

    if files.is_empty() {
        errors.push(format!("i18n/{locale}: no `.ftl` files at all"));
        return None;
    }

    for path in files {
        let resource = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("an .ftl file name")
            .to_string();
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        let parsed = match parser::parse(text.as_str()) {
            Ok(resource) => resource,
            Err((_, parse_errors)) => {
                for error in parse_errors {
                    let snippet = error
                        .slice
                        .as_ref()
                        .and_then(|range| text.get(range.clone()))
                        .unwrap_or("")
                        .trim();
                    errors.push(format!(
                        "i18n/{locale}/{resource}.ftl: syntax error at byte {}: {snippet:?}",
                        error.pos.start
                    ));
                }
                continue;
            }
        };

        for entry in parsed.body {
            let message = match entry {
                ast::Entry::Message(message) => message,
                ast::Entry::Junk { content } => {
                    errors.push(format!(
                        "i18n/{locale}/{resource}.ftl: unparsable entry {content:?}"
                    ));
                    continue;
                }
                ast::Entry::Term(term) => {
                    errors.push(format!(
                        "i18n/{locale}/{resource}.ftl: `-{}-` is a term, and terms are not part of \
                         this catalogue — they exist to be shared between messages, and every \
                         message here has to stand alone.",
                        term.id.name
                    ));
                    continue;
                }
                // Section headers. Free documentation, not a message.
                ast::Entry::Comment(_)
                | ast::Entry::GroupComment(_)
                | ast::Entry::ResourceComment(_) => continue,
            };

            let id = message.id.name.to_string();
            if !id.starts_with(&format!("{resource}-")) {
                errors.push(format!(
                    "i18n/{locale}/{resource}.ftl: `{id}` does not start with `{resource}-`. The \
                     prefix is the file, so that an id says where to look."
                ));
            }
            if !id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                errors.push(format!(
                    "i18n/{locale}/{resource}.ftl: `{id}` is not `[a-z0-9-]+`"
                ));
            }
            if !message.attributes.is_empty() {
                errors.push(format!(
                    "i18n/{locale}/{resource}.ftl: `{id}` has attributes. This catalogue is one \
                     string per message; split it into two messages instead."
                ));
            }
            let Some(value) = message.value.as_ref() else {
                errors.push(format!("i18n/{locale}/{resource}.ftl: `{id}` has no value"));
                continue;
            };

            let comment: Vec<String> = message
                .comment
                .as_ref()
                .map(|comment| {
                    comment
                        .content
                        .iter()
                        .map(|line| line.trim().to_string())
                        .collect()
                })
                .unwrap_or_default();

            let mut variables = Vec::new();
            let mut text = String::new();
            collect_pattern(
                value,
                false,
                &mut variables,
                &mut text,
                &id,
                locale,
                &resource,
                errors,
            );

            let entry = Message {
                resource: resource.clone(),
                comment,
                text: text.split_whitespace().collect::<Vec<_>>().join(" "),
                variables,
            };
            if out.insert(id.clone(), entry).is_some() {
                errors.push(format!(
                    "i18n/{locale}: `{id}` is declared in more than one resource"
                ));
            }
        }
    }

    Some(out)
}

/// Walk one pattern: gather the `$variables` in order of first appearance, remember whether each is
/// a number, and render the pattern back to something readable for the generated documentation.
#[allow(clippy::too_many_arguments)]
fn collect_pattern(
    pattern: &ast::Pattern<&str>,
    in_number: bool,
    variables: &mut Vec<Variable>,
    out: &mut String,
    id: &str,
    locale: &str,
    resource: &str,
    errors: &mut Vec<String>,
) {
    for element in &pattern.elements {
        match element {
            ast::PatternElement::TextElement { value } => out.push_str(value),
            ast::PatternElement::Placeable { expression } => {
                out.push('{');
                collect_expression(
                    expression, in_number, variables, out, id, locale, resource, errors,
                );
                out.push('}');
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_expression(
    expression: &ast::Expression<&str>,
    in_number: bool,
    variables: &mut Vec<Variable>,
    out: &mut String,
    id: &str,
    locale: &str,
    resource: &str,
    errors: &mut Vec<String>,
) {
    match expression {
        ast::Expression::Inline(inline) => collect_inline(
            inline, in_number, variables, out, id, locale, resource, errors,
        ),
        ast::Expression::Select { selector, variants } => {
            // A selection whose every variant is a plural category selects a *number*. Anything
            // else selects a string, and Fluent is handed the two differently.
            let plural = variants.iter().all(|variant| match &variant.key {
                ast::VariantKey::Identifier { name } => PLURAL_CATEGORIES.contains(name),
                ast::VariantKey::NumberLiteral { .. } => false,
            });
            collect_inline(
                selector, plural, variables, out, id, locale, resource, errors,
            );
            out.push_str(" ->");
            for variant in variants {
                let key = match &variant.key {
                    ast::VariantKey::Identifier { name } => (*name).to_string(),
                    ast::VariantKey::NumberLiteral { value } => (*value).to_string(),
                };
                write!(out, " {}{key}]", if variant.default { "*[" } else { "[" }).ok();
                collect_pattern(
                    &variant.value,
                    in_number,
                    variables,
                    out,
                    id,
                    locale,
                    resource,
                    errors,
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_inline(
    inline: &ast::InlineExpression<&str>,
    in_number: bool,
    variables: &mut Vec<Variable>,
    out: &mut String,
    id: &str,
    locale: &str,
    resource: &str,
    errors: &mut Vec<String>,
) {
    match inline {
        ast::InlineExpression::VariableReference { id: variable } => {
            let kind = if in_number { Kind::Number } else { Kind::Text };
            declare(variables, variable.name, kind);
            write!(out, " ${}", variable.name).ok();
        }
        ast::InlineExpression::NumberLiteral { value } => {
            write!(out, " {value}").ok();
        }
        ast::InlineExpression::StringLiteral { value } => {
            write!(out, " {value:?}").ok();
        }
        ast::InlineExpression::FunctionReference {
            id: function,
            arguments,
        } => {
            if !FUNCTIONS.contains(&function.name) {
                errors.push(format!(
                    "i18n/{locale}/{resource}.ftl: `{id}` calls `{}`, and the only function this \
                     catalogue installs is {FUNCTIONS:?}",
                    function.name
                ));
                return;
            }
            // `NUMBER($n)`: its positional arguments are the number, its options are not.
            write!(out, " {}( ", function.name).ok();
            for argument in &arguments.positional {
                collect_inline(argument, true, variables, out, id, locale, resource, errors);
                out.push(' ');
            }
            out.push(')');
        }
        ast::InlineExpression::MessageReference { id: reference, .. } => {
            errors.push(format!(
                "i18n/{locale}/{resource}.ftl: `{id}` refers to the message `{}`. Messages do not \
                 refer to each other here: a translator would have to hold two of them in mind at \
                 once to see one screen.",
                reference.name
            ));
        }
        ast::InlineExpression::TermReference { id: term, .. } => {
            errors.push(format!(
                "i18n/{locale}/{resource}.ftl: `{id}` refers to the term `-{}-`",
                term.name
            ));
        }
        ast::InlineExpression::Placeable { expression } => {
            out.push_str(" (");
            collect_expression(
                expression, in_number, variables, out, id, locale, resource, errors,
            );
            out.push(')');
        }
    }
}

fn declare(variables: &mut Vec<Variable>, name: &str, kind: Kind) {
    if let Some(existing) = variables.iter_mut().find(|variable| variable.name == name) {
        // A number used bare in one place and inside `NUMBER()` in another is still a number.
        if kind == Kind::Number {
            existing.kind = Kind::Number;
        }
        return;
    }
    variables.push(Variable {
        name: name.to_string(),
        kind,
    });
}

fn generate(
    locales: &[String],
    source: &Catalogue,
    all: &BTreeMap<String, Catalogue>,
) -> Result<(), String> {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    // --- locales.rs -------------------------------------------------------------------------
    let mut locales_rs = String::new();
    locales_rs.push_str(
        "/// The languages this build speaks. One variant per directory under `i18n/`, generated\n\
         /// by `build.rs` — adding a language is adding a directory, and translating every\n\
         /// message in it.\n\
         #[derive(\n    Debug,\n    Clone,\n    Copy,\n    PartialEq,\n    Eq,\n    PartialOrd,\n    Ord,\n    Hash,\n    serde::Serialize,\n    serde::Deserialize,\n)]\n\
         pub enum Locale {\n",
    );
    for locale in locales {
        writeln!(
            locales_rs,
            "    /// `{locale}`\n    #[serde(rename = \"{locale}\")]\n    {},\n",
            variant(locale)
        )
        .ok();
    }
    locales_rs.push_str("}\n\nimpl Locale {\n");

    writeln!(
        locales_rs,
        "    /// Every locale this build carries, in the order the language picker shows them.\n    \
         pub const ALL: &'static [Locale] = &[{}];\n",
        locales
            .iter()
            .map(|locale| format!("Locale::{}", variant(locale)))
            .collect::<Vec<_>>()
            .join(", ")
    )
    .ok();

    writeln!(
        locales_rs,
        "\n    /// The language the messages are written in, and the one a bundle falls back to.\n    \
         pub const SOURCE: Locale = Locale::{};\n",
        variant(SOURCE)
    )
    .ok();

    locales_rs.push_str(
        "\n    /// The tag this locale is stored and matched under: `en`, `ru`, `pt-br`.\n    \
         pub const fn id(self) -> &'static str {\n        match self {\n",
    );
    for locale in locales {
        writeln!(
            locales_rs,
            "            Locale::{} => \"{locale}\",",
            variant(locale)
        )
        .ok();
    }
    locales_rs.push_str("        }\n    }\n");

    locales_rs.push_str(
        "\n    /// Index into the bundle table, so a locale can be looked up without a map.\n    \
         const fn index(self) -> usize {\n        match self {\n",
    );
    for (position, locale) in locales.iter().enumerate() {
        writeln!(
            locales_rs,
            "            Locale::{} => {position},",
            variant(locale)
        )
        .ok();
    }
    locales_rs.push_str("        }\n    }\n");

    locales_rs.push_str(
        "\n    /// A locale from its tag. `None` for a tag this build does not carry — an unknown\n    \
         /// language is not an error, it is English.\n    \
         pub fn from_id(id: &str) -> Option<Self> {\n        match id {\n",
    );
    for locale in locales {
        writeln!(
            locales_rs,
            "            \"{locale}\" => Some(Locale::{}),",
            variant(locale)
        )
        .ok();
    }
    locales_rs.push_str("            _ => None,\n        }\n    }\n}\n\n");

    // The resource tables: the catalogues, embedded in the binary.
    locales_rs.push_str(
        "/// Every catalogue file of one locale, as it is embedded in the binary. The order is\n\
         /// stable — a `.ftl` file added to `i18n/` appears here after a rebuild and nothing else\n\
         /// has to change.\n\
         fn resources(locale: Locale) -> &'static [(&'static str, &'static str)] {\n    \
         match locale {\n",
    );
    for locale in locales {
        writeln!(
            locales_rs,
            "        Locale::{} => {}_RESOURCES,",
            variant(locale),
            constant(locale)
        )
        .ok();
    }
    locales_rs.push_str("    }\n}\n");

    for locale in locales {
        writeln!(
            locales_rs,
            "\nconst {}_RESOURCES: &[(&str, &str)] = &[",
            constant(locale)
        )
        .ok();
        for resource in all.get(locale).map(resources_of).unwrap_or_default() {
            writeln!(
                locales_rs,
                "    (\"{resource}\", include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/i18n/{locale}/{resource}.ftl\"))),"
            )
            .ok();
        }
        locales_rs.push_str("];\n");
    }

    fs::write(out_dir.join("locales.rs"), &locales_rs).map_err(|error| error.to_string())?;

    // --- messages.rs ------------------------------------------------------------------------
    let mut messages_rs = String::new();
    messages_rs.push_str(
        "// Generated by build.rs from i18n/en: one method per message, taking the arguments that\n\
         // message's own pattern uses. Do not edit — edit the catalogues.\n\n\
         impl I18n {\n",
    );

    for (id, message) in source {
        let method = method_name(id);
        for line in &message.comment {
            writeln!(messages_rs, "    /// {line}").ok();
        }
        writeln!(messages_rs, "    ///").ok();
        writeln!(
            messages_rs,
            "    /// `{}` · i18n/en/{}.ftl",
            message.text.replace('\\', "\\\\"),
            message.resource
        )
        .ok();
        if message.variables.len() > 7 {
            messages_rs.push_str("    #[allow(clippy::too_many_arguments)]\n");
        }
        write!(messages_rs, "    pub fn {method}(&self").ok();
        for variable in &message.variables {
            match variable.kind {
                Kind::Text => write!(messages_rs, ", {}: impl AsRef<str>", variable.name).ok(),
                Kind::Number => write!(messages_rs, ", {}: i64", variable.name).ok(),
            };
        }
        messages_rs.push_str(") -> String {\n        self.format(\n            \"");
        messages_rs.push_str(id);
        messages_rs.push_str("\",\n            &[");
        for (position, variable) in message.variables.iter().enumerate() {
            if position > 0 {
                messages_rs.push_str(", ");
            }
            match variable.kind {
                Kind::Text => write!(
                    messages_rs,
                    "(\"{}\", FluentValue::from({}.as_ref()))",
                    variable.name, variable.name
                )
                .ok(),
                Kind::Number => write!(
                    messages_rs,
                    "(\"{}\", FluentValue::from({}))",
                    variable.name, variable.name
                )
                .ok(),
            };
        }
        messages_rs.push_str("],\n        )\n    }\n\n");
    }
    messages_rs.push_str("}\n");

    // The ids, for the tests that have to look at all of them at once.
    messages_rs.push_str(
        "\n/// Every message id in the catalogue.\n\
         #[allow(dead_code)]\n\
         pub(crate) struct MessageIds;\n\n\
         #[allow(dead_code)]\n\
         impl MessageIds {\n    \
         pub(crate) const ALL: &'static [&'static str] = &[\n",
    );
    for id in source.keys() {
        writeln!(messages_rs, "        \"{id}\",").ok();
    }
    messages_rs.push_str("    ];\n}\n");

    fs::write(out_dir.join("messages.rs"), &messages_rs).map_err(|error| error.to_string())?;
    Ok(())
}

/// `pt-br` becomes `PtBr`.
fn variant(locale: &str) -> String {
    locale
        .split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// `pt-br` becomes `PT_BR`.
fn constant(locale: &str) -> String {
    locale.to_uppercase().replace('-', "_")
}
