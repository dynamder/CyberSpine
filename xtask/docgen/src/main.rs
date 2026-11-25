//! Render the tables that are facts about the code into the documents.
//!
//! Every generated range sits between an open marker and `<!-- docgen:end -->`.
//! `cargo xtask docgen` rewrites the ranges; `--check` fails when a document and
//! the code disagree, and the CI gate runs that. Prose around a range stays a
//! reader's job: this tool only owns what a type can answer.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cyberspine_isa::{OpCode, OperandParam, RegName};

const BEGIN: &str = "<!-- docgen:";
const END: &str = "<!-- docgen:end -->";

/// One generated range.
#[derive(Clone, Copy)]
enum Block {
    Count,
    Group(u8),
    Summary,
}

impl Block {
    fn document(self) -> &'static str {
        "docs/isa.md"
    }

    fn id(self) -> String {
        match self {
            Self::Count => "isa.count".to_owned(),
            Self::Group(group) => format!("isa.group.0x{group:02X}"),
            Self::Summary => "isa.summary".to_owned(),
        }
    }

    fn render(self) -> String {
        match self {
            Self::Count => format!("全部 **{} 条**。", OpCode::ALL.len()),
            Self::Group(group) => isa_group(group),
            Self::Summary => isa_summary(),
        }
    }
}

const BLOCKS: &[Block] = &[
    Block::Count,
    Block::Group(0x00),
    Block::Group(0x10),
    Block::Group(0x20),
    Block::Group(0x30),
    Block::Group(0x40),
    Block::Group(0x50),
    Block::Group(0x60),
    Block::Group(0x70),
    Block::Group(0x80),
    Block::Group(0x90),
    Block::Group(0xA0),
    Block::Summary,
];

fn main() -> ExitCode {
    let check = std::env::args()
        .skip(1)
        .any(|argument| argument == "--check");
    match run(check) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("docgen: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(check: bool) -> Result<(), String> {
    let root = root();
    let mut by_document: BTreeMap<&'static str, Vec<Block>> = BTreeMap::new();
    for block in BLOCKS {
        by_document
            .entry(block.document())
            .or_default()
            .push(*block);
    }

    let mut stale = Vec::new();
    for (document, blocks) in by_document {
        let path = root.join(document);
        let raw =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;

        // A checkout may hand the file either ending; the comparison and the
        // rewrite work on one, and the file keeps the ending it arrived with.
        let crlf = raw.contains("\r\n");
        let text = raw.replace("\r\n", "\n");
        let rewritten = rewrite(document, &text, &blocks)?;
        let rewritten = if crlf {
            rewritten.replace('\n', "\r\n")
        } else {
            rewritten
        };
        if rewritten == raw {
            continue;
        }

        stale.push(document.to_owned());
        if !check {
            fs::write(&path, &rewritten).map_err(|error| format!("{}: {error}", path.display()))?;
        } else {
            eprintln!("docgen: {}", first_difference(&raw, &rewritten));
        }
    }

    if check && !stale.is_empty() {
        return Err(format!(
            "{} document(s) out of date: {}; run `cargo xtask docgen`",
            stale.len(),
            stale.join(", ")
        ));
    }
    Ok(())
}

/// Replace every generated range, and refuse anything ambiguous.
fn rewrite(document: &str, text: &str, blocks: &[Block]) -> Result<String, String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut seen: Vec<String> = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let line = lines[index];
        if let Some(id) = open_id(line) {
            let block = blocks
                .iter()
                .find(|block| block.id() == id)
                .ok_or_else(|| format!("{document}: `{id}` is not a generated block"))?;
            if seen.contains(&id) {
                return Err(format!("{document}: `{id}` appears twice"));
            }

            let end = (index + 1..lines.len())
                .find(|candidate| lines[*candidate].trim_end() == END)
                .ok_or_else(|| format!("{document}: `{id}` has no end marker"))?;

            out.push(line.to_owned());
            out.extend(block.render().lines().map(str::to_owned));
            out.push(lines[end].to_owned());
            seen.push(id);
            index = end + 1;
            continue;
        }

        if line.trim_end() == END {
            return Err(format!("{document}: an end marker without an open one"));
        }

        out.push(line.to_owned());
        index += 1;
    }

    for block in blocks {
        let id = block.id();
        if !seen.contains(&id) {
            return Err(format!("{document}: `{id}` is missing"));
        }
    }

    let mut text = out.join("\n");
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    Ok(text)
}

fn open_id(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix(BEGIN)?;
    let id = rest.strip_suffix(" -->")?.trim();
    Some(id.to_owned())
}

/// Where a document and its generated form diverge, for the failure message.
fn first_difference(actual: &str, generated: &str) -> String {
    for (index, (left, right)) in actual.lines().zip(generated.lines()).enumerate() {
        if left != right {
            return format!(
                "line {} differs:\n  document:  {left}\n  generated: {right}",
                index + 1
            );
        }
    }
    "the generated text has a different length".to_owned()
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest is two levels below the checkout")
        .to_path_buf()
}

fn isa_group(group: u8) -> String {
    let mut out = String::from("| 编码 | 助记符 | 操作数 | 语义 |\n|---|---|---|---|\n");
    for op in OpCode::ALL
        .iter()
        .copied()
        .filter(|op| op.byte() >> 4 == group >> 4)
    {
        let summary = op.summary();
        let summary = if summary.is_ascii() {
            format!("`{summary}`")
        } else {
            summary.to_owned()
        };
        out.push_str(&format!(
            "| 0x{:02X} | `{}` | {} | {summary} |\n",
            op.byte(),
            op.mnemonic(),
            operands(op)
        ));
    }
    out
}

fn operands(op: OpCode) -> String {
    let names: Vec<&str> = op
        .signature()
        .iter()
        .map(|param| match param {
            OperandParam::Reg(name, _) => match name {
                RegName::Rd => "rd",
                RegName::Ra => "ra",
                RegName::Rb => "rb",
                RegName::Rc => "rc",
            },
            OperandParam::Imm(_) => "imm",
        })
        .collect();

    if names.is_empty() {
        "—".to_owned()
    } else {
        format!("`{}`", names.join(", "))
    }
}

fn isa_summary() -> String {
    let mut out = String::from("```text\n");
    for group in 0..=0xA_u8 {
        let ops: Vec<OpCode> = OpCode::ALL
            .iter()
            .copied()
            .filter(|op| op.byte() >> 4 == group)
            .collect();
        if ops.is_empty() {
            continue;
        }

        let mut line = format!("0x{:02X} {:>2}   ", group << 4, ops.len());
        for (index, op) in ops.iter().enumerate() {
            let name = op.mnemonic();
            let separator = if index == 0 { "" } else { "  " };
            if index > 0 && line.len() + separator.len() + name.len() > 76 {
                out.push_str(&line);
                out.push('\n');
                line = format!("          {name}");
            } else {
                line.push_str(separator);
                line.push_str(name);
            }
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("```\n");
    out
}
