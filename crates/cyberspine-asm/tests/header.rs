//! `header()` 的行为测试（`docs/asm.md` §1–§3）。
//!
//! **为什么每个逆向用例都配基线。** "它被拒绝了"不是要断言的性质，断言的性质是
//! "**这条规则**拒绝了它"。所以每个非法输入都构造成「合法基线 + 一处扰动」，
//! 扰动之外的每一行都保持合法；`control_*` 用例则证明基线本身能通过。
//! 否则一个"拒绝一切"的解析器会让整份逆向测试全绿 ——
//! `crates/cyberspine-core/tests/load.rs` 开头警告的正是这个失败模式。
//!
//! **错误契约。** `header()` 现在返回 `Result<AsmHeader, AsmParseErrors>`；
//! 每个错误带 1-based 行列和稳定的 `ParseErrorKind`，并且按行聚合。
//! 所以逆向用例不仅能断言"被拒绝"，还能断言"被哪条规则、哪一行拒绝"。

use cyberspine_asm::assembler::{
    AsmParseErrors, Location, ParseError, ParseErrorKind,
    header::{AsmHeader, FixedHeader, GroupDef, header},
};
use smol_str::SmolStr;

// ---------------------------------------------------------------------------
// 脚手架
// ---------------------------------------------------------------------------

fn parse(src: &str) -> Result<AsmHeader, AsmParseErrors> {
    let mut cursor = src;
    header(&mut cursor)
}

#[track_caller]
fn parse_ok(src: &str) -> AsmHeader {
    match parse(src) {
        Ok(parsed) => parsed,
        Err(err) => panic!("期望解析成功，却被拒绝：{err:?}\n--- 输入 ---\n{src}"),
    }
}

#[track_caller]
fn parse_errors(src: &str) -> Vec<ParseError> {
    match parse(src) {
        Ok(parsed) => panic!("期望解析被拒绝，却解析成功：{parsed:?}\n--- 输入 ---\n{src}"),
        Err(errors) => errors.errors,
    }
}

type KindMatcher = fn(&ParseErrorKind) -> bool;

fn invalid_value(kind: &ParseErrorKind) -> bool {
    matches!(kind, ParseErrorKind::InvalidFieldValue { .. })
}

fn unknown_directive(kind: &ParseErrorKind) -> bool {
    matches!(kind, ParseErrorKind::UnknownDirective(_))
}

/// The fixture keeps exactly one defect, so the test can demand the one rule
/// that must fire — "it was rejected" would pass against a parser that rejected
/// everything (`docs/asm.md` §6.1).
#[track_caller]
fn assert_rejected(src: &str, why: &str, expected: KindMatcher) {
    let errors = parse_errors(src);
    assert_eq!(errors.len(), 1, "{why}: {errors:?}");
    assert!(
        expected(&errors[0].kind),
        "{why}: unexpected kind {:?}",
        errors[0].kind
    );
    assert_eq!(
        errors[0].location.line, 5,
        "{why}: the error belongs on the perturbed line"
    );
}

/// 把若干行拼成一份以 `\n` 结尾的源码 —— 这是磁盘上真实文件的形状。
fn text(lines: &[&str]) -> String {
    let mut joined = lines.join("\n");
    joined.push('\n');
    joined
}

/// 四个必填标量 + `.text`（`docs/asm.md` §3.1：当前实现要求显式给出）。
/// 缺失字段的行为单独由 `missing_required_scalars_are_rejected` 钉住。
const BASE: &[&str] = &[
    ".layer 10",
    ".clock hz = 60",
    ".fuel 4096",
    ".max_wait 600.0",
    ".text",
];

/// 基线 + 一条额外的伪指令（插在 `.text` 之前）。
/// 失败只可能来自 `extra` 这一行。
fn base_with(extra: &str) -> String {
    let mut lines = BASE.to_vec();
    lines.insert(BASE.len() - 1, extra);
    text(&lines)
}

/// 基线去掉第 `index` 行。
fn base_missing(index: usize) -> String {
    let mut lines = BASE.to_vec();
    lines.remove(index);
    text(&lines)
}

// ---------------------------------------------------------------------------
// 1. 合法：完整 header
// ---------------------------------------------------------------------------

const FULL: &[&str] = &[
    ".layer 10",
    ".clock hz = 60",
    ".fuel 4096",
    ".max_wait 600.0",
    ".rig \"rigs/live2d.rig.json\"",
    ".group brows = brow_l, brow_r",
    ".var_f phase = 0.0",
    ".var_i state = 0",
    ".text",
];

fn expected_full() -> AsmHeader {
    AsmHeader {
        fixed: FixedHeader {
            layer: 10,
            clock_hz: 60,
            fuel: 4096,
            max_wait: 600.0,
        },
        description: None,
        rig: Some(SmolStr::new("rigs/live2d.rig.json")),
        muscle_groups: vec![GroupDef {
            name: SmolStr::new("brows"),
            members: vec![SmolStr::new("brow_l"), SmolStr::new("brow_r")],
        }],
        vars_f: vec![(SmolStr::new("phase"), 0.0)],
        vars_i: vec![(SmolStr::new("state"), 0)],
    }
}

#[test]
fn control_full_header_matches_expected_value() {
    let got = parse_ok(&text(FULL));
    assert_eq!(got, expected_full());
}

/// §3.3：变量的表序按声明序。顺序是语义的一部分（名字→下标），不能被解析顺手排序。
#[test]
fn control_variable_declaration_order_is_preserved() {
    let src = text(&[
        ".layer 1",
        ".clock hz = 1",
        ".fuel 1",
        ".max_wait 1.0",
        ".var_i z = 1",
        ".var_i y = 2",
        ".text",
    ]);
    let got = parse_ok(&src);
    assert_eq!(
        got.vars_i,
        vec![(SmolStr::new("z"), 1), (SmolStr::new("y"), 2)]
    );
}

// ---------------------------------------------------------------------------
// 2. 合法：可选字段缺失 / 省略
// ---------------------------------------------------------------------------

#[test]
fn control_header_with_no_name_tables_parses() {
    let got = parse_ok(&text(BASE));
    assert_eq!(
        got.fixed,
        FixedHeader {
            layer: 10,
            clock_hz: 60,
            fuel: 4096,
            max_wait: 600.0,
        }
    );
    assert!(got.muscle_groups.is_empty());
    assert!(got.vars_f.is_empty());
    assert!(got.vars_i.is_empty());
    assert_eq!(got.rig, None, "未给出 `.rig` 时由调用方决定 manifest");
}

/// §2：「`.text` 只是可选标记行」。
#[test]
fn control_text_marker_is_optional() {
    let src = text(&[".layer 1", ".clock hz = 1", ".fuel 1", ".max_wait 1.0"]);
    let got = parse_ok(&src);
    assert_eq!(got.fixed.layer, 1);
}

/// 只给一部分声明时，其余为空。
#[test]
fn control_partial_declarations_parse() {
    let src = text(&[
        ".layer 1",
        ".clock hz = 1",
        ".fuel 1",
        ".max_wait 1.0",
        ".group brow = brow_l",
        ".var_f phase = 0.0",
        ".text",
    ]);
    let got = parse_ok(&src);
    assert_eq!(
        got.muscle_groups,
        vec![GroupDef {
            name: SmolStr::new("brow"),
            members: vec![SmolStr::new("brow_l")],
        }]
    );
    assert_eq!(got.vars_f, vec![(SmolStr::new("phase"), 0.0)]);
    assert!(got.vars_i.is_empty());
}

/// `group` 只有一个成员时不应被当成语法错误（`.group` 的成员表是 1..N）。
#[test]
fn control_group_may_have_a_single_member() {
    let src = text(&[
        ".layer 1",
        ".clock hz = 1",
        ".fuel 1",
        ".max_wait 1.0",
        ".group solo = brow_l",
        ".text",
    ]);
    let got = parse_ok(&src);
    assert_eq!(
        got.muscle_groups,
        vec![GroupDef {
            name: SmolStr::new("solo"),
            members: vec![SmolStr::new("brow_l")],
        }]
    );
}

/// §7 的完整例子就是用空行分隔小节的。
#[test]
fn control_blank_lines_between_directives_are_allowed() {
    let src = "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait 600.0

.group brows = brow_l

.text
";
    let got = parse_ok(src);
    assert_eq!(got.muscle_groups.len(), 1);
}

/// §2 行结构：`line ::= ... comment?`，注释是行尾的一部分。
#[test]
fn control_trailing_comments_are_allowed() {
    let src = "\
.layer 10          # 层号
.clock hz = 60     # tick 率
.fuel 4096
.max_wait 600.0    # WAIT 上界
.text
";
    let got = parse_ok(src);
    assert_eq!(got.fixed.clock_hz, 60);
}

/// 整行注释（§7 例子第一行就是 `# greet.csa`）。
#[test]
fn control_comment_only_lines_are_allowed() {
    let src = "\
# greet.csa — 打招呼
.layer 10
# 每次恢复的指令预算
.clock hz = 60
.fuel 4096
.max_wait 600.0
.text
";
    let got = parse_ok(src);
    assert_eq!(got.fixed.layer, 10);
}

/// §1：「助记符/伪指令**大小写不敏感**」。
#[test]
fn control_directive_names_are_case_insensitive() {
    let src = text(&[
        ".LAYER 1",
        ".CLOCK hz = 1",
        ".FUEL 1",
        ".MAX_WAIT 1.0",
        ".TEXT",
    ]);
    let got = parse_ok(&src);
    assert_eq!(got.fixed.layer, 1);
}

// ---------------------------------------------------------------------------
// 3a. 非法：必填标量缺失
// ---------------------------------------------------------------------------

#[test]
fn missing_required_scalars_are_rejected() {
    for (index, field) in [
        (0usize, "layer"),
        (1, "clock_hz"),
        (2, "fuel"),
        (3, "max_wait"),
    ] {
        let src = base_missing(index);
        let errors = parse_errors(&src);
        assert_eq!(errors.len(), 1, "for missing {field}: {errors:?}");
        assert!(
            matches!(&errors[0].kind, ParseErrorKind::MissingField(name) if *name == field),
            "for missing {field}: {errors:?}"
        );
        // The declaration section ends at the `.text` marker; that is where a
        // missing required field is reported.
        assert_eq!(errors[0].location.line, 4, "missing {field}");
        assert_eq!(errors[0].location.column, 1, "missing {field}");
    }
}

// ---------------------------------------------------------------------------
// 3a'. 非法：必填标量重复
// ---------------------------------------------------------------------------

#[test]
fn duplicate_required_scalars_are_rejected() {
    for (line, field) in [
        (".layer 11", "layer"),
        (".clock hz = 1", "clock_hz"),
        (".fuel 1", "fuel"),
        (".max_wait 1.0", "max_wait"),
    ] {
        let src = base_with(line);
        let errors = parse_errors(&src);
        assert_eq!(errors.len(), 1, "for duplicate {field}: {errors:?}");
        assert!(
            matches!(&errors[0].kind, ParseErrorKind::DuplicateField(name) if *name == field),
            "for duplicate {field}: {errors:?}"
        );
        // `base_with` inserts the duplicate just before `.text`, i.e. on line 5.
        assert_eq!(errors[0].location.line, 5, "duplicate {field}");
    }
}

#[test]
fn unknown_directive_reports_its_name_and_line() {
    let errors = parse_errors(&base_with(".wobble 1"));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        matches!(&errors[0].kind, ParseErrorKind::UnknownDirective(name) if name == "wobble"),
        "{errors:?}"
    );
    assert_eq!(errors[0].location.line, 5);
    assert_eq!(errors[0].location.column, 1);
}

#[test]
fn a_bad_field_value_names_the_directive() {
    let errors = parse_errors(&base_with(".layer abc"));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        matches!(
            &errors[0].kind,
            ParseErrorKind::InvalidFieldValue {
                directive: "layer",
                ..
            }
        ),
        "{errors:?}"
    );
}

#[test]
fn a_missing_directive_name_is_reported() {
    let errors = parse_errors(&base_with("."));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        matches!(&errors[0].kind, ParseErrorKind::MissingDirectiveName { .. }),
        "{errors:?}"
    );
    assert_eq!(errors[0].location.line, 5);
}

#[test]
fn a_dangling_group_comma_is_rejected() {
    let errors = parse_errors(&base_with(".group g = a,"));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        matches!(
            &errors[0].kind,
            ParseErrorKind::MissingFieldValue {
                directive: "group",
                ..
            }
        ),
        "{errors:?}"
    );
}

#[test]
fn independent_header_errors_are_all_reported() {
    let src = text(&[
        ".layer 10",
        ".clock hz = 60",
        ".fuel 4096",
        ".max_wait 600.0",
        ".wobble 1",
        ".group g = a",
        ".hey 2",
        ".text",
    ]);
    let errors = parse_errors(&src);
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(errors[0].location.line, 5);
    assert_eq!(errors[1].location.line, 7);
}

// ---------------------------------------------------------------------------
// 3b. 非法：可选字段类型不匹配
// ---------------------------------------------------------------------------

/// 每一条都是「基线 + 一行类型不对的伪指令」。左侧期望被拒绝，右侧说明原因。
const TYPE_MISMATCHES: &[(&str, &str, KindMatcher)] = &[
    (".layer abc", "layer 是 u16，给了标识符", invalid_value),
    (".layer -1", "u16 不接受负号", invalid_value),
    (".layer 70000", "u16 上溢", invalid_value),
    (".clock hz = 1.5", "hz 是 u32，给了浮点", invalid_value),
    (".clock hz = abc", "hz 是 u32，给了标识符", invalid_value),
    (".fuel 1.5", "fuel 是 u32，给了浮点", invalid_value),
    (
        ".max_wait abc",
        "max_wait 是 f32，给了标识符",
        invalid_value,
    ),
    (
        ".var_i state = 1.5",
        "var_i 是 i32，给了浮点",
        invalid_value,
    ),
    (
        ".var_i state = abc",
        "var_i 是 i32，给了标识符",
        invalid_value,
    ),
    (
        ".var_f phase = abc",
        "var_f 是 f32，给了标识符",
        invalid_value,
    ),
];

#[test]
fn optional_field_type_mismatches_are_rejected() {
    for (line, why, expected) in TYPE_MISMATCHES {
        assert_rejected(&base_with(line), why, *expected);
    }
}

/// 缺 `=`、缺字段名一类"记号不对"的写法。
const MALFORMED_MARKERS: &[(&str, &str, KindMatcher)] = &[
    (".clock = 60", "缺 hz 字段名", invalid_value),
    (".clock hz 60", "缺 =", invalid_value),
    (".clock 60", "既缺 hz 也缺 =", invalid_value),
    (".var_f phase 1.0", "缺 =", invalid_value),
    (".var_i state 0", "缺 =", invalid_value),
    (".group brows brow_l, brow_r", "缺 =", invalid_value),
];

#[test]
fn malformed_markers_are_rejected() {
    for (line, why, expected) in MALFORMED_MARKERS {
        assert_rejected(&base_with(line), why, *expected);
    }
}

// ---------------------------------------------------------------------------
// 3c. 非法：拼写错误
// ---------------------------------------------------------------------------

const TYPOS: &[(&str, &str, KindMatcher)] = &[
    (".layr 10", "layer 拼错", unknown_directive),
    (".laye 10", "layer 拼错", unknown_directive),
    (".clockk hz = 60", "clock 拼错", unknown_directive),
    (".clock hr = 60", "字段名 hz 拼错", invalid_value),
    (".fuell 4096", "fuel 拼错", unknown_directive),
    (".max_waitt 1.0", "max_wait 拼错", unknown_directive),
    (".muscel brow_l", "muscle 拼错", unknown_directive),
    (".sensorf smile", "sensor_f 拼错", unknown_directive),
    (".sensor_f_ smile", "sensor_f 拼错", unknown_directive),
    (".varr_i state = 0", "var_i 拼错", unknown_directive),
    (".textt", "text 拼错", unknown_directive),
];

#[test]
fn typos_are_rejected() {
    for (line, why, expected) in TYPOS {
        assert_rejected(&base_with(line), why, *expected);
    }
}

/// 未知伪指令必须是错误，而且不能被"当成没看见"跳过。
#[test]
fn unknown_directive_is_rejected() {
    assert_rejected(
        &base_with(".wobble 1"),
        "unknown directive",
        unknown_directive,
    );
}

/// §3.2：肌肉与可观测量已由 manifest 声明，旧伪指令就此消失 —— 它们没有被留成
/// 别名或迁移特例，所以报的就是"不认识的伪指令"。
#[test]
fn the_old_declarations_are_unknown_directives() {
    for (line, name) in [
        (".muscle brow_l", "muscle"),
        (".sensor_f smile", "sensor_f"),
        (".sensor_i gaze_lock", "sensor_i"),
        (".MUSCLE brow_l", "MUSCLE"),
    ] {
        let errors = parse_errors(&base_with(line));
        assert_eq!(errors.len(), 1, "for {line}: {errors:?}");
        assert!(
            matches!(&errors[0].kind, ParseErrorKind::UnknownDirective(found) if found == name),
            "for {line}: {errors:?}"
        );
        assert_eq!(errors[0].location.line, 5, "for {line}");
        assert_eq!(errors[0].location.column, 1, "for {line}");
    }
}

// ---------------------------------------------------------------------------
// 边界
// ---------------------------------------------------------------------------

/// `.text` 之后的内容不属于头段（§2：声明型伪指令必须在第一条指令之前）。
/// 当前实现 `break` 掉剩余行，所以指令行不会被当伪指令解析 —— 这条断言把这个性质钉住。
#[test]
fn content_after_text_marker_is_not_part_of_header() {
    let src = "\
.layer 10
.clock hz = 60
.fuel 4096
.max_wait 600.0
.text
    FMOVI f1, 1.0
    HALT
";
    let got = parse_ok(src);
    assert_eq!(got.fixed.layer, 10);
}

// ---------------------------------------------------------------------------
// `.rig`：清单路径（`docs/asm.md` §3.2）
// ---------------------------------------------------------------------------

/// 只有 `\\` 和 `\"` 是转义；其余反斜杠必须原样写两遍。
#[test]
fn a_rig_path_is_a_quoted_string_with_two_escapes() {
    let src = text(&[
        ".layer 0",
        ".clock hz = 60",
        ".fuel 1",
        ".max_wait 1.0",
        ".rig \"rigs\\\\a\\\"b.rig.json\"",
        ".text",
    ]);
    let got = parse_ok(&src);
    assert_eq!(got.rig.as_deref(), Some("rigs\\a\"b.rig.json"));
}

#[test]
fn a_second_rig_directive_is_rejected() {
    let src = text(&[
        ".layer 0",
        ".clock hz = 60",
        ".fuel 1",
        ".max_wait 1.0",
        ".rig \"a.rig.json\"",
        ".rig \"b.rig.json\"",
        ".text",
    ]);
    let errors = parse_errors(&src);
    assert!(matches!(
        errors[0].kind,
        ParseErrorKind::DuplicateField("rig")
    ));
}

#[test]
fn an_unterminated_rig_path_is_rejected() {
    let src = text(&[
        ".layer 0",
        ".clock hz = 60",
        ".fuel 1",
        ".max_wait 1.0",
        ".rig \"a.rig.json",
        ".text",
    ]);
    assert_rejected(&src, "an unterminated string literal", |kind| {
        matches!(kind, ParseErrorKind::UnterminatedString)
    });
}

// ---------------------------------------------------------------------------
// `.description`：作者说明（`docs/asm.md` §3.4）
// ---------------------------------------------------------------------------

/// §3.4：值是各行内容按 Zig 语义用 `\n` 连接，末尾不追加换行。
#[test]
fn a_description_is_collected_without_a_trailing_newline() {
    let src = text(&[
        ".layer 10",
        ".clock hz = 60",
        ".fuel 4096",
        ".max_wait 600.0",
        ".description",
        r"    \\抬眉 → 微笑",
        r"    \\hold, then reset.",
        ".text",
    ]);
    let got = parse_ok(&src);
    assert_eq!(
        got.description.as_deref(),
        Some("抬眉 → 微笑\nhold, then reset.")
    );
}

/// `\\` 之后是原文：空格、`#`、反斜杠都不特殊；单独 `\\` 是空内容行。
#[test]
fn a_description_preserves_its_content_verbatim() {
    let src = text(&[
        ".layer 10",
        ".clock hz = 60",
        ".fuel 4096",
        ".max_wait 600.0",
        ".description",
        r"    \\  leading and trailing  ",
        r"    \\# this is content, not a comment",
        r"    \\back\\slash",
        r"    \\",
        ".text",
    ]);
    let got = parse_ok(&src);
    assert_eq!(
        got.description.as_deref(),
        Some("  leading and trailing  \n# this is content, not a comment\nback\\\\slash\n")
    );
}

/// §3.4：所有 `\\` 必须对齐；第一行决定基准列，错误定位到越界行的 `\\`。
#[test]
fn a_description_requires_aligned_markers() {
    let src = text(&[
        ".layer 10",
        ".clock hz = 60",
        ".fuel 4096",
        ".max_wait 600.0",
        ".description",
        r"    \\first",
        r"  \\second",
        ".text",
    ]);
    let errors = parse_errors(&src);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        matches!(
            &errors[0].kind,
            ParseErrorKind::DescriptionMisaligned {
                expected: 5,
                found: 3
            }
        ),
        "{errors:?}"
    );
    assert_eq!(errors[0].location, Location::new(7, 3));
}

#[test]
fn a_second_description_is_rejected() {
    let src = text(&[
        ".layer 10",
        ".clock hz = 60",
        ".fuel 4096",
        ".max_wait 600.0",
        ".description",
        r"    \\first",
        ".description",
        r"    \\second",
        ".text",
    ]);
    let errors = parse_errors(&src);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        matches!(
            errors[0].kind,
            ParseErrorKind::DuplicateField("description")
        ),
        "{errors:?}"
    );
    assert_eq!(errors[0].location.line, 7);
}

/// `.description` 只接受多行字符串，不接受行内值。
#[test]
fn an_inline_description_is_rejected() {
    for line in [".description hello", ".description \"hello\""] {
        let errors = parse_errors(&base_with(line));
        assert_eq!(errors.len(), 1, "for {line}: {errors:?}");
        assert!(
            matches!(
                &errors[0].kind,
                ParseErrorKind::InvalidFieldValue {
                    directive: "description",
                    ..
                }
            ),
            "for {line}: {errors:?}"
        );
        assert_eq!(errors[0].location.line, 5, "for {line}");
    }
}

#[test]
fn a_description_without_any_marker_line_is_rejected() {
    let src = text(&[
        ".layer 10",
        ".clock hz = 60",
        ".fuel 4096",
        ".max_wait 600.0",
        ".description",
        ".text",
    ]);
    let errors = parse_errors(&src);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        matches!(errors[0].kind, ParseErrorKind::MissingDescription),
        "{errors:?}"
    );
    assert_eq!(errors[0].location.line, 5);
}

/// 块在第一个非 `\\` 行结束，下一行继续按声明解析。
#[test]
fn a_description_block_ends_at_the_next_declaration() {
    let src = text(&[
        ".layer 10",
        ".clock hz = 60",
        ".fuel 4096",
        ".max_wait 600.0",
        ".description",
        r"    \\hello",
        ".var_i state = 0",
        ".text",
    ]);
    let got = parse_ok(&src);
    assert_eq!(got.description.as_deref(), Some("hello"));
    assert_eq!(got.vars_i, vec![(SmolStr::new("state"), 0)]);
}

/// 块结束后，`header()` 的共享游标停在第一条指令上。
#[test]
fn a_description_block_leaves_the_next_instruction_in_the_cursor() {
    let src = text(&[
        ".layer 10",
        ".clock hz = 60",
        ".fuel 4096",
        ".max_wait 600.0",
        ".description",
        r"    \\hello",
        "HALT",
    ]);
    let mut cursor = src.as_str();
    let got = header(&mut cursor).expect("the header should parse");
    assert_eq!(got.description.as_deref(), Some("hello"));
    assert_eq!(cursor, "HALT\n");
}

/// 空行会结束块；空内容必须写成单独的 `\\`（Zig 风格）。
#[test]
fn a_blank_line_ends_the_description_block() {
    let src = text(&[
        ".layer 10",
        ".clock hz = 60",
        ".fuel 4096",
        ".max_wait 600.0",
        ".description",
        r"    \\first",
        "",
        r"    \\second",
        ".text",
    ]);
    let mut cursor = src.as_str();
    let got = header(&mut cursor).expect("the header should parse");
    assert_eq!(got.description.as_deref(), Some("first"));
    assert_eq!(cursor, "    \\\\second\n.text\n");
}

#[test]
fn a_description_accepts_crlf_line_endings() {
    let src = ".layer 10\r\n.clock hz = 60\r\n.fuel 4096\r\n.max_wait 600.0\r\n.description\r\n    \\\\first\r\n    \\\\second\r\n.text\r\n";
    let got = parse_ok(src);
    assert_eq!(got.description.as_deref(), Some("first\nsecond"));
}

#[test]
fn a_description_at_eof_without_a_final_newline_parses() {
    let src = ".layer 1\n.clock hz = 1\n.fuel 1\n.max_wait 1.0\n.description\n    \\\\hello";
    let got = parse_ok(src);
    assert_eq!(got.description.as_deref(), Some("hello"));
}

#[test]
fn a_description_directive_may_have_a_trailing_comment() {
    let src = text(&[
        ".layer 10",
        ".clock hz = 60",
        ".fuel 4096",
        ".max_wait 600.0",
        ".description # author note",
        r"    \\hello",
        ".text",
    ]);
    let got = parse_ok(&src);
    assert_eq!(got.description.as_deref(), Some("hello"));
}

/// 声明顺序自由：`.description` 可以出现在四个标量之前。
#[test]
fn a_description_may_precede_the_fixed_scalars() {
    let src = text(&[
        ".description",
        r"    \\hello",
        ".layer 1",
        ".clock hz = 1",
        ".fuel 1",
        ".max_wait 1.0",
        ".text",
    ]);
    let got = parse_ok(&src);
    assert_eq!(got.description.as_deref(), Some("hello"));
    assert_eq!(got.fixed.layer, 1);
}

#[test]
fn a_description_marker_may_be_indented_with_tabs() {
    let src = text(&[
        ".layer 1",
        ".clock hz = 1",
        ".fuel 1",
        ".max_wait 1.0",
        ".description",
        "\t\\\\first",
        "\t\\\\second",
        ".text",
    ]);
    let got = parse_ok(&src);
    assert_eq!(got.description.as_deref(), Some("first\nsecond"));
}
