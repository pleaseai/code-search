//! File language detection and content classification. Port of
//! `src/indexing/files.ts` (← semble `index/files.py`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

use crate::types::ContentType;

/// Extension (including the leading dot, lowercase) → tree-sitter language name.
/// Transcribed verbatim from the upstream `EXTENSION_TO_LANGUAGE`.
pub const EXTENSION_TO_LANGUAGE: &[(&str, &str)] = &[
    (".4th", "forth"),
    (".ada", "ada"),
    (".adb", "ada"),
    (".adoc", "asciidoc"),
    (".ads", "ada"),
    (".agda", "agda"),
    (".al", "al"),
    (".as", "actionscript"),
    (".asciidoc", "asciidoc"),
    (".asm", "asm"),
    (".astro", "astro"),
    (".awk", "awk"),
    (".axi", "netlinx"),
    (".axs", "netlinx"),
    (".bash", "bash"),
    (".bat", "batch"),
    (".bb", "bitbake"),
    (".bbappend", "bitbake"),
    (".bbclass", "bitbake"),
    (".beancount", "beancount"),
    (".bib", "bibtex"),
    (".bicep", "bicep"),
    (".blade", "blade"),
    (".bq", "sql_bigquery"),
    (".brs", "brightscript"),
    (".bsl", "bsl"),
    (".bzl", "starlark"),
    (".c", "c"),
    (".c3", "c3"),
    (".c3i", "c3"),
    (".c3t", "c3"),
    (".caddyfile", "caddy"),
    (".cairo", "cairo"),
    (".capnp", "capnp"),
    (".cbl", "cobol"),
    (".cc", "cpp"),
    (".cedar", "cedar"),
    (".cedarschema", "cedarschema"),
    (".cel", "cel"),
    (".cfc", "cfml"),
    (".cfg", "ini"),
    (".chatito", "chatito"),
    (".circom", "circom"),
    (".cjs", "javascript"),
    (".ck", "chuck"),
    (".cl", "commonlisp"),
    (".clar", "clarity"),
    (".clj", "clojure"),
    (".cljc", "clojure"),
    (".cljs", "clojure"),
    (".cls", "abl"),
    (".cmake", "cmake"),
    (".cmd", "batch"),
    (".cob", "cobol"),
    (".cobol", "cobol"),
    (".conf", "nginx"),
    (".cook", "cooklang"),
    (".corn", "corn"),
    (".cpon", "cpon"),
    (".cpp", "cpp"),
    (".cr", "crystal"),
    (".cs", "csharp"),
    (".cshtml", "razor"),
    (".css", "css"),
    (".cst", "cst"),
    (".csv", "csv"),
    (".cts", "typescript"),
    (".cu", "cuda"),
    (".cuda", "cuda"),
    (".cue", "cue"),
    (".cxx", "cpp"),
    (".cylc", "cylc"),
    (".d", "d"),
    (".dart", "dart"),
    (".desktop", "desktop"),
    (".dhall", "dhall"),
    (".diff", "diff"),
    (".dj", "djot"),
    (".dl", "souffle"),
    (".dockerfile", "dockerfile"),
    (".dot", "dot"),
    (".dsp", "faust"),
    (".dtd", "dtd"),
    (".dts", "devicetree"),
    (".dtsi", "devicetree"),
    (".ebnf", "ebnf"),
    (".eds", "eds"),
    (".eex", "eex"),
    (".el", "elisp"),
    (".elm", "elm"),
    (".elv", "elvish"),
    (".enforce", "enforce"),
    (".eps", "postscript"),
    (".erb", "embeddedtemplate"),
    (".erl", "erlang"),
    (".ex", "elixir"),
    (".exs", "elixir"),
    (".f", "fortran"),
    (".f03", "fortran"),
    (".f08", "fortran"),
    (".f90", "fortran"),
    (".f95", "fortran"),
    (".fc", "func"),
    (".fidl", "fidl"),
    (".filter", "poe_filter"),
    (".fir", "firrtl"),
    (".fish", "fish"),
    (".fnl", "fennel"),
    (".fs", "fsharp"),
    (".fsd", "facility"),
    (".fsi", "fsharp_signature"),
    (".fsx", "fsharp"),
    (".fth", "forth"),
    (".fun", "sml"),
    (".g", "gap"),
    (".gd", "gdscript"),
    (".gdshader", "gdshader"),
    (".gi", "gap"),
    (".gitattributes", "gitattributes"),
    (".gitignore", "gitignore"),
    (".gleam", "gleam"),
    (".glsl", "glsl"),
    (".gn", "gn"),
    (".gni", "gn"),
    (".gnuplot", "gnuplot"),
    (".go", "go"),
    (".gotmpl", "gotmpl"),
    (".gp", "gnuplot"),
    (".gql", "graphql"),
    (".gradle", "groovy"),
    (".graphql", "graphql"),
    (".gren", "gren"),
    (".groovy", "groovy"),
    (".gv", "dot"),
    (".h", "c"),
    (".hack", "hack"),
    (".hare", "hare"),
    (".hbs", "glimmer"),
    (".hcl", "hcl"),
    (".heex", "heex"),
    (".hjson", "hjson"),
    (".hlsl", "hlsl"),
    (".hocon", "hocon"),
    (".hoon", "hoon"),
    (".hpp", "cpp"),
    (".hrl", "erlang"),
    (".hs", "haskell"),
    (".htm", "html"),
    (".html", "html"),
    (".http", "http"),
    (".hurl", "hurl"),
    (".hx", "haxe"),
    (".hxx", "cpp"),
    (".idr", "idris"),
    (".inc", "sourcepawn"),
    (".ini", "ini"),
    (".ino", "arduino"),
    (".ispc", "ispc"),
    (".j2", "jinja2"),
    (".jai", "jai"),
    (".janet", "janet"),
    (".java", "java"),
    (".jinja2", "jinja2"),
    (".jl", "julia"),
    (".journal", "ledger"),
    (".jq", "jq"),
    (".js", "javascript"),
    (".json", "json"),
    (".json5", "json5"),
    (".jsonnet", "jsonnet"),
    (".jsx", "javascript"),
    (".just", "just"),
    (".k", "kcl"),
    (".kdl", "kdl"),
    (".kt", "kotlin"),
    (".kts", "kotlin"),
    (".lc", "elsa"),
    (".ldg", "ledger"),
    (".lds", "linkerscript"),
    (".lean", "lean"),
    (".ledger", "ledger"),
    (".leex", "eex"),
    (".less", "less"),
    (".libsonnet", "jsonnet"),
    (".liquid", "liquid"),
    (".lisp", "commonlisp"),
    (".ll", "llvm"),
    (".lua", "lua"),
    (".luau", "luau"),
    (".m", "objc"),
    (".magik", "magik"),
    (".makefile", "make"),
    (".markdown", "markdown"),
    (".matlab", "matlab"),
    (".md", "markdown"),
    (".mermaid", "mermaid"),
    (".meson", "meson"),
    (".mjs", "javascript"),
    (".mk", "make"),
    (".ml", "ocaml"),
    (".mli", "ocaml_interface"),
    (".mlir", "mlir"),
    (".mll", "ocamllex"),
    (".mmd", "mermaid"),
    (".mod", "gomod"),
    (".mojo", "mojo"),
    (".move", "move"),
    (".mts", "typescript"),
    (".nasm", "nasm"),
    (".ncl", "nickel"),
    (".nginx", "nginx"),
    (".nim", "nim"),
    (".nims", "nim"),
    (".ninja", "ninja"),
    (".nix", "nix"),
    (".norg", "norg"),
    (".nqc", "nqc"),
    (".nu", "nushell"),
    (".nut", "squirrel"),
    (".odin", "odin"),
    (".org", "org"),
    (".p", "abl"),
    (".pas", "pascal"),
    (".patch", "diff"),
    (".pbtxt", "textproto"),
    (".pem", "pem"),
    (".pgn", "pgn"),
    (".php", "php"),
    (".pkl", "pkl"),
    (".pl", "perl"),
    (".plt", "gnuplot"),
    (".pm", "perl"),
    (".po", "po"),
    (".pony", "pony"),
    (".pot", "po"),
    (".pp", "puppet"),
    (".prisma", "prisma"),
    (".pro", "prolog"),
    (".promql", "promql"),
    (".properties", "properties"),
    (".proto", "proto"),
    (".prql", "prql"),
    (".ps", "postscript"),
    (".ps1", "powershell"),
    (".psd1", "powershell"),
    (".psm1", "powershell"),
    (".psv", "psv"),
    (".pug", "pug"),
    (".purs", "purescript"),
    (".py", "python"),
    (".pyi", "python"),
    (".pyw", "python"),
    (".ql", "ql"),
    (".qml", "qmljs"),
    (".r", "r"),
    (".rasi", "rasi"),
    (".razor", "razor"),
    (".rb", "ruby"),
    (".rbs", "rbs"),
    (".re", "re2c"),
    (".rego", "rego"),
    (".res", "rescript"),
    (".resi", "rescript"),
    (".rkt", "racket"),
    (".robot", "robot"),
    (".roc", "roc"),
    (".ron", "ron"),
    (".rs", "rust"),
    (".rst", "rst"),
    (".rtf", "rtf"),
    (".s", "asm"),
    (".scad", "openscad"),
    (".scala", "scala"),
    (".scm", "scheme"),
    (".scss", "scss"),
    (".sh", "bash"),
    (".shtml", "superhtml"),
    (".sig", "sml"),
    (".slang", "slang"),
    (".smali", "smali"),
    (".smithy", "smithy"),
    (".smk", "snakemake"),
    (".sml", "sml"),
    (".sol", "solidity"),
    (".sp", "sourcepawn"),
    (".sparql", "sparql"),
    (".sql", "sql"),
    (".squirrel", "squirrel"),
    (".st", "smalltalk"),
    (".stan", "stan"),
    (".star", "starlark"),
    (".sv", "systemverilog"),
    (".svelte", "svelte"),
    (".svh", "systemverilog"),
    (".sw", "sway"),
    (".swift", "swift"),
    (".tact", "tact"),
    (".tal", "uxntal"),
    (".tape", "vhs"),
    (".tcl", "tcl"),
    (".td", "tablegen"),
    (".templ", "templ"),
    (".tera", "tera"),
    (".tex", "latex"),
    (".textproto", "textproto"),
    (".tf", "terraform"),
    (".tfvars", "terraform"),
    (".thrift", "thrift"),
    (".tl", "teal"),
    (".tla", "tlaplus"),
    (".todotxt", "todotxt"),
    (".toml", "toml"),
    (".tres", "godot_resource"),
    (".trigger", "apex"),
    (".ts", "typescript"),
    (".tscn", "godot_resource"),
    (".tsconfig", "typoscript"),
    (".tsp", "typespec"),
    (".tsv", "tsv"),
    (".tsx", "tsx"),
    (".ttl", "turtle"),
    (".twig", "twig"),
    // `.txt` → `vimdoc` intentionally omitted (overly broad).
    (".typoscript", "typoscript"),
    (".typst", "typst"),
    (".v", "v"),
    (".vb", "vb"),
    (".verilog", "verilog"),
    (".vhd", "vhdl"),
    (".vhdl", "vhdl"),
    (".vim", "vim"),
    (".vrl", "vrl"),
    (".vue", "vue"),
    (".w", "abl"),
    (".wast", "wast"),
    (".wat", "wat"),
    (".wgsl", "wgsl"),
    (".wit", "wit"),
    (".wl", "wolfram"),
    (".xml", "xml"),
    (".xsl", "xml"),
    (".xslt", "xml"),
    (".yaml", "yaml"),
    (".yml", "yaml"),
    (".yuck", "yuck"),
    (".zig", "zig"),
    (".ziggy", "ziggy"),
    (".zsh", "zsh"),
];

const DOC_LANGUAGES: &[&str] = &[
    "asciidoc",
    "bibtex",
    "djot",
    "doxygen",
    "html",
    "javadoc",
    "jsdoc",
    "latex",
    "luadoc",
    "markdown",
    "markdown_inline",
    "mermaid",
    "norg",
    "norg_meta",
    "org",
    "phpdoc",
    "po",
    "rst",
    "rtf",
    "vimdoc",
];

const CONFIG_LANGUAGES: &[&str] = &[
    "beancount",
    "capnp",
    "cedarschema",
    "comment",
    "cooklang",
    "cpon",
    "desktop",
    "devicetree",
    "diff",
    "dtd",
    "editorconfig",
    "ebnf",
    "git_config",
    "gitattributes",
    "gitcommit",
    "gitignore",
    "godot_resource",
    "gomod",
    "gosum",
    "gowork",
    "gpg",
    "hjson",
    "hocon",
    "ini",
    "kdl",
    "ledger",
    "pem",
    "pgn",
    "properties",
    "proto",
    "requirements",
    "ron",
    "smithy",
    "ssh_config",
    "textproto",
    "thrift",
    "todotxt",
    "toml",
    "turtle",
    "typespec",
    "wit",
    "xcompose",
    "xml",
    "yaml",
    "ziggy_schema",
];

const DATA_LANGUAGES: &[&str] = &["csv", "json", "json5", "psv", "tsv"];

/// Extension → language lookup.
static EXT_MAP: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| EXTENSION_TO_LANGUAGE.iter().copied().collect());

/// Every language referenced by the extension map.
pub static ALL_LANGUAGES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    EXTENSION_TO_LANGUAGE
        .iter()
        .map(|&(_, lang)| lang)
        .collect()
});

static DOC_SET: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| DOC_LANGUAGES.iter().copied().collect());
static CONFIG_SET: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| CONFIG_LANGUAGES.iter().copied().collect());
static DATA_SET: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| DATA_LANGUAGES.iter().copied().collect());

/// Code languages = ALL − DOC − CONFIG − DATA.
static CODE_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ALL_LANGUAGES
        .iter()
        .copied()
        .filter(|l| !DOC_SET.contains(l) && !CONFIG_SET.contains(l) && !DATA_SET.contains(l))
        .collect()
});

/// language → extensions (collecting duplicates, in map order).
static LANGUAGE_TO_EXTENSIONS: LazyLock<HashMap<&'static str, Vec<&'static str>>> =
    LazyLock::new(|| {
        let mut inv: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        for &(ext, lang) in EXTENSION_TO_LANGUAGE {
            inv.entry(lang).or_default().push(ext);
        }
        inv
    });

fn languages_for(content_type: ContentType) -> &'static HashSet<&'static str> {
    match content_type {
        ContentType::Code => &CODE_SET,
        ContentType::Docs => &DOC_SET,
        ContentType::Config => &CONFIG_SET,
    }
}

/// Detect the language of a file by its extension. Matching is case-insensitive
/// on the final `.suffix` (mirroring `Path(...).suffix.lower()`); a leading-dot
/// dotfile (`.gitignore`) has no suffix and returns `None`.
pub fn detect_language(file_name: &str) -> Option<&'static str> {
    let last_sep = file_name.rfind(['/', '\\']);
    let base = match last_sep {
        Some(i) => &file_name[i + 1..],
        None => file_name,
    };
    let dot = base.rfind('.')?;
    if dot == 0 {
        return None;
    }
    let ext = base[dot..].to_ascii_lowercase();
    EXT_MAP.get(ext.as_str()).copied()
}

/// Resolve content types to the sorted, de-duplicated union of file extensions
/// for their languages, plus any `extra` extensions appended verbatim.
pub fn get_extensions(types: &[ContentType], extra: Option<&[String]>) -> Vec<String> {
    let mut languages: HashSet<&'static str> = HashSet::new();
    for &t in types {
        for &lang in languages_for(t) {
            languages.insert(lang);
        }
    }
    let mut out: BTreeSet<String> = BTreeSet::new();
    for lang in languages {
        if let Some(exts) = LANGUAGE_TO_EXTENSIONS.get(lang) {
            for &ext in exts {
                out.insert(ext.to_string());
            }
        }
    }
    if let Some(extra) = extra {
        for ext in extra {
            out.insert(ext.clone());
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_languages_by_extension() {
        assert_eq!(detect_language("foo.ts"), Some("typescript"));
        assert_eq!(detect_language("foo.tsx"), Some("tsx"));
        assert_eq!(detect_language("foo.py"), Some("python"));
        assert_eq!(detect_language("foo.md"), Some("markdown"));
    }

    #[test]
    fn unknown_extension_is_none() {
        assert_eq!(detect_language("foo.unknown"), None);
    }

    #[test]
    fn case_insensitive_suffix() {
        assert_eq!(detect_language("Foo.TS"), Some("typescript"));
    }

    #[test]
    fn no_extension_is_none() {
        assert_eq!(detect_language("Makefile"), None);
    }

    #[test]
    fn dotfiles_have_no_suffix() {
        assert_eq!(detect_language(".gitignore"), None);
        assert_eq!(detect_language("dir/.gitignore"), None);
        assert_eq!(detect_language("dir\\.gitignore"), None);
    }

    #[test]
    fn matches_final_suffix_with_multiple_dots() {
        assert_eq!(detect_language("foo.bar.ts"), Some("typescript"));
    }

    #[test]
    fn handles_directory_separators() {
        assert_eq!(detect_language("src/indexing/files.ts"), Some("typescript"));
        assert_eq!(
            detect_language("src\\indexing\\files.ts"),
            Some("typescript")
        );
        assert_eq!(detect_language("C:\\Users\\me\\foo.py"), Some("python"));
    }

    #[test]
    fn code_extensions_include_common_languages() {
        let exts = get_extensions(&[ContentType::Code], None);
        assert!(exts.iter().any(|e| e == ".ts"));
        assert!(exts.iter().any(|e| e == ".py"));
        assert!(exts.iter().any(|e| e == ".go"));
    }

    #[test]
    fn doc_extensions_exclude_code() {
        let exts = get_extensions(&[ContentType::Docs], None);
        assert!(exts.iter().any(|e| e == ".md"));
        assert!(exts.iter().any(|e| e == ".rst"));
        assert!(!exts.iter().any(|e| e == ".ts"));
    }

    #[test]
    fn config_extensions_present() {
        let exts = get_extensions(&[ContentType::Config], None);
        assert!(exts.iter().any(|e| e == ".toml"));
        assert!(exts.iter().any(|e| e == ".yaml"));
    }

    #[test]
    fn appends_user_extensions() {
        let exts = get_extensions(&[ContentType::Code], Some(&[".foo".to_string()]));
        assert!(exts.iter().any(|e| e == ".foo"));
    }

    #[test]
    fn sorted_and_deduplicated() {
        let exts = get_extensions(
            &[ContentType::Code, ContentType::Docs],
            Some(&[".ts".to_string(), ".foo".to_string()]),
        );
        let mut sorted = exts.clone();
        sorted.sort();
        assert_eq!(exts, sorted);
        let unique: BTreeSet<&String> = exts.iter().collect();
        assert_eq!(unique.len(), exts.len());
    }

    #[test]
    fn unions_multiple_content_types() {
        let code: HashSet<String> = get_extensions(&[ContentType::Code], None)
            .into_iter()
            .collect();
        let docs: HashSet<String> = get_extensions(&[ContentType::Docs], None)
            .into_iter()
            .collect();
        let both: HashSet<String> = get_extensions(&[ContentType::Code, ContentType::Docs], None)
            .into_iter()
            .collect();
        for ext in code.iter().chain(docs.iter()) {
            assert!(both.contains(ext));
        }
    }

    #[test]
    fn language_sets_non_empty_and_consistent() {
        assert!(!EXTENSION_TO_LANGUAGE.is_empty());
        assert!(!ALL_LANGUAGES.is_empty());
        assert!(!DOC_SET.is_empty());
        assert!(!CONFIG_SET.is_empty());
        assert!(!DATA_SET.is_empty());
        for &(_, lang) in EXTENSION_TO_LANGUAGE {
            assert!(ALL_LANGUAGES.contains(lang));
        }
    }
}
