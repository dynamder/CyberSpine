//! `OpCode` 的助记符表测试（`docs/isa.md` §4）。
//!
//! **契约。** 本文件按约定好的接口编写，实现尚未存在：
//!
//! ```ignore
//! // crates/cyberspine-vm/src/vm/op.rs，在 isa_table! 内部生成
//! impl FromStr for OpCode { type Err = /* 待定 */; ... }   // 大小写不敏感
//! ```
//!
//! 所以 `cargo test --test opcode` 在 `FromStr` 落地前不会编译。
//!
//! **为什么不绑定 `Err` 类型。** 下面所有逆向断言只写 `is_err()`，正向只取
//! `Ok` 的值，`Err` 的变体名和载荷一律不碰——错误类型是另一个待定的设计决定，
//! 不该让这份测试跟着它变。
//!
//! **这份测试在保护什么。** `isa_table!` 是助记符的**唯一**来源（`docs/asm.md`
//! §9：asm 不得手抄任何清单）。唯一来源要成立，需要两个性质：
//!   1. 表里**没有两条指令共用同一个助记符**（否则 `FromStr` 有歧义，
//!      而且失败是静默的——先撞上的那条永远赢）；
//!   2. 字节 ↔ 助记符 ↔ 变体三者**互相闭合**，不存在"能编码却查不到名字"
//!      或"解码不回来"的洞。
//! 这两条目前都没有被覆盖：`tests/handmade.rs` 只查了 golden 文件覆盖了每个
//! *字节*，`mnemonic()` 从来没被验证过。

use std::str::FromStr;

use cyberspine_vm::vm::op::OpCode;

// ---------------------------------------------------------------------------
// 脚手架
// ---------------------------------------------------------------------------

#[track_caller]
fn parse(name: &str) -> OpCode {
    OpCode::from_str(name).unwrap_or_else(|_| panic!("助记符 {name:?} 应当能解析成 OpCode"))
}

#[track_caller]
fn rejected(name: &str) {
    if let Ok(parsed) = OpCode::from_str(name) {
        panic!("助记符 {name:?} 本不该解析成功，却得到 {parsed:?}");
    }
}

// ---------------------------------------------------------------------------
// 1. 唯一性：FromStr 无歧义的前提
// ---------------------------------------------------------------------------

/// 两条指令用同一个助记符 ⇒ `FromStr` 必然只能返回其中一个，
/// 另一个变得**永远写不出来**，而且没有任何报错。
///
/// 大小写不敏感地比：`FADD` 和 `fadd` 是同一条，不算冲突；
/// 但 `FLT` 和 `Flt` 分属两条指令才是真正的冲突。
#[test]
fn mnemonics_are_unique_case_insensitively() {
    let mut seen: Vec<&'static str> = Vec::new();
    for op in OpCode::ALL {
        let mnemonic = op.mnemonic();
        let clashes = seen.iter().any(|other| other.eq_ignore_ascii_case(mnemonic));
        assert!(
            !clashes,
            "助记符 {mnemonic:?}（{op:?}）与表里已有的条目冲突（大小写不敏感），\
             FromStr 会有歧义：先撞上的那条永远赢",
        );
        seen.push(mnemonic);
    }
}

/// `handmade.rs` 查的是"golden 文件覆盖了每一条指令的字节"；
/// 这条查的是**表自身**的字节不重复。两者互补。
#[test]
fn opcode_bytes_are_unique() {
    let mut bytes: Vec<u8> = OpCode::ALL.iter().map(|op| op.byte()).collect();
    bytes.sort_unstable();
    let total = bytes.len();
    bytes.dedup();
    assert_eq!(bytes.len(), total, "有两个 OpCode 用了同一个编码字节");
}

// ---------------------------------------------------------------------------
// 2. 规范拼写
// ---------------------------------------------------------------------------

/// 助记符会出现在错误信息、反汇编输出和调试信息里，所以规范拼写本身是接口的一部分。
#[test]
fn canonical_mnemonics_are_non_empty_uppercase_identifiers() {
    for op in OpCode::ALL {
        let mnemonic = op.mnemonic();
        assert!(!mnemonic.is_empty(), "{op:?} 的助记符为空");
        assert_eq!(
            mnemonic,
            mnemonic.to_ascii_uppercase(),
            "{op:?} 的规范拼写应当是大写，实际是 {mnemonic:?}",
        );
        assert!(
            mnemonic
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "{op:?} 的助记符含非标识符字符：{mnemonic:?}",
        );
    }
}

// ---------------------------------------------------------------------------
// 3. 大小写不敏感（docs/asm.md §1）
// ---------------------------------------------------------------------------

/// `docs/asm.md` §1：「助记符/伪指令**大小写不敏感**」。case 没有意义，
/// 所以规范拼写、全小写、全大写、混合大小写都必须落到同一个变体。
#[test]
fn mnemonic_lookup_ignores_case() {
    for op in OpCode::ALL {
        let canonical = op.mnemonic();
        assert_eq!(parse(canonical), *op, "规范拼写 {canonical:?}");
        assert_eq!(
            parse(&canonical.to_ascii_lowercase()),
            *op,
            "{canonical:?} 的全小写",
        );
        assert_eq!(
            parse(&canonical.to_ascii_uppercase()),
            *op,
            "{canonical:?} 的全大写",
        );
    }
}

#[test]
fn mixed_case_mnemonics_are_accepted() {
    assert_eq!(parse("FaDd"), OpCode::Fadd);
    assert_eq!(parse("fLuSh"), OpCode::Flush);
    assert_eq!(parse("iJnZ"), OpCode::Ijnz);
    assert_eq!(parse("FvSeT"), OpCode::Fvset);
    assert_eq!(parse("fExCiTe"), OpCode::Fexcite);
    assert_eq!(parse("hAlT"), OpCode::Halt);
}

// ---------------------------------------------------------------------------
// 4. 三者闭合：字节 ↔ 助记符 ↔ 变体
// ---------------------------------------------------------------------------

/// 每一条指令都必须能从自己的字节和自己的助记符回到自身。
/// 加上"字节唯一"（上面）就得到双射：不存在两条指令互相遮蔽。
#[test]
fn byte_and_mnemonic_round_trip_to_the_same_variant() {
    for op in OpCode::ALL {
        assert_eq!(
            OpCode::from_byte(op.byte()),
            Some(*op),
            "{op:?} 的 from_byte(byte()) 没有回到自身",
        );
        assert_eq!(
            parse(op.mnemonic()),
            *op,
            "{op:?} 的 from_str(mnemonic()) 没有回到自身",
        );
    }
}

/// `OpCode::ALL` 与 `from_byte` 的可解码集合必须完全一致。
/// 两侧不一致就意味着存在"能编码但解不回来"或"能解码但不在 ALL 里"的指令——
/// 前者会让 loader 拒绝自家汇编器产出的文件。
#[test]
fn all_covers_exactly_the_decodable_bytes() {
    let mut decodable: Vec<u8> = (0u8..=0xFF)
        .filter(|byte| OpCode::from_byte(*byte).is_some())
        .collect();
    let mut all: Vec<u8> = OpCode::ALL.iter().map(|op| op.byte()).collect();
    decodable.sort_unstable();
    all.sort_unstable();
    assert_eq!(
        decodable, all,
        "ALL 与 from_byte 的可解码集合不一致",
    );
}

/// `docs/binfmt.md` §9.3 I1：「保留 opcode 一律拒绝」。
/// 逐个字节查一遍 256 个值，两个方向都查。
#[test]
fn every_byte_outside_the_table_is_undefined() {
    let defined: Vec<u8> = OpCode::ALL.iter().map(|op| op.byte()).collect();
    for byte in 0u8..=0xFF {
        if defined.contains(&byte) {
            assert!(
                OpCode::from_byte(byte).is_some(),
                "{byte:#04X} 在 ALL 里，from_byte 却解不出来",
            );
        } else {
            assert!(
                OpCode::from_byte(byte).is_none(),
                "保留字节 {byte:#04X} 不应当被解码",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 5. 未知名必须被拒绝
// ---------------------------------------------------------------------------

#[test]
fn unknown_mnemonics_are_rejected() {
    rejected("");
    rejected("FADDD");
    rejected("HLT");
    rejected("MOV"); // ISA 里是 FMOV / IMOV，没有 MOV
    rejected("FADD f1"); // 操作数不属于助记符
    rejected("0x50");
    rejected("JMP\n");
}

/// 伪指令（`docs/asm.md` §3）不是指令，不能从 `OpCode::from_str` 进来——
/// `.text` / `.layer` 的解析归汇编器，不归 ISA 表。
#[test]
fn assembler_directives_are_not_opcodes() {
    rejected("LAYER");
    rejected("CLOCK");
    rejected("MUSCLE");
    rejected("TEXT");
    rejected("MAX_WAIT");
    rejected("VAR_F");
    rejected("GROUP");
}

/// 空白是**记号分隔符**，由汇编器的词法器处理，不属于助记符本身。
/// 所以带空白的输入必须是错误，而不是被 `trim` 掉悄悄接受。
#[test]
fn surrounding_whitespace_is_not_part_of_a_mnemonic() {
    rejected(" FADD");
    rejected("FADD ");
    rejected(" FADD ");
    rejected("FAD D");
    rejected("F\tADD");
}
