//! The documents against the tree.
//!
//! A document that names a section, a package, a subcommand, a path or a
//! relative link is stale as soon as that name stops existing. Every one of
//! those is decidable from the tree alone, so this check never compiles
//! anything and still runs when the code does not.
//!
//! What it cannot decide is whether the prose is still *true*. A section that
//! exists but says the wrong thing passes; that is a reader's job, and the
//! reason the prose is kept to what a reader needs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::SUBCOMMANDS;

/// How far back a `§N.M` looks for the document it means.
const LOOKBEHIND: usize = 40;

/// The prefixes a backticked span must have to be read as a repository path.
const ROOTS: [&str; 8] = [
    "crates/",
    "demos/",
    "docs/",
    "firmware/",
    "fuzz/",
    "xtask/",
    ".cargo/",
    ".github/",
];

pub fn check(root: &Path) -> Vec<String> {
    let documents = read_all(root);
    let sections = documents
        .iter()
        .map(|(path, text)| (stem(path).to_owned(), headings(text)))
        .collect::<BTreeMap<_, _>>();

    let mut problems = Vec::new();
    let mut packages = BTreeSet::new();
    for (path, text) in &documents {
        references(path, text, &sections, &mut problems);
        commands(root, path, text, &mut packages, &mut problems);
        paths(root, path, text, &mut problems);
        links(path, text, &mut problems);
    }
    existing_packages(root, &packages, &mut problems);

    problems
}

fn read_all(root: &Path) -> Vec<(PathBuf, String)> {
    let mut documents = Vec::new();

    let mut paths = match std::fs::read_dir(root.join("docs")) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    paths.sort();
    for name in [
        "README.md",
        "AGENTS.md",
        "fuzz/README.md",
        "firmware/README.md",
    ] {
        let path = root.join(name);
        if path.exists() {
            paths.push(path);
        }
    }

    for path in paths {
        if let Ok(text) = std::fs::read_to_string(&path) {
            documents.push((path, text));
        }
    }
    documents
}

fn stem(path: &Path) -> &str {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn headings(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|line| {
            let hashes = line
                .chars()
                .take_while(|character| *character == '#')
                .count();
            if !(2..=4).contains(&hashes) {
                return None;
            }
            let section = line[hashes..]
                .trim_start()
                .chars()
                .take_while(|character| character.is_ascii_digit() || *character == '.')
                .collect::<String>();
            section
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
                .then(|| section.trim_end_matches('.').to_owned())
        })
        .collect()
}

fn references(
    path: &Path,
    text: &str,
    sections: &BTreeMap<String, BTreeSet<String>>,
    problems: &mut Vec<String>,
) {
    let own = stem(path);
    let characters = text.chars().collect::<Vec<_>>();
    let mut line = 1;
    let mut index = 0;

    while index < characters.len() {
        if characters[index] == '\n' {
            line += 1;
            index += 1;
            continue;
        }
        if characters[index] != '§' {
            index += 1;
            continue;
        }

        let mut cursor = index + 1;
        while cursor < characters.len() && characters[cursor].is_whitespace() {
            cursor += 1;
        }
        let start = cursor;
        while cursor < characters.len()
            && (characters[cursor].is_ascii_digit() || characters[cursor] == '.')
        {
            cursor += 1;
        }
        if cursor == start {
            index += 1;
            continue;
        }

        let section = characters[start..cursor]
            .iter()
            .collect::<String>()
            .trim_end_matches('.')
            .to_owned();
        let window = characters[index.saturating_sub(LOOKBEHIND)..index]
            .iter()
            .collect::<String>();
        let target = document_named(&window)
            .filter(|named| sections.contains_key(*named))
            .unwrap_or(own);
        let exists = sections
            .get(target)
            .is_some_and(|known| known.contains(&section));

        if !exists {
            problems.push(format!(
                "docs: {}:{line}: §{section} does not exist in {target}.md",
                path.display()
            ));
        }
        index = cursor;
    }
}

/// The document a reference means: the last `name.md` before it, if any.
fn document_named(window: &str) -> Option<&str> {
    let end = window.rfind(".md")?;
    let bytes = window.as_bytes();
    let mut start = end;
    while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
        start -= 1;
    }
    (start < end).then(|| &window[start..end])
}

fn fenced(text: &str) -> Vec<(usize, &str)> {
    let mut inside = false;
    let mut lines = Vec::new();
    for (number, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            inside = !inside;
            continue;
        }
        if inside {
            lines.push((number + 1, line));
        }
    }
    lines
}

fn commands(
    root: &Path,
    path: &Path,
    text: &str,
    packages: &mut BTreeSet<String>,
    problems: &mut Vec<String>,
) {
    for (number, line) in fenced(text) {
        let command = line
            .split_once(" #")
            .map_or(line, |(command, _)| command)
            .trim();
        if command.contains('|')
            || command.contains('$')
            || command.contains("...")
            || command.trim_end().ends_with('\\')
        {
            continue;
        }
        let words = command.split_whitespace().collect::<Vec<_>>();
        let at = |word: &str| words.iter().position(|candidate| *candidate == word);

        if let Some(position) = at("xtask") {
            match words.get(position + 1) {
                Some(name) if SUBCOMMANDS.iter().any(|(known, _)| known == name) => {}
                Some(name) => problems.push(format!(
                    "docs: {}:{number}: `cargo xtask {name}` is not a subcommand",
                    path.display()
                )),
                None => problems.push(format!(
                    "docs: {}:{number}: `cargo xtask` names no subcommand",
                    path.display()
                )),
            }
        }

        for flag in ["-p", "--package"] {
            if let Some(position) = at(flag) {
                match words.get(position + 1) {
                    Some(name) => {
                        packages.insert(name.trim_end_matches([',', '.']).to_owned());
                    }
                    None => problems.push(format!(
                        "docs: {}:{number}: `{flag}` names no package",
                        path.display()
                    )),
                }
            }
        }

        if let Some(position) = at("--path") {
            match words.get(position + 1) {
                Some(directory) => {
                    let target = root.join(directory.trim_end_matches([',', '.']));
                    if !target.exists() {
                        problems.push(format!(
                            "docs: {}:{number}: `--path {directory}` does not exist",
                            path.display()
                        ));
                    }
                }
                None => problems.push(format!(
                    "docs: {}:{number}: `--path` names nothing",
                    path.display()
                )),
            }
        }
    }
}

fn existing_packages(root: &Path, packages: &BTreeSet<String>, problems: &mut Vec<String>) {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    for package in packages {
        let status = Command::new(&cargo)
            .args(["pkgid", "-p", package])
            .current_dir(root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match status {
            Ok(status) if status.success() => {}
            Ok(_) => problems.push(format!(
                "docs: a document names package `{package}`, which the workspace does not have"
            )),
            Err(error) => {
                problems.push(format!("docs: cannot run `cargo pkgid`: {error}"));
                break;
            }
        }
    }
}

fn paths(root: &Path, path: &Path, text: &str, problems: &mut Vec<String>) {
    for span in backticked(text) {
        if span.contains('\n') || span.contains(' ') || span.contains('*') {
            continue;
        }
        let name = span
            .split("::")
            .next()
            .unwrap_or_default()
            .trim_end_matches('/');
        if !ROOTS.iter().any(|prefix| name.starts_with(prefix)) {
            continue;
        }
        if !root.join(name).exists() {
            problems.push(format!(
                "docs: {}: `{name}` does not exist",
                relative(root, path)
            ));
        }
    }
}

fn backticked(text: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    let mut inside = false;
    let mut start = 0;
    for (index, character) in text.char_indices() {
        if character != '`' {
            continue;
        }
        if inside {
            spans.push(&text[start..index]);
        } else {
            start = index + 1;
        }
        inside = !inside;
    }
    spans
}

fn links(path: &Path, text: &str, problems: &mut Vec<String>) {
    let directory = path.parent().unwrap_or(Path::new("."));
    let mut rest = text;

    while let Some(position) = rest.find("](") {
        rest = &rest[position + 2..];
        let Some(end) = rest.find(')') else {
            break;
        };
        let target = &rest[..end];
        rest = &rest[end + 1..];

        if target.starts_with("http") || target.starts_with('#') || target.is_empty() {
            continue;
        }
        let file = target.split('#').next().unwrap_or_default();
        if file.is_empty() {
            continue;
        }
        if !directory.join(file).exists() {
            problems.push(format!(
                "docs: {}: link `{target}` does not exist",
                path.display()
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_belongs_to_the_document_named_before_it() {
        assert_eq!(document_named("见 `isa.md` §4.8 的"), Some("isa"));
        assert_eq!(document_named("§4.8"), None);
        assert_eq!(document_named("a.md then b.md "), Some("b"));
    }

    #[test]
    fn headings_are_the_numbered_ones() {
        let found = headings("# 0\n## 4. table\n### 4.1 row\n#### deep\n## no number\n");
        assert_eq!(found, ["4", "4.1"].into_iter().map(str::to_owned).collect());
    }

    #[test]
    fn backticks_pair_up() {
        assert_eq!(backticked("`crates/a` and `b`"), ["crates/a", "b"]);
        assert_eq!(backticked("no code"), Vec::<&str>::new());
    }

    #[test]
    fn fences_carry_the_line_numbers() {
        let text = "before\n```bash\ncargo xtask ci\n```\nafter\n";
        assert_eq!(fenced(text), vec![(3, "cargo xtask ci")]);
    }
}
