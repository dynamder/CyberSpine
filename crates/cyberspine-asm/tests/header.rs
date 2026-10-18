//! `header()` 的行为测试（`docs/asm.md` §1–§3）。
//!
//! **为什么每个逆向用例都配基线。** "它被拒绝了"不是要断言的性质，断言的性质是
//! "**这条规则**拒绝了它"。所以每个非法输入都构造成「合法基线 + 一处扰动」，
//! 扰动之外的每一行都保持合法；`control_*` 用例则证明基线本身能通过。
//! 否则一个"拒绝一切"的解析器会让整份逆向测试全绿 ——
//! `crates/cyberspine-vm/tests/load.rs` 开头警告的正是这个失败模式。
//!
//! **当前缺口。** `header()` 返回 `ModalResult`，也就是 `ErrMode<ContextError>`，
//! 它不携带"是哪个字段出错"的信息；`FixedHeaderError` 在调用处的 `.unwrap()`
//! 被丢弃（错误路径直接 panic，而不是返回 `Err`）。因此本文件目前只能断言
//! "被拒绝"，还无法断言"被哪条规则拒绝"。

use cyberspine_asm::assembler::header::{AsmHeader, FixedHeader, GroupDef, header};
use smol_str::SmolStr;
use winnow::ModalResult;

// ---------------------------------------------------------------------------
// 脚手架
// ---------------------------------------------------------------------------

fn parse(src: &str) -> ModalResult<AsmHeader> {
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
fn parse_rejected(src: &str) {
    if let Ok(parsed) = parse(src) {
        panic!("期望解析被拒绝，却解析成功：{parsed:?}\n--- 输入 ---\n{src}");
    }
}

/// 把若干行拼成一份以 `\n` 结尾的源码 —— 这是磁盘上真实文件的形状。
fn text(lines: &[&str]) -> String {
    let mut joined = lines.join("\n");
    joined.push('\n');
    joined
}

/// 四个必填标量 + `.text`。
///
/// 注意 `docs/asm.md` §3.1 说标量"全部可省，取缺省值"，而当前实现把缺失当错误
/// （`FixedHeaderError::Missing`）。为了让基线在两种读法下都合法，这里四个全部给出；
/// 这个规范冲突单独由 `missing_required_scalars_are_rejected` 标记。
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
    ".muscle brow_l",
    ".muscle brow_r",
    ".group brows = brow_l, brow_r",
    ".sensor_f smile",
    ".sensor_i gaze_lock",
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
        muscles: vec![SmolStr::new("brow_l"), SmolStr::new("brow_r")],
        muscle_groups: vec![GroupDef {
            name: SmolStr::new("brows"),
            members: vec![SmolStr::new("brow_l"), SmolStr::new("brow_r")],
        }],
        sensors_f: vec![SmolStr::new("smile")],
        sensors_i: vec![SmolStr::new("gaze_lock")],
        vars_f: vec![(SmolStr::new("phase"), 0.0)],
        vars_i: vec![(SmolStr::new("state"), 0)],
    }
}

#[test]
fn control_full_header_matches_expected_value() {
    let got = parse_ok(&text(FULL));
    assert_eq!(got, expected_full());
}

/// §3.3：表序按声明序。顺序是语义的一部分（名字→下标），不能被解析顺手排序。
#[test]
fn control_declaration_order_is_preserved() {
    let src = text(&[
        ".layer 1",
        ".clock hz = 1",
        ".fuel 1",
        ".max_wait 1.0",
        ".muscle c",
        ".muscle a",
        ".muscle b",
        ".var_i z = 1",
        ".var_i y = 2",
        ".text",
    ]);
    let got = parse_ok(&src);
    let names: Vec<SmolStr> = ["c", "a", "b"].into_iter().map(SmolStr::new).collect();
    assert_eq!(got.muscles, names);
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
    assert!(got.muscles.is_empty(), "未声明肌肉时不应凭空产生条目");
    assert!(got.muscle_groups.is_empty());
    assert!(got.sensors_f.is_empty());
    assert!(got.sensors_i.is_empty());
    assert!(got.vars_f.is_empty());
    assert!(got.vars_i.is_empty());
}

/// §2：「`.text` 只是可选标记行」。
#[test]
fn control_text_marker_is_optional() {
    let src = text(&[".layer 1", ".clock hz = 1", ".fuel 1", ".max_wait 1.0"]);
    let got = parse_ok(&src);
    assert_eq!(got.fixed.layer, 1);
}

/// 只给部分名字表，其余省略。
#[test]
fn control_partial_name_tables_parse() {
    let src = text(&[
        ".layer 1",
        ".clock hz = 1",
        ".fuel 1",
        ".max_wait 1.0",
        ".muscle brow_l",
        ".sensor_i gaze_lock",
        ".text",
    ]);
    let got = parse_ok(&src);
    assert_eq!(got.muscles, vec![SmolStr::new("brow_l")]);
    assert_eq!(got.sensors_i, vec![SmolStr::new("gaze_lock")]);
    assert!(got.sensors_f.is_empty());
    assert!(got.vars_f.is_empty());
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

.muscle brow_l

.group brows = brow_l

.text
";
    let got = parse_ok(src);
    assert_eq!(got.muscles, vec![SmolStr::new("brow_l")]);
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
        // 期望的错误是 FixedHeaderError::Missing(field)；当前实现会在
        // build().unwrap() 处 panic，看不到 field，所以这里只断言"被拒绝"。
        // TODO: this test will pass as long as we don't unwrap on the FixedHeaderError.
        parse_rejected(&src);
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
        // 期望 FixedHeaderError::Duplicate(field)；当前是 panic。
        // TODO: this test will pass as long as the fixed header error is not unwraped.
        parse_rejected(&src);
    }
}

// ---------------------------------------------------------------------------
// 3b. 非法：可选字段类型不匹配
// ---------------------------------------------------------------------------

/// 每一条都是「基线 + 一行类型不对的伪指令」。左侧期望被拒绝，右侧说明原因。
const TYPE_MISMATCHES: &[(&str, &str)] = &[
    (".layer abc", "layer 是 u16，给了标识符"),
    (".layer -1", "u16 不接受负号"),
    (".layer 70000", "u16 上溢"),
    (".clock hz = 1.5", "hz 是 u32，给了浮点"),
    (".clock hz = abc", "hz 是 u32，给了标识符"),
    (".fuel 1.5", "fuel 是 u32，给了浮点"),
    (".max_wait abc", "max_wait 是 f32，给了标识符"),
    (".sensor_f 123", "sensor 名必须是标识符"),
    (".sensor_i 1.5", "sensor 名必须是标识符"),
    (".muscle 123", "肌肉名必须是标识符"),
    (".var_i state = 1.5", "var_i 是 i32，给了浮点"),
    (".var_i state = abc", "var_i 是 i32，给了标识符"),
    (".var_f phase = abc", "var_f 是 f32，给了标识符"),
];

#[test]
fn optional_field_type_mismatches_are_rejected() {
    for (line, why) in TYPE_MISMATCHES {
        let src = base_with(line);
        if parse(&src).is_ok() {
            panic!("应当拒绝（{why}），却解析成功：{line}\n--- 输入 ---\n{src}");
        }
    }
}

/// 缺 `=`、缺字段名一类"记号不对"的写法。
const MALFORMED_MARKERS: &[(&str, &str)] = &[
    (".clock = 60", "缺 hz 字段名"),
    (".clock hz 60", "缺 ="),
    (".clock 60", "既缺 hz 也缺 ="),
    (".var_f phase 1.0", "缺 ="),
    (".var_i state 0", "缺 ="),
    (".group brows brow_l, brow_r", "缺 ="),
];

#[test]
fn malformed_markers_are_rejected() {
    for (line, why) in MALFORMED_MARKERS {
        let src = base_with(line);
        if parse(&src).is_ok() {
            panic!("应当拒绝（{why}），却解析成功：{line}\n--- 输入 ---\n{src}");
        }
    }
}

// ---------------------------------------------------------------------------
// 3c. 非法：拼写错误
// ---------------------------------------------------------------------------

const TYPOS: &[(&str, &str)] = &[
    (".layr 10", "layer 拼错"),
    (".laye 10", "layer 拼错"),
    (".clockk hz = 60", "clock 拼错"),
    (".clock hr = 60", "字段名 hz 拼错"),
    (".fuell 4096", "fuel 拼错"),
    (".max_waitt 1.0", "max_wait 拼错"),
    (".muscel brow_l", "muscle 拼错"),
    (".sensorf smile", "sensor_f 拼错"),
    (".sensor_f_ smile", "sensor_f 拼错"),
    (".varr_i state = 0", "var_i 拼错"),
    (".textt", "text 拼错"),
];

#[test]
fn typos_are_rejected() {
    for (line, why) in TYPOS {
        let src = base_with(line);
        if parse(&src).is_ok() {
            panic!("应当拒绝（{why}），却解析成功：{line}\n--- 输入 ---\n{src}");
        }
    }
}

/// 未知伪指令必须是错误，而且不能被"当成没看见"跳过。
#[test]
fn unknown_directive_is_rejected() {
    parse_rejected(&base_with(".wobble 1"));
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
