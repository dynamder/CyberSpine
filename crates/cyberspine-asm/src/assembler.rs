pub mod diagnostic;
pub mod header;
pub mod text;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use cyberspine_core::binfmt::image::{
    FORMAT_VERSION, FixedHeader as ImageFixedHeader, ISA_VERSION, Image, Muscle, Sensor,
    SensorSource, VarF, VarI,
};
use cyberspine_core::isa::jump::{jump_offset, jump_target};
use cyberspine_core::isa::{ImmType, Insn, OpCode, RegName};
use cyberspine_core::rig::Rig;
use smol_str::SmolStr;
use thiserror::Error;
use winnow::{Parser, combinator::trace, token::take_while};

use diagnostic::PResult;
pub use diagnostic::{AsmParseErrors, Location, ParseError, ParseErrorKind};
use header::{AsmHeader, header as header_section};
use text::{AsmInsn, AsmOperand, TextItem, text as text_section};

pub type Line<'i> = &'i str;

/// `docs/asm.md` §1 / §6.8: `fN`/`iN` is the register shape and `dt`/`tick` are
/// built-in sensor names, so none of them can be a label or a declared name.
pub(crate) fn is_reserved_name(name: &str) -> bool {
    if matches!(name, "dt" | "tick") {
        return true;
    }
    let mut chars = name.chars();
    if !matches!(chars.next(), Some('f' | 'i')) {
        return false;
    }
    let rest = chars.as_str();
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

/// `docs/asm.md` §3.2: muscles and observables come from the rig manifest, so
/// the directives that used to declare them are gone rather than unknown.
pub(crate) fn is_removed_directive(name: &str) -> bool {
    name.eq_ignore_ascii_case("muscle")
        || name.eq_ignore_ascii_case("sensor_f")
        || name.eq_ignore_ascii_case("sensor_i")
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

pub(crate) fn ident<'i>(input: &mut Line<'i>) -> PResult<&'i str> {
    trace(
        "ident",
        (
            take_while(1.., |c: char| c.is_ascii_alphabetic() || c == '_'),
            take_while(0.., |c: char| c.is_ascii_alphanumeric() || c == '_'),
        ),
    )
    .take()
    .parse_next(input)
}

pub(crate) fn comments<'i>(input: &mut Line<'i>) -> PResult<()> {
    trace(
        "comments",
        ('#', take_while(0.., |c: char| c != '\n')).void(),
    )
    .parse_next(input)
}

/// The parsed source: the declaration section plus the flat instruction stream.
#[derive(Debug, Clone)]
pub struct Asm {
    pub header: AsmHeader,
    pub text: Vec<TextItem>,
}

/// Parse a whole `.csa` source. `header` owns the cursor up to `.text` (or the
/// first instruction); `text` continues from exactly there (`docs/asm.md` §2).
///
/// Both sections are attempted even when the first one fails: each one recovers
/// at line boundaries, so one source can report every independent mistake.
pub fn parse(asm_str: &str) -> Result<Asm, AsmParseErrors> {
    let mut cursor = asm_str;
    let header = header_section(&mut cursor);

    // `text()` numbers lines from its own input, which starts *after* the
    // declaration section. Shift its errors back into file coordinates.
    let header_len = asm_str.len() - cursor.len();
    let text_line_offset = asm_str[..header_len].matches('\n').count();
    let text = text_section(&mut cursor);

    match (header, text) {
        (Ok(header), Ok(text)) => Ok(Asm { header, text }),
        (header, text) => {
            let mut errors = Vec::new();
            if let Err(mut section_errors) = header {
                errors.append(&mut section_errors.errors);
            }
            if let Err(mut section_errors) = text {
                for error in &mut section_errors.errors {
                    error.location.line += text_line_offset;
                }
                errors.append(&mut section_errors.errors);
            }
            errors.sort_by_key(|error| (error.location.line, error.location.column));
            Err(AsmParseErrors::new(errors))
        }
    }
}

/// Everything the assembler can reject without a backend (`docs/asm.md` §6).
#[derive(Debug, Error, PartialEq)]
pub enum ImageGenError {
    #[error("duplicate label `{0}`")]
    DuplicateLabel(SmolStr),
    #[error("undefined label `{0}`")]
    UndefinedLabel(SmolStr),
    #[error(
        "label `{label}` points to {index}, past the end of the code ({code_len} instructions)"
    )]
    LabelOutOfRange {
        label: SmolStr,
        index: usize,
        code_len: usize,
    },
    #[error("instruction {pc}: offset {offset} leaves the code (code_len {code_len})")]
    JumpTargetOutOfRange {
        pc: usize,
        offset: i32,
        code_len: usize,
    },
    #[error("instruction {pc}: jump distance does not fit an i32 offset")]
    JumpOffsetOverflow { pc: usize },
    #[error("malformed {0:?}: expected an index operand")]
    MissingIndexOperand(OpCode),
    #[error("`{0}` is declared more than once")]
    DuplicateName(SmolStr),
    #[error("`{0}` is a reserved name")]
    ReservedName(SmolStr),
    #[error("the rig declares `{0}`, which is not an identifier")]
    NotAnIdentifier(SmolStr),
    #[error("FEXCITE target `{0}` is neither a muscle nor a group")]
    UnknownExciteTarget(SmolStr),
    #[error("unknown float sensor `{0}`")]
    UnknownSensorF(SmolStr),
    #[error("unknown integer sensor `{0}`")]
    UnknownSensorI(SmolStr),
    #[error("unknown float variable `{0}`")]
    UnknownVarF(SmolStr),
    #[error("unknown integer variable `{0}`")]
    UnknownVarI(SmolStr),
    #[error("group `{0}` is only legal as a FEXCITE target")]
    GroupOutsideExcite(SmolStr),
    #[error("group `{group}` contains unknown muscle `{member}`")]
    UnknownGroupMember { group: SmolStr, member: SmolStr },
    #[error("group `{0}` collides with a muscle name")]
    GroupNameCollision(SmolStr),
    #[error("FEXCITE at {excite_pc} is still pending at {suspend_pc}; FLUSH is missing")]
    PendingNotFlushed { excite_pc: usize, suspend_pc: usize },
    #[error("WAIT at {pc} is not a non-negative finite duration up to max_wait")]
    InvalidWait { pc: usize, seconds: f32 },
    #[error("`hz` must be greater than zero")]
    ZeroHz,
    #[error("`max_wait` must be a non-negative finite f32")]
    InvalidMaxWait,
    #[error("the program is too large for the format's 32-bit instruction count")]
    TooManyInstructions,
}

/// Resolve names, labels and groups against `rig`, then produce the canonical
/// `Image`.
pub fn generate_image(asm: &Asm, rig: &Rig) -> Result<Image, ImageGenError> {
    check_header(&asm.header)?;
    check_rig(rig)?;

    let symbols = Symbols::build(&asm.header, rig)?;
    let layout = layout(&asm.text, &symbols)?;
    let mut code: Vec<Insn> = emit(&layout, &symbols).collect::<Result<_, _>>()?;
    debug_assert_eq!(code.len(), layout.code_len);
    append_trailing_halt(&mut code);

    if code.len() > u32::MAX as usize {
        return Err(ImageGenError::TooManyInstructions);
    }

    check_labels(&layout.labels, code.len())?;
    check_jumps(&code)?;
    check_waits(&code, asm.header.fixed.max_wait)?;
    check_pending(&code)?;

    Ok(Image {
        header: ImageFixedHeader {
            format_version: FORMAT_VERSION,
            isa_version: ISA_VERSION,
            layer: asm.header.fixed.layer,
            hz: asm.header.fixed.clock_hz,
            fuel: asm.header.fixed.fuel,
            max_wait: asm.header.fixed.max_wait,
            rig: rig.fingerprint(),
        },
        code,
        muscles: symbols.image_muscles,
        sensors_f: symbols.image_sensors_f,
        sensors_i: symbols.image_sensors_i,
        vars_f: symbols.image_vars_f,
        vars_i: symbols.image_vars_i,
    })
}

fn check_header(header: &AsmHeader) -> Result<(), ImageGenError> {
    if header.fixed.clock_hz == 0 {
        return Err(ImageGenError::ZeroHz);
    }
    let max_wait = header.fixed.max_wait;
    if !max_wait.is_finite() || max_wait < 0.0 {
        return Err(ImageGenError::InvalidMaxWait);
    }
    Ok(())
}

/// A `Rig` built in code bypasses [`cyberspine_rig`]'s checks, but its names
/// still have to produce an image the loader accepts (`docs/binfmt.md` §9.2 M1,
/// M2, M5): one identifier namespace, no built-in names, no repeats.
fn check_rig(rig: &Rig) -> Result<(), ImageGenError> {
    let mut seen = HashSet::new();
    for name in rig
        .muscles
        .iter()
        .chain(&rig.sensors_f)
        .chain(&rig.sensors_i)
    {
        let name = SmolStr::new(&**name);
        if is_reserved_name(&name) {
            return Err(ImageGenError::ReservedName(name));
        }
        if !is_identifier(&name) {
            return Err(ImageGenError::NotAnIdentifier(name));
        }
        if !seen.insert(name.clone()) {
            return Err(ImageGenError::DuplicateName(name));
        }
    }
    Ok(())
}

/// The five name tables as name -> physical index, mirroring the on-disk order
/// (`docs/binfmt.md` §3 / §6, `docs/asm.md` §3.3): `muscles` from the rig,
/// `sensors_f` = read-backs ++ rig observables ++ `dt`,
/// `sensors_i` = rig observables ++ `tick`, then the two var tables.
struct Symbols {
    muscles: HashMap<SmolStr, u32>,
    sensors_f: HashMap<SmolStr, u32>,
    sensors_i: HashMap<SmolStr, u32>,
    vars_f: HashMap<SmolStr, u32>,
    vars_i: HashMap<SmolStr, u32>,
    groups: HashMap<SmolStr, Vec<u32>>,
    image_muscles: Vec<Muscle>,
    image_sensors_f: Vec<Sensor>,
    image_sensors_i: Vec<Sensor>,
    image_vars_f: Vec<VarF>,
    image_vars_i: Vec<VarI>,
}

impl Symbols {
    fn build(header: &AsmHeader, rig: &Rig) -> Result<Self, ImageGenError> {
        let mut muscles = HashMap::new();
        let mut sensors_f = HashMap::new();
        let mut sensors_i = HashMap::new();
        let mut vars_f = HashMap::new();
        let mut vars_i = HashMap::new();
        let mut groups = HashMap::new();

        let mut image_muscles = Vec::new();
        let mut image_sensors_f = Vec::new();
        let mut image_sensors_i = Vec::new();
        let mut image_vars_f = Vec::new();
        let mut image_vars_i = Vec::new();

        for (index, name) in rig.muscles.iter().enumerate() {
            let index = index as u32;
            let name = SmolStr::new(&**name);
            muscles.insert(name.clone(), index);
            // §6.2: every muscle is its own read-back float sensor, at the same index.
            sensors_f.insert(name.clone(), index);
            image_muscles.push(Muscle {
                name: name.as_str().into(),
            });
            image_sensors_f.push(Sensor {
                name: name.as_str().into(),
                source: SensorSource::Muscle,
            });
        }

        for group in &header.muscle_groups {
            check_declarable(&group.name)?;
            if muscles.contains_key(&group.name) {
                return Err(ImageGenError::GroupNameCollision(group.name.clone()));
            }
            if groups.contains_key(&group.name) {
                return Err(ImageGenError::DuplicateName(group.name.clone()));
            }
            let mut members = Vec::with_capacity(group.members.len());
            for member in &group.members {
                let index =
                    *muscles
                        .get(member)
                        .ok_or_else(|| ImageGenError::UnknownGroupMember {
                            group: group.name.clone(),
                            member: member.clone(),
                        })?;
                members.push(index);
            }
            groups.insert(group.name.clone(), members);
        }

        let sensor_f_base = image_sensors_f.len() as u32;
        for (offset, name) in rig.sensors_f.iter().enumerate() {
            let name = SmolStr::new(&**name);
            sensors_f.insert(name.clone(), sensor_f_base + offset as u32);
            image_sensors_f.push(Sensor {
                name: name.as_str().into(),
                source: SensorSource::Backend,
            });
        }
        sensors_f.insert(SmolStr::new("dt"), image_sensors_f.len() as u32);
        image_sensors_f.push(Sensor {
            name: "dt".into(),
            source: SensorSource::BuiltinDt,
        });

        for (index, name) in rig.sensors_i.iter().enumerate() {
            let name = SmolStr::new(&**name);
            sensors_i.insert(name.clone(), index as u32);
            image_sensors_i.push(Sensor {
                name: name.as_str().into(),
                source: SensorSource::Backend,
            });
        }
        sensors_i.insert(SmolStr::new("tick"), image_sensors_i.len() as u32);
        image_sensors_i.push(Sensor {
            name: "tick".into(),
            source: SensorSource::BuiltinTick,
        });

        for (index, (name, init)) in header.vars_f.iter().enumerate() {
            check_declarable(name)?;
            if vars_f.contains_key(name) {
                return Err(ImageGenError::DuplicateName(name.clone()));
            }
            vars_f.insert(name.clone(), index as u32);
            image_vars_f.push(VarF {
                name: name.as_str().into(),
                init: *init,
            });
        }

        for (index, (name, init)) in header.vars_i.iter().enumerate() {
            check_declarable(name)?;
            if vars_i.contains_key(name) {
                return Err(ImageGenError::DuplicateName(name.clone()));
            }
            vars_i.insert(name.clone(), index as u32);
            image_vars_i.push(VarI {
                name: name.as_str().into(),
                init: *init,
            });
        }

        Ok(Self {
            muscles,
            sensors_f,
            sensors_i,
            vars_f,
            vars_i,
            groups,
            image_muscles,
            image_sensors_f,
            image_sensors_i,
            image_vars_f,
            image_vars_i,
        })
    }
}

fn check_declarable(name: &SmolStr) -> Result<(), ImageGenError> {
    if is_reserved_name(name) {
        Err(ImageGenError::ReservedName(name.clone()))
    } else {
        Ok(())
    }
}

/// One source instruction together with everything the second pass needs:
/// where it starts in the code, and (for `FEXCITE`) the muscles it expands to.
struct Emission<'a> {
    insn: &'a AsmInsn,
    start_pc: usize,
    /// `None` for every instruction except `FEXCITE`; `Some` has one muscle
    /// index per emitted instruction, so a group can yield N instructions.
    targets: Option<Vec<u32>>,
}

/// First pass (`docs/asm.md` §5): labels point at the instruction that follows
/// them, and a `group` FEXCITE is expanded once into `Emission::targets`.
struct Layout<'a> {
    labels: BTreeMap<SmolStr, usize>,
    emissions: Vec<Emission<'a>>,
    /// Instruction count before the implicit `HALT`.
    code_len: usize,
}

fn layout<'a>(items: &'a [TextItem], symbols: &Symbols) -> Result<Layout<'a>, ImageGenError> {
    let mut labels = BTreeMap::new();
    let mut emissions = Vec::new();
    let mut pc = 0usize;

    for item in items {
        match item {
            TextItem::Label(name) => {
                if labels.insert(name.clone(), pc).is_some() {
                    return Err(ImageGenError::DuplicateLabel(name.clone()));
                }
            }
            TextItem::Insn(insn) => {
                let targets = if insn.op == OpCode::Fexcite {
                    Some(excite_targets(excite_name(insn)?, symbols)?)
                } else {
                    None
                };
                let count = targets.as_ref().map_or(1, Vec::len);
                emissions.push(Emission {
                    insn,
                    start_pc: pc,
                    targets,
                });
                pc += count;
            }
        }
    }

    Ok(Layout {
        labels,
        emissions,
        code_len: pc,
    })
}

/// Second pass: yield one `Insn` per emitted instruction. A `FEXCITE group`
/// becomes N items that share the same source register (`docs/asm.md` §4.2).
///
/// `start_pc` is fixed by the layout pass, so this needs no mutable code buffer
/// and no running counter: `pc`, and therefore the jump offset, is known before
/// the first item is yielded.
fn emit<'a, 'i>(
    layout: &'a Layout<'i>,
    symbols: &'a Symbols,
) -> impl Iterator<Item = Result<Insn, ImageGenError>> + 'a {
    layout.emissions.iter().flat_map(move |emission| {
        let count = emission.targets.as_ref().map_or(1, Vec::len);
        (0..count).map(move |offset| {
            let excite_index = emission.targets.as_ref().map(|targets| targets[offset]);
            emit_insn(
                emission.insn,
                emission.start_pc + offset,
                &layout.labels,
                symbols,
                excite_index,
            )
        })
    })
}

fn excite_name(insn: &AsmInsn) -> Result<&SmolStr, ImageGenError> {
    match insn.operands.first() {
        Some(AsmOperand::Index(name)) => Ok(name),
        _ => Err(ImageGenError::MissingIndexOperand(insn.op)),
    }
}

fn excite_targets(name: &SmolStr, symbols: &Symbols) -> Result<Vec<u32>, ImageGenError> {
    if let Some(members) = symbols.groups.get(name) {
        return Ok(members.clone());
    }
    if let Some(&muscle) = symbols.muscles.get(name) {
        return Ok(vec![muscle]);
    }
    Err(ImageGenError::UnknownExciteTarget(name.clone()))
}

fn emit_insn(
    asm: &AsmInsn,
    pc: usize,
    labels: &BTreeMap<SmolStr, usize>,
    symbols: &Symbols,
    excite_index: Option<u32>,
) -> Result<Insn, ImageGenError> {
    let mut insn = Insn::new(asm.op);

    for operand in &asm.operands {
        match operand {
            AsmOperand::Reg { id, name, .. } => match *name {
                RegName::Rd => insn.rd = *id,
                RegName::Ra => insn.ra = *id,
                RegName::Rb => insn.rb = *id,
                RegName::Rc => insn.rc = *id,
            },
            AsmOperand::ImmF32(value) => insn.imm = (*value).to_bits(),
            AsmOperand::ImmI32(value) => insn.imm = *value as u32,
            AsmOperand::ImmNone => {}
            AsmOperand::Index(name) => {
                insn.imm = match excite_index {
                    Some(index) => index,
                    None => resolve_index(asm.op, name, symbols)?,
                };
            }
            AsmOperand::Target(name) => {
                let target = *labels
                    .get(name)
                    .ok_or_else(|| ImageGenError::UndefinedLabel(name.clone()))?;
                let offset =
                    jump_offset(pc, target).ok_or(ImageGenError::JumpOffsetOverflow { pc })?;
                insn.imm = offset as u32;
            }
        }
    }

    Ok(insn)
}

fn resolve_index(op: OpCode, name: &SmolStr, symbols: &Symbols) -> Result<u32, ImageGenError> {
    let (table, unknown) = match op {
        OpCode::Fsense => (
            &symbols.sensors_f,
            ImageGenError::UnknownSensorF(name.clone()),
        ),
        OpCode::Isense => (
            &symbols.sensors_i,
            ImageGenError::UnknownSensorI(name.clone()),
        ),
        OpCode::Fvget | OpCode::Fvset => {
            (&symbols.vars_f, ImageGenError::UnknownVarF(name.clone()))
        }
        OpCode::Ivget | OpCode::Ivset => {
            (&symbols.vars_i, ImageGenError::UnknownVarI(name.clone()))
        }
        // No other opcode declares an `Index` operand. Treat the impossible
        // combination as a malformed instruction rather than a panic.
        _ => return Err(ImageGenError::MissingIndexOperand(op)),
    };

    if let Some(&index) = table.get(name) {
        return Ok(index);
    }
    if symbols.groups.contains_key(name) {
        return Err(ImageGenError::GroupOutsideExcite(name.clone()));
    }
    Err(unknown)
}

fn append_trailing_halt(code: &mut Vec<Insn>) {
    if code.last().map(|insn| insn.op) != Some(OpCode::Halt) {
        code.push(Insn::new(OpCode::Halt));
    }
}

fn check_labels(labels: &BTreeMap<SmolStr, usize>, code_len: usize) -> Result<(), ImageGenError> {
    for (label, &index) in labels {
        if index >= code_len {
            return Err(ImageGenError::LabelOutOfRange {
                label: label.clone(),
                index,
                code_len,
            });
        }
    }
    Ok(())
}

/// `docs/binfmt.md` §9.3 I6: `target = pc + 1 + off` must stay in `[0, code_len)`.
/// The arithmetic comes from `jump_target`, the single shared function.
fn check_jumps(code: &[Insn]) -> Result<(), ImageGenError> {
    for (pc, insn) in code.iter().enumerate() {
        if insn.op.imm_type() != ImmType::JumpOffset {
            continue;
        }
        let offset = insn.imm as i32;
        match jump_target(pc, offset) {
            Some(target) if target < code.len() => {}
            _ => {
                return Err(ImageGenError::JumpTargetOutOfRange {
                    pc,
                    offset,
                    code_len: code.len(),
                });
            }
        }
    }
    Ok(())
}

/// `docs/isa.md` §4.8 / §10.7: `WAIT` is a non-negative finite duration no
/// greater than the header's `max_wait`.
fn check_waits(code: &[Insn], max_wait: f32) -> Result<(), ImageGenError> {
    for (pc, insn) in code.iter().enumerate() {
        if insn.op != OpCode::Wait {
            continue;
        }
        let seconds = f32::from_bits(insn.imm);
        if !seconds.is_finite() || seconds < 0.0 || seconds > max_wait {
            return Err(ImageGenError::InvalidWait { pc, seconds });
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PendingState {
    Unreached,
    Clear,
    Set(usize),
}

/// `docs/asm.md` §6.6: a boolean dataflow over the CFG. `FEXCITE` sets pending,
/// `FLUSH` clears it, and a suspend point or `HALT` may only be reached clean.
/// Branches join with **union** (pending on any incoming edge is pending), which
/// is why a linear scan is not enough.
fn check_pending(code: &[Insn]) -> Result<(), ImageGenError> {
    let len = code.len();
    if len == 0 {
        return Ok(());
    }

    let mut states = vec![PendingState::Unreached; len];
    let mut queue = VecDeque::new();
    states[0] = PendingState::Clear;
    queue.push_back(0usize);

    while let Some(pc) = queue.pop_front() {
        let state = states[pc];
        let insn = code[pc];

        if matches!(insn.op, OpCode::Yield | OpCode::Wait | OpCode::Halt)
            && let PendingState::Set(excite_pc) = state
        {
            return Err(ImageGenError::PendingNotFlushed {
                excite_pc,
                suspend_pc: pc,
            });
        }

        let after = match insn.op {
            OpCode::Fexcite => PendingState::Set(pc),
            OpCode::Flush => PendingState::Clear,
            _ => state,
        };

        let mut successors = [None, None];
        match insn.op {
            OpCode::Halt => {}
            OpCode::Jmp => successors[0] = jump_target(pc, insn.imm as i32),
            OpCode::Yield | OpCode::Wait => successors[0] = Some(pc + 1),
            op if op.imm_type() == ImmType::JumpOffset => {
                successors[0] = Some(pc + 1);
                successors[1] = jump_target(pc, insn.imm as i32);
            }
            _ => successors[0] = Some(pc + 1),
        }

        for successor in successors.into_iter().flatten() {
            if successor >= len {
                continue;
            }
            let merged = merge_pending(states[successor], after);
            if merged != states[successor] {
                states[successor] = merged;
                queue.push_back(successor);
            }
        }
    }

    Ok(())
}

fn merge_pending(old: PendingState, new: PendingState) -> PendingState {
    match (old, new) {
        (PendingState::Unreached, other) | (other, PendingState::Unreached) => other,
        (PendingState::Clear, PendingState::Clear) => PendingState::Clear,
        (PendingState::Set(a), PendingState::Set(b)) => PendingState::Set(a.min(b)),
        (PendingState::Set(a), PendingState::Clear)
        | (PendingState::Clear, PendingState::Set(a)) => PendingState::Set(a),
    }
}
