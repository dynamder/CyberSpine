//! 指令段解析测试（`docs/asm.md` §1 / §2 / §4 / §5）。
//!
//! **契约。** 本文件按约定好的接口编写：
//!
//! ```ignore
//! // crates/cyberspine-asm/src/assembler/text.rs
//! pub enum AsmOperand {
//!     Reg { id: RegId, name: RegName, reg_type: RegType },
//!     ImmF32(f32),
//!     ImmI32(i32),
//!     ImmNone,
//!     Index(SmolStr),
//!     Target(SmolStr),
//! }
//! pub struct AsmInsn { pub op: OpCode, pub operands: Vec<AsmOperand> }
//! pub enum TextItem { Label(SmolStr), Insn(AsmInsn) }
//! pub fn text(input: &mut &str) -> ModalResult<Vec<TextItem>>;
//! ```
//!
//! 三个类型需要 `Debug + PartialEq`（本文件用 `assert_eq!` 比对），不需要 `Clone`。
//!
//! **本文件不测什么。** `Index` 名字是否真的存在、标签是否解析得出来、`group`
//! 的 1→N 展开、隐式 `HALT` 补齐——全属于第二遍（`docs/asm.md` §5）。
//! `text()` 只负责词法 / arity / 寄存器文件 / 立即数字面量，`Index` 与 `Target`
//! 保持符号形式。把这一层单独测，是因为它只依赖 `op.rs`，不需要 header。
//!
//! **为什么每个逆向用例都配基线。** 同 `tests/header.rs`：断言的性质是
//! "**这条规则**拒绝了它"。一个"拒绝一切"的解析器必须不能通过，所以每条逆向
//! 用例旁边都有结构几乎相同的合法输入。
//!
//! **寄存器操作数的形状。** `AsmOperand::Reg` 是结构体变体：除了 `RegId`，它还带着
//! 该位置在 `OpCode::signature()` 里的 `RegName` 与 `RegType`。所以下面的期望值用
//! `frd`/`fra`/`irb` 这类按角色命名的构造器写——名字本身就在断言"哪个寄存器文件、
//! 哪个字段"，而 `register_file_mismatch_is_rejected` 负责证明文件不匹配会被拒绝。

use cyberspine_asm::assembler::header::header;
use cyberspine_asm::assembler::text::{AsmInsn, AsmOperand, TextItem, text};
use cyberspine_vm::register::{RegId, RegType};
use cyberspine_vm::vm::op::{OpCode, RegName};
use smol_str::SmolStr;
use winnow::ModalResult;

// ---------------------------------------------------------------------------
// 脚手架
// ---------------------------------------------------------------------------

fn text_of(source: &str) -> ModalResult<Vec<TextItem>> {
    let mut cursor = source;
    text(&mut cursor)
}

#[track_caller]
fn items(source: &str) -> Vec<TextItem> {
    text_of(source)
        .unwrap_or_else(|err| panic!("期望解析成功，却被拒绝：{err:?}\n--- 输入 ---\n{source}"))
}

#[track_caller]
fn rejected(source: &str) {
    if let Ok(parsed) = text_of(source) {
        panic!("期望解析被拒绝，却解析成功：{parsed:?}\n--- 输入 ---\n{source}");
    }
}

#[track_caller]
fn one_insn(source: &str) -> AsmInsn {
    let mut got = items(source);
    assert_eq!(
        got.len(),
        1,
        "期望恰好一条指令，得到 {got:?}（输入 {source}）"
    );
    match got.remove(0) {
        TextItem::Insn(insn) => insn,
        other => panic!("期望一条指令，得到 {other:?}（输入 {source}）"),
    }
}

/// 取最后一个操作数当 f32 立即数——`FMOVI f1, x` 是 `[Reg, ImmF32]`。
#[track_caller]
fn imm_f32(source: &str) -> f32 {
    match one_insn(source).operands.last() {
        Some(AsmOperand::ImmF32(value)) => *value,
        other => panic!("期望 ImmF32 立即数，得到 {other:?}（输入 {source}）"),
    }
}

#[track_caller]
fn imm_i32(source: &str) -> i32 {
    match one_insn(source).operands.last() {
        Some(AsmOperand::ImmI32(value)) => *value,
        other => panic!("期望 ImmI32 立即数，得到 {other:?}（输入 {source}）"),
    }
}

fn op(opcode: OpCode, operands: Vec<AsmOperand>) -> AsmInsn {
    AsmInsn {
        op: opcode,
        operands,
    }
}

/// 按 `OpCode::signature()` 的角色构造寄存器操作数：`f`/`i` 选寄存器文件，
/// `rd`/`ra`/`rb`/`rc` 选字段位置。
fn reg(index: u8, name: RegName, reg_type: RegType) -> AsmOperand {
    AsmOperand::new_reg(
        RegId::new(index).expect("测试用的寄存器号必须 < 64"),
        name,
        reg_type,
    )
}

fn frd(index: u8) -> AsmOperand {
    reg(index, RegName::Rd, RegType::Float)
}

fn fra(index: u8) -> AsmOperand {
    reg(index, RegName::Ra, RegType::Float)
}

fn frb(index: u8) -> AsmOperand {
    reg(index, RegName::Rb, RegType::Float)
}

fn frc(index: u8) -> AsmOperand {
    reg(index, RegName::Rc, RegType::Float)
}

fn ird(index: u8) -> AsmOperand {
    reg(index, RegName::Rd, RegType::Int)
}

fn ira(index: u8) -> AsmOperand {
    reg(index, RegName::Ra, RegType::Int)
}

fn irb(index: u8) -> AsmOperand {
    reg(index, RegName::Rb, RegType::Int)
}

fn idx(name: &str) -> AsmOperand {
    AsmOperand::Index(SmolStr::new(name))
}

fn tgt(name: &str) -> AsmOperand {
    AsmOperand::Target(SmolStr::new(name))
}

fn label(name: &str) -> TextItem {
    TextItem::Label(SmolStr::new(name))
}

fn insn(opcode: OpCode, operands: Vec<AsmOperand>) -> TextItem {
    TextItem::Insn(op(opcode, operands))
}

/// 拼一份以 `\n` 结尾的源码——磁盘上真实文件的形状。
fn src(lines: &[&str]) -> String {
    let mut joined = lines.join("\n");
    joined.push('\n');
    joined
}

// ---------------------------------------------------------------------------
// 1. 零操作数指令
// ---------------------------------------------------------------------------

#[test]
fn zero_operand_instructions_parse() {
    for (line, expected) in [
        ("YIELD", OpCode::Yield),
        ("HALT", OpCode::Halt),
        ("NOP", OpCode::Nop),
        ("FLUSH", OpCode::Flush),
    ] {
        assert_eq!(items(line), vec![insn(expected, vec![])], "解析 {line}");
    }
}

// ---------------------------------------------------------------------------
// 2. 操作数顺序 = signature 顺序（R1）
// ---------------------------------------------------------------------------

/// `isa.md` §4.1 R1 的推论：分支目标在最后，存储/激发目标在最前。
/// 这是"照 sequence 走"最容易写反的地方——`FEXCITE mouth, f1` 的 imm 在**前**，
/// `FSENSE f5, brow` 的 imm 在**后**。
#[test]
fn operands_follow_signature_order() {
    // imm 在最前
    assert_eq!(
        items("FEXCITE mouth_open, f1"),
        vec![insn(OpCode::Fexcite, vec![idx("mouth_open"), fra(1)])]
    );
    assert_eq!(
        items("FVSET phase, f1"),
        vec![insn(OpCode::Fvset, vec![idx("phase"), fra(1)])]
    );
    assert_eq!(
        items("IVSET state, i1"),
        vec![insn(OpCode::Ivset, vec![idx("state"), ira(1)])]
    );

    // imm 在最后
    assert_eq!(
        items("FSENSE f5, brow_raise"),
        vec![insn(OpCode::Fsense, vec![frd(5), idx("brow_raise")])]
    );
    assert_eq!(
        items("FVGET f2, phase"),
        vec![insn(OpCode::Fvget, vec![frd(2), idx("phase")])]
    );

    // 分支目标在最后
    assert_eq!(
        items("FGE f1, f2, L_ok"),
        vec![insn(OpCode::Fge, vec![fra(1), frb(2), tgt("L_ok")])]
    );
    assert_eq!(
        items("JMP L_loop"),
        vec![insn(OpCode::Jmp, vec![tgt("L_loop")])]
    );
    assert_eq!(
        items("IJNZ i1, L_done"),
        vec![insn(OpCode::Ijnz, vec![ira(1), tgt("L_done")])]
    );

    // 三元 / 四元
    assert_eq!(
        items("FADD f1, f2, f3"),
        vec![insn(OpCode::Fadd, vec![frd(1), fra(2), frb(3)])]
    );
    assert_eq!(
        items("FMADD f1, f2, f3, f4"),
        vec![insn(OpCode::Fmadd, vec![frd(1), fra(2), frb(3), frc(4)])]
    );
}

// ---------------------------------------------------------------------------
// 3. 寄存器
// ---------------------------------------------------------------------------

/// `docs/asm.md` §6.2 原文就点了名：「`FADD f1,i2,f3` 汇编期报错」。
#[test]
fn register_file_mismatch_is_rejected() {
    rejected("FADD f1, i2, f3");
    rejected("IADD i1, f2, i3");
    rejected("IMOV f1, i2");
    rejected("FMOV f1, i2");
    rejected("FEXCITE mouth_open, i1");
    rejected("FVSET phase, i1");
    rejected("IVGET f1, state");
    rejected("ITOF i1, i2"); // ITOF 是 (rd F), (ra I)
    rejected("FTOI f1, f2"); // FTOI 是 (rd I), (ra F)
}

/// 与上一条配对：ITOF / FTOI 本来就是跨寄存器文件的，必须能过。
#[test]
fn cross_file_conversions_accept_their_real_types() {
    assert_eq!(
        items("ITOF f1, i2"),
        vec![insn(OpCode::Itof, vec![frd(1), ira(2)])]
    );
    assert_eq!(
        items("FTOI i1, f2"),
        vec![insn(OpCode::Ftoi, vec![ird(1), fra(2)])]
    );
}

#[test]
fn register_indices_are_bounded_by_the_file_size() {
    rejected("FADD f1, f2, f64"); // f0..f63
    rejected("FADD f1, f2, i64");
    rejected("FADD f1, f2, f"); // 没有数字
    rejected("FADD f1, f2, f1x"); // 尾巴不是数字
    rejected("FADD f1, f2, 1"); // 缺文件字母
}

/// `isa.md` §2.2：`f0`/`i0` 硬连零，但**仍然可写**，不是非法操作数。
#[test]
fn f0_and_i0_are_legal_operands() {
    assert_eq!(
        items("FADD f0, f0, f0"),
        vec![insn(OpCode::Fadd, vec![frd(0), fra(0), frb(0)])]
    );
    assert_eq!(
        items("IADD i0, i0, i0"),
        vec![insn(OpCode::Iadd, vec![ird(0), ira(0), irb(0)])]
    );
    assert_eq!(
        items("FADD f63, f63, f63"),
        vec![insn(OpCode::Fadd, vec![frd(63), fra(63), frb(63)])]
    );
}

// ---------------------------------------------------------------------------
// 4. 立即数
// ---------------------------------------------------------------------------

/// `docs/asm.md` §4.1：十进制浮点必须用 `f32::from_str` 直读（单次正确舍入），
/// 不得先转 f64 再转 f32。所以这里拿 `f32::from_str` 的结果当基准。
#[test]
fn f32_immediates_match_f32_from_str() {
    for literal in ["1.0", "-0.25", "1e-3", "0.1", "600.0", "3.14159265358979"] {
        let line = format!("FMOVI f1, {literal}");
        let expected: f32 = literal.parse().expect("f32::from_str 必须接受这个字面量");
        assert_eq!(imm_f32(&line), expected, "字面量 {literal}");
    }
}

#[test]
fn f32_infinity_literals_are_accepted() {
    assert_eq!(imm_f32("FMOVI f1, inf"), f32::INFINITY);
    assert_eq!(imm_f32("FMOVI f1, -inf"), f32::NEG_INFINITY);
}

/// NaN 与自身不相等，`assert_eq!` 会误报，只能查 `is_nan()`。
#[test]
fn f32_nan_literal_is_accepted() {
    assert!(imm_f32("FMOVI f1, nan").is_nan(), "nan 应当解析成 NaN");
}

/// `docs/asm.md` §4.1 R2 的第二支：`0x…` **直接就是 imm 字段**（f32 位模式）。
#[test]
fn f32_immediate_accepts_a_raw_bit_pattern() {
    assert_eq!(imm_f32("FMOVI f1, 0x3F800000"), 1.0);
    assert_eq!(imm_f32("FMOVI f1, 0x00000000"), 0.0);
    assert_eq!(imm_f32("FMOVI f1, 0xC0000000"), -2.0);
    // WAIT 的 imm 也是 F32，按 §4.1 的表同样接受位模式
    assert_eq!(imm_f32("WAIT 0x3F800000"), 1.0);
    assert_eq!(imm_f32("WAIT 2.0"), 2.0);
}

#[test]
fn f32_immediate_rejects_non_literals() {
    rejected("FMOVI f1, abc");
    rejected("FMOVI f1, 0x");
    rejected("FMOVI f1, 1.0.0");
    rejected("WAIT abc");
}

#[test]
fn i32_immediates_parse_decimally() {
    assert_eq!(imm_i32("IMOVI i1, 0"), 0);
    assert_eq!(imm_i32("IMOVI i1, 42"), 42);
    assert_eq!(imm_i32("IMOVI i1, -42"), -42);
    assert_eq!(imm_i32("IMOVI i1, 2147483647"), i32::MAX);
    assert_eq!(imm_i32("IMOVI i1, -2147483648"), i32::MIN);
}

/// `docs/asm.md` §1：「数字允许 `_` 分隔」。
#[test]
fn integer_underscore_separators_are_allowed() {
    assert_eq!(imm_i32("IMOVI i1, 1_000_000"), 1_000_000);
    assert_eq!(imm_i32("IMOVI i1, 0xFF_FF"), 0xFFFF);
    assert_eq!(imm_i32("IMOVI i1, 0b1010_1010"), 0b1010_1010);
}

/// `docs/asm.md` §1 / §4.1：`0x`/`0b`/`0o` 按 **u32 位模式**，再重解释为 i32。
/// 所以 `0xFFFFFFFF` 是 `-1`，不是溢出错误。
#[test]
fn non_decimal_integer_literals_are_bit_patterns() {
    assert_eq!(imm_i32("IMOVI i1, 0x1F"), 31);
    assert_eq!(imm_i32("IMOVI i1, 0b1010"), 10);
    assert_eq!(imm_i32("IMOVI i1, 0o17"), 15);
    assert_eq!(imm_i32("IMOVI i1, 0xFFFFFFFF"), -1);
    assert_eq!(imm_i32("IMOVI i1, 0x80000000"), i32::MIN);
}

#[test]
fn i32_immediates_reject_out_of_range() {
    rejected("IMOVI i1, 2147483648"); // i32::MAX + 1
    rejected("IMOVI i1, -2147483649"); // i32::MIN - 1
    rejected("IMOVI i1, 0x100000000"); // 33 位
    rejected("IMOVI i1, 1abc"); // 尾巴是垃圾
}

/// `docs/asm.md` §4.1 表格最后一行：「`JumpOffset` ｜ **标签**，不接受数字」。
#[test]
fn jump_offset_operand_must_be_a_label_not_a_number() {
    rejected("JMP 5");
    rejected("JMP -1");
    rejected("JMP 0");
    rejected("IJNZ i1, 0");
    rejected("FGE f1, f2, 3");
}

// ---------------------------------------------------------------------------
// 5. 标签与行结构（`docs/asm.md` §2）
// ---------------------------------------------------------------------------

#[test]
fn labels_parse_on_their_own_line() {
    assert_eq!(items("entry:"), vec![label("entry")]);
    assert_eq!(items("_l1:"), vec![label("_l1")]);
}

/// §2：「一行可堆多个标签」。
#[test]
fn several_labels_may_share_a_line() {
    assert_eq!(items("a: b: c:"), vec![label("a"), label("b"), label("c")]);
}

/// §2：「`label: instr` 允许同行」。
#[test]
fn a_label_may_share_a_line_with_an_instruction() {
    assert_eq!(
        items("L_ok: HALT"),
        vec![label("L_ok"), insn(OpCode::Halt, vec![])]
    );
    assert_eq!(
        items("L_ok: FMOVI f1, 1.0"),
        vec![
            label("L_ok"),
            insn(OpCode::Fmovi, vec![frd(1), AsmOperand::ImmF32(1.0)])
        ]
    );
}

/// §7 的完整例子把指令缩进了，所以前导空白必须被接受。
#[test]
fn instructions_may_be_indented() {
    assert_eq!(
        items("    FMOVI f1, 1.0"),
        vec![insn(OpCode::Fmovi, vec![frd(1), AsmOperand::ImmF32(1.0)])]
    );
    assert_eq!(items("\tHALT"), vec![insn(OpCode::Halt, vec![])]);
}

#[test]
fn blank_and_comment_only_lines_are_skipped() {
    let source = src(&["", "   ", "# 整行注释", "\t# 缩进注释", "HALT"]);
    let mut cursor = source.as_str();
    assert_eq!(
        text(&mut cursor).expect("应当成功"),
        vec![insn(OpCode::Halt, vec![])]
    );
}

#[test]
fn trailing_comments_are_stripped() {
    assert_eq!(items("HALT   # 收尾"), vec![insn(OpCode::Halt, vec![])]);
    assert_eq!(
        items("FMOVI f1, 1.0 # 常量"),
        vec![insn(OpCode::Fmovi, vec![frd(1), AsmOperand::ImmF32(1.0)])]
    );
}

/// §1：「助记符/伪指令**大小写不敏感**」——标签仍然大小写敏感。
#[test]
fn mnemonic_lookup_is_case_insensitive() {
    assert_eq!(items("fadd f1, f2, f3"), items("FADD f1, f2, f3"));
    assert_eq!(items("Fadd f1, f2, f3"), items("FADD f1, f2, f3"));
    assert_eq!(items("Halt"), items("HALT"));
    assert_eq!(items("Flush"), items("FLUSH"));
}

#[test]
fn commas_allow_optional_surrounding_space() {
    assert_eq!(items("FADD f1,f2,f3"), items("FADD f1, f2, f3"));
    assert_eq!(items("FADD f1 , f2 ,f3"), items("FADD f1, f2, f3"));
    assert_eq!(items("FADD   f1,  f2,  f3  "), items("FADD f1, f2, f3"));
}

#[test]
fn unknown_mnemonic_is_rejected() {
    rejected("FADDD f1, f2, f3");
    rejected("HLT");
    rejected("MOV f1, f2");
    rejected("fexcite2 mouth_open, f1");
}

#[test]
fn arity_must_match_the_signature() {
    rejected("FADD f1, f2"); // 少一个
    rejected("FADD f1, f2, f3, f4"); // 多一个
    rejected("HALT f1"); // 零操作数
    rejected("FLUSH f1");
    rejected("YIELD 1.0");
    rejected("JMP"); // 缺标签
    rejected("FMOVI f1"); // 缺立即数
}

#[test]
fn trailing_garbage_on_a_line_is_rejected() {
    rejected("HALT extra");
    rejected("FMOVI f1, 1.0 1.0");
}

/// §1：`fN`/`iN` 是保留形状；§6 第 8 条：`dt`/`tick` 不得用作标签。
#[test]
fn reserved_shapes_cannot_be_labels() {
    rejected("f0:");
    rejected("i63:");
    rejected("f1: HALT");
    rejected("dt:");
    rejected("tick:");
    rejected("dt: HALT");
}

/// `docs/asm.md` §2：声明型伪指令属于头段，指令段里出现必须是错误。
#[test]
fn header_directives_are_not_instructions() {
    rejected(".layer 10");
    rejected(".muscle brow_l");
}

// ---------------------------------------------------------------------------
// 6. `Index` 保持符号形式
// ---------------------------------------------------------------------------

/// `text()` **不查表**。名字存在与否是第二遍（§3.3 表序 / §6.4）的事，
/// 所以一个从未声明的名字在这一层必须解析成功。
#[test]
fn index_operands_stay_symbolic_and_unresolved() {
    assert_eq!(
        items("FEXCITE not_declared_anywhere, f1"),
        vec![insn(
            OpCode::Fexcite,
            vec![idx("not_declared_anywhere"), fra(1)]
        )]
    );
    assert_eq!(
        items("FSENSE f5, also_not_declared"),
        vec![insn(OpCode::Fsense, vec![frd(5), idx("also_not_declared")])]
    );
}

#[test]
fn index_operand_must_be_an_identifier() {
    rejected("FSENSE f5, 123");
    rejected("FSENSE f5, 1.5");
    rejected("FEXCITE 123, f1");
    rejected("FSENSE f5, 1a"); // 数字开头
}

#[test]
fn index_names_follow_identifier_rules() {
    assert_eq!(
        items("FSENSE f5, _a1"),
        vec![insn(OpCode::Fsense, vec![frd(5), idx("_a1")])]
    );
    assert_eq!(
        items("FSENSE f5, a_1_b2"),
        vec![insn(OpCode::Fsense, vec![frd(5), idx("a_1_b2")])]
    );
}

// ---------------------------------------------------------------------------
// 7. header 与 text 共享游标
// ---------------------------------------------------------------------------

/// 衔接契约（形态 (a)）：`header` 结束时把 `*input` 推到 `.text` 之后，
/// `text` 从那里接着走。这是 `docs/asm.md` §7 完整例子的头半段。
#[test]
fn header_and_text_share_one_cursor() {
    let source = src(&[
        ".layer 10",
        ".clock hz = 60",
        ".fuel 4096",
        ".max_wait 600.0",
        ".muscle brow_l",
        ".text",
        "entry:",
        "    FMOVI f1, 1.0",
        "    FEXCITE brow_l, f1",
        "    FLUSH",
        "L_done:",
        "    HALT",
    ]);

    let mut cursor = source.as_str();
    let parsed_header = header(&mut cursor).expect("header 阶段应当成功");
    assert_eq!(parsed_header.fixed.layer, 10);
    assert_eq!(
        cursor,
        "entry:\n    FMOVI f1, 1.0\n    FEXCITE brow_l, f1\n    FLUSH\nL_done:\n    HALT\n"
    );

    assert_eq!(
        text(&mut cursor).expect("text 阶段应当成功"),
        vec![
            label("entry"),
            insn(OpCode::Fmovi, vec![frd(1), AsmOperand::ImmF32(1.0)]),
            insn(OpCode::Fexcite, vec![idx("brow_l"), fra(1)]),
            insn(OpCode::Flush, vec![]),
            label("L_done"),
            insn(OpCode::Halt, vec![]),
        ]
    );
}

/// §2：「`.text` 只是可选标记行」。没有它时，头段必须在**第一条指令处**停下——
/// 判据是该行不是 `.` 开头的已识别伪指令。
#[test]
fn text_marker_is_optional_but_header_still_ends_at_the_first_instruction() {
    let source = src(&[
        ".layer 1",
        ".clock hz = 1",
        ".fuel 1",
        ".max_wait 1.0",
        "entry:",
        "HALT",
    ]);

    let mut cursor = source.as_str();
    header(&mut cursor).expect("没有 .text 时 header 也应当成功");
    assert_eq!(cursor, "entry:\nHALT\n");
    assert_eq!(
        text(&mut cursor).expect("text 应当成功"),
        vec![label("entry"), insn(OpCode::Halt, vec![])]
    );
}

/// §5 的隐式 `HALT` 补齐是**发射阶段**的事，不是 `text()` 的事：
/// 只有 `.text` 而没有指令时，指令段就是空的。
#[test]
fn empty_instruction_section_yields_no_items() {
    let source = src(&[
        ".layer 1",
        ".clock hz = 1",
        ".fuel 1",
        ".max_wait 1.0",
        ".text",
    ]);

    let mut cursor = source.as_str();
    header(&mut cursor).expect("header 应当成功");
    assert_eq!(cursor, "");
    assert_eq!(text(&mut cursor).expect("text 应当成功"), vec![]);
}

// ---------------------------------------------------------------------------
// 8. 存疑：规范没写清的，先挂起来，不制造假红
// ---------------------------------------------------------------------------

/// §1 只说「数字允许 `_` 分隔（`1_000_000`）」，例子是整数；浮点是否含 `_`
/// 没定义。winnow 的 `float` 不接受 `_`，所以这一条等了才决定要不要自己写 lexer。
#[test]
#[ignore = "docs/asm.md §1 未定义浮点是否允许 _ 分隔符"]
fn float_underscore_separators_are_allowed() {
    assert_eq!(imm_f32("FMOVI f1, 1_000.5"), 1000.5);
}

/// §1/§2 没提 CRLF。注意 winnow 的 `space0` 只吃空格和制表符
/// （`AsChar::is_space` 的文档原文是 "Tests that self is ASCII space or tab"），
/// 所以 `\r` 会留在行尾把 `eof` 顶掉。
#[test]
#[ignore = "docs/asm.md §1/§2 未规定 CRLF 行尾是否接受"]
fn crlf_line_endings_are_accepted() {
    let source = ".layer 1\r\n.clock hz = 1\r\n.fuel 1\r\n.max_wait 1.0\r\n.text\r\nHALT\r\n";
    let mut cursor = source;
    header(&mut cursor).expect("header 应当成功");
    assert_eq!(
        text(&mut cursor).expect("text 应当成功"),
        vec![insn(OpCode::Halt, vec![])]
    );
}
