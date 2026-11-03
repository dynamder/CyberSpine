pub mod diagnostic;
pub mod header;
pub mod lint;
pub mod text;

use std::collections::{BTreeMap, HashMap, VecDeque};

use cyberspine_core::binfmt::image::{
    FORMAT_VERSION, FixedHeader as ImageFixedHeader, ISA_VERSION, Image, SensorSource,
};
use cyberspine_core::binfmt::reason::{MAX_REASON_LEN, Template, TemplateError};
use cyberspine_core::isa::clock::{Hz, Seconds};
use cyberspine_core::isa::jump::{jump_offset, jump_target};
use cyberspine_core::isa::{ImmType, Insn, OpCode, RegName};
use cyberspine_core::rig::{Name, Rig, RigError, is_reserved as is_reserved_name};
use smol_str::SmolStr;
use thiserror::Error;
use winnow::{Parser, combinator::trace, token::take_while};

use diagnostic::PResult;
pub use diagnostic::{AsmParseErrors, Location, ParseError, ParseErrorKind};
use header::{AsmHeader, header as header_section};
pub use lint::{Warning, WarningKind, lint};
use text::{AsmInsn, AsmOperand, Located, TextItem, text as text_section};

pub type Line<'i> = &'i str;

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
    pub text: Vec<Located<TextItem>>,
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
    // declaration section. Shift both its errors and its item locations back
    // into file coordinates, so a generation error points at the real line.
    let header_len = asm_str.len() - cursor.len();
    let text_line_offset = asm_str[..header_len].matches('\n').count();
    let text = text_section(&mut cursor);

    match (header, text) {
        (Ok(header), Ok(mut text)) => {
            for item in &mut text {
                item.location.line += text_line_offset;
            }
            Ok(Asm { header, text })
        }
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
    #[error("duplicate label `{label}`")]
    DuplicateLabel { label: SmolStr, location: Location },
    #[error("undefined label `{label}`")]
    UndefinedLabel { label: SmolStr, location: Location },
    #[error(
        "label `{label}` points to {index}, past the end of the code ({code_len} instructions)"
    )]
    LabelOutOfRange {
        label: SmolStr,
        index: usize,
        code_len: usize,
        location: Location,
    },
    #[error("offset {offset} leaves the code (code_len {code_len})")]
    JumpTargetOutOfRange {
        pc: usize,
        offset: i32,
        code_len: usize,
        location: Location,
    },
    #[error("jump distance does not fit an i32 offset")]
    JumpOffsetOverflow { pc: usize, location: Location },
    #[error("malformed {op:?}: expected an index operand")]
    MissingIndexOperand { op: OpCode, location: Location },
    #[error("`{0}` is declared more than once")]
    DuplicateName(SmolStr),
    #[error("`{0}` is a reserved name")]
    ReservedName(SmolStr),
    #[error("the rig is not usable: {0}")]
    Rig(#[from] RigError),
    #[error("FEXCITE target `{name}` is neither a muscle nor a group")]
    UnknownExciteTarget { name: SmolStr, location: Location },
    #[error("unknown float sensor `{name}`")]
    UnknownSensorF { name: SmolStr, location: Location },
    #[error("unknown integer sensor `{name}`")]
    UnknownSensorI { name: SmolStr, location: Location },
    #[error("unknown float variable `{name}`")]
    UnknownVarF { name: SmolStr, location: Location },
    #[error("unknown integer variable `{name}`")]
    UnknownVarI { name: SmolStr, location: Location },
    #[error("group `{name}` is only legal as a FEXCITE target")]
    GroupOutsideExcite { name: SmolStr, location: Location },
    #[error("group `{group}` contains unknown muscle `{member}`")]
    UnknownGroupMember { group: SmolStr, member: SmolStr },
    #[error("group `{0}` collides with a muscle name")]
    GroupNameCollision(SmolStr),
    #[error("FEXCITE is still pending here; FLUSH is missing")]
    PendingNotFlushed {
        excite_pc: usize,
        suspend_pc: usize,
        location: Location,
    },
    #[error("WAIT {seconds} is not a non-negative finite duration up to max_wait")]
    InvalidWait {
        pc: usize,
        seconds: f32,
        location: Location,
    },
    #[error("`hz` must be greater than zero")]
    ZeroHz,
    #[error("`max_wait` must be a non-negative finite f32")]
    InvalidMaxWait,
    #[error("INTR needs a quoted reason template")]
    MissingReason { location: Location },
    #[error("invalid reason template: {source}")]
    InvalidReason {
        location: Location,
        #[source]
        source: TemplateError,
    },
    #[error("the reason template is {len} bytes, above the {MAX_REASON_LEN}-byte limit")]
    ReasonTooLong { len: usize, location: Location },
    #[error("the program is too large for the format's 32-bit instruction count")]
    TooManyInstructions,
    /// A mismatch between the layout pass and the emitted code. No source can
    /// cause it: it means the assembler is wrong, and the CLI says so.
    #[error("emitted {emitted} instructions, the layout says {layout}")]
    Internal { emitted: usize, layout: usize },
}

impl ImageGenError {
    /// Where in the source this error belongs, when the source construct is
    /// known. Header, rig and size errors have no single line to point at.
    pub fn location(&self) -> Option<Location> {
        match self {
            Self::DuplicateLabel { location, .. }
            | Self::UndefinedLabel { location, .. }
            | Self::LabelOutOfRange { location, .. }
            | Self::JumpTargetOutOfRange { location, .. }
            | Self::JumpOffsetOverflow { location, .. }
            | Self::MissingIndexOperand { location, .. }
            | Self::UnknownExciteTarget { location, .. }
            | Self::UnknownSensorF { location, .. }
            | Self::UnknownSensorI { location, .. }
            | Self::UnknownVarF { location, .. }
            | Self::UnknownVarI { location, .. }
            | Self::GroupOutsideExcite { location, .. }
            | Self::PendingNotFlushed { location, .. }
            | Self::MissingReason { location }
            | Self::InvalidReason { location, .. }
            | Self::ReasonTooLong { location, .. }
            | Self::InvalidWait { location, .. } => Some(*location),
            Self::DuplicateName(_)
            | Self::ReservedName(_)
            | Self::UnknownGroupMember { .. }
            | Self::GroupNameCollision(_)
            | Self::Rig(_)
            | Self::ZeroHz
            | Self::InvalidMaxWait
            | Self::TooManyInstructions
            | Self::Internal { .. } => None,
        }
    }
}

/// Resolve names, labels and groups against `rig`, then produce the canonical
/// `Image`.
pub fn generate_image(asm: &Asm, rig: &Rig) -> Result<Image, ImageGenError> {
    check_header(&asm.header)?;
    check_rig(rig)?;

    let symbols = Symbols::build(&asm.header, rig)?;
    let reasons = ReasonTable::build(&asm.text, &symbols)?;
    let layout = layout(&asm.text, &symbols)?;

    let mut code = Vec::with_capacity(layout.code_len);
    let mut locations = Vec::with_capacity(layout.code_len);
    for emitted in emit(&layout, &symbols, &reasons) {
        let (insn, location) = emitted?;
        code.push(insn);
        locations.push(location);
    }
    if code.len() != layout.code_len {
        return Err(ImageGenError::Internal {
            emitted: code.len(),
            layout: layout.code_len,
        });
    }

    // The implicit `HALT` has no line of its own; if it ever becomes the site of
    // a diagnostic, the last real instruction is the closest thing to it.
    if append_trailing_halt(&mut code) {
        locations.push(
            layout
                .emissions
                .last()
                .map_or(Location::new(1, 1), |e| e.location),
        );
    }
    debug_assert_eq!(locations.len(), code.len());

    if code.len() > u32::MAX as usize {
        return Err(ImageGenError::TooManyInstructions);
    }

    check_labels(&layout.labels, code.len())?;
    check_jumps(&code, &locations)?;
    check_waits(&code, &locations, max_wait(&asm.header)?)?;
    check_pending(&code, &locations)?;

    let mut image = Image::new(code);
    *image.header_mut() = ImageFixedHeader {
        format_version: FORMAT_VERSION,
        isa_version: ISA_VERSION,
        layer: asm.header.fixed.layer,
        hz: clock_hz(&asm.header)?,
        fuel: asm.header.fixed.fuel,
        max_wait: max_wait(&asm.header)?,
        rig: rig.fingerprint(),
    };
    for muscle in symbols.image_muscles {
        image.push_muscle_only(&muscle);
    }
    for (sensor, source) in symbols.image_sensors_f {
        image.push_sensor_f(&sensor, source);
    }
    for (sensor, source) in symbols.image_sensors_i {
        image.push_sensor_i(&sensor, source);
    }
    for (var, init) in symbols.image_vars_f {
        image.push_var_f(&var, init);
    }
    for (var, init) in symbols.image_vars_i {
        image.push_var_i(&var, init);
    }
    for reason in reasons.entries() {
        image.push_reason(reason);
    }
    Ok(image)
}

/// The `INTR` reason table, built before emission so every instruction can be
/// handed its index.
///
/// Interning is keyed by the **canonical** template, so two sites that resolve
/// to the same text share one row. The order is first use in the instruction
/// stream, which is what makes the table — and therefore the image hash —
/// independent of anything but the source (`docs/binfmt.md` §8).
struct ReasonTable {
    entries: Vec<String>,
    by_location: HashMap<Location, u32>,
}

impl ReasonTable {
    fn build(items: &[Located<TextItem>], symbols: &Symbols) -> Result<Self, ImageGenError> {
        let mut entries: Vec<String> = Vec::new();
        let mut by_text: HashMap<String, u32> = HashMap::new();
        let mut by_location = HashMap::new();

        for located in items {
            let TextItem::Insn(insn) = &located.item else {
                continue;
            };
            if insn.op != OpCode::Intr {
                continue;
            }
            let location = located.location;
            let source = match insn.operands.first() {
                Some(AsmOperand::Text(text)) => text,
                _ => return Err(ImageGenError::MissingReason { location }),
            };

            let mut template = Template::parse(source)
                .map_err(|source| ImageGenError::InvalidReason { location, source })?;
            template
                .resolve_vars(
                    |name| symbols.vars_f.get(name).copied(),
                    |name| symbols.vars_i.get(name).copied(),
                )
                .map_err(|source| ImageGenError::InvalidReason { location, source })?;

            let canonical = template.canonical();
            if canonical.len() > MAX_REASON_LEN {
                return Err(ImageGenError::ReasonTooLong {
                    len: canonical.len(),
                    location,
                });
            }

            let index = *by_text.entry(canonical.clone()).or_insert_with(|| {
                let index = entries.len() as u32;
                entries.push(canonical);
                index
            });
            by_location.insert(location, index);
        }

        Ok(Self {
            entries,
            by_location,
        })
    }

    fn entries(&self) -> &[String] {
        &self.entries
    }

    fn index(&self, location: Location) -> u32 {
        self.by_location[&location]
    }
}

/// The clock rate the header declares, as the type every consumer divides by.
fn clock_hz(header: &AsmHeader) -> Result<Hz, ImageGenError> {
    Hz::new(header.fixed.clock_hz).ok_or(ImageGenError::ZeroHz)
}

fn check_header(header: &AsmHeader) -> Result<(), ImageGenError> {
    clock_hz(header)?;
    max_wait(header)?;
    Ok(())
}

fn max_wait(header: &AsmHeader) -> Result<Seconds, ImageGenError> {
    Seconds::new(header.fixed.max_wait).ok_or(ImageGenError::InvalidMaxWait)
}

/// The rig-level rules themselves live in `core::rig::Rig::validate`; this only
/// maps them into the assembler's error type. A rig built in code never passes
/// through the manifest parser, so the assembler re-checks it here.
fn check_rig(rig: &Rig) -> Result<(), ImageGenError> {
    Ok(rig.validate()?)
}

/// The five name tables as name -> physical index, mirroring the on-disk order
/// (`docs/binfmt.md` §3 / §6, `docs/asm.md` §3.3): `muscles` from the rig,
/// `sensors_f` = read-backs ++ rig observables ++ `dt`,
/// `sensors_i` = rig observables ++ `tick`, then the two var tables.
///
/// The `image_*` rows are the same tables in file order, as the argument lists the
/// image builder wants: a row's name lives in the image's arena, so it cannot be
/// built here as a finished table entry.
struct Symbols {
    muscles: HashMap<SmolStr, u32>,
    sensors_f: HashMap<SmolStr, u32>,
    sensors_i: HashMap<SmolStr, u32>,
    vars_f: HashMap<SmolStr, u32>,
    vars_i: HashMap<SmolStr, u32>,
    groups: HashMap<SmolStr, Vec<u32>>,
    image_muscles: Vec<Name>,
    image_sensors_f: Vec<(Name, SensorSource)>,
    image_sensors_i: Vec<(Name, SensorSource)>,
    image_vars_f: Vec<(Name, f32)>,
    image_vars_i: Vec<(Name, i32)>,
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
            let key = SmolStr::new(name.as_str());
            muscles.insert(key.clone(), index);
            // binfmt.md §6.2: every muscle is its own read-back float sensor, at the same index.
            sensors_f.insert(key, index);
            image_muscles.push(name.clone());
            image_sensors_f.push((name.clone(), SensorSource::Muscle));
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
            sensors_f.insert(SmolStr::new(name.as_str()), sensor_f_base + offset as u32);
            image_sensors_f.push((name.clone(), SensorSource::Backend));
        }
        sensors_f.insert(SmolStr::new("dt"), image_sensors_f.len() as u32);
        image_sensors_f.push((builtin("dt"), SensorSource::BuiltinDt));

        for (index, name) in rig.sensors_i.iter().enumerate() {
            sensors_i.insert(SmolStr::new(name.as_str()), index as u32);
            image_sensors_i.push((name.clone(), SensorSource::Backend));
        }
        sensors_i.insert(SmolStr::new("tick"), image_sensors_i.len() as u32);
        image_sensors_i.push((builtin("tick"), SensorSource::BuiltinTick));

        for (index, (name, init)) in header.vars_f.iter().enumerate() {
            check_declarable(name)?;
            if vars_f.contains_key(name) {
                return Err(ImageGenError::DuplicateName(name.clone()));
            }
            vars_f.insert(name.clone(), index as u32);
            image_vars_f.push((
                Name::new(name.as_str()).expect("the ident parser only produces identifiers"),
                *init,
            ));
        }

        for (index, (name, init)) in header.vars_i.iter().enumerate() {
            check_declarable(name)?;
            if vars_i.contains_key(name) {
                return Err(ImageGenError::DuplicateName(name.clone()));
            }
            vars_i.insert(name.clone(), index as u32);
            image_vars_i.push((
                Name::new(name.as_str()).expect("the ident parser only produces identifiers"),
                *init,
            ));
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

/// The two built-in sensor names, which are known to be identifiers.
fn builtin(text: &'static str) -> Name {
    Name::new(text).expect("the built-in names are identifiers")
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
    location: Location,
    start_pc: usize,
    /// `None` for every instruction except `FEXCITE`; `Some` has one muscle
    /// index per emitted instruction, so a group can yield N instructions.
    targets: Option<Vec<u32>>,
}

/// First pass (`docs/asm.md` §5): labels point at the instruction that follows
/// them, and a `group` FEXCITE is expanded once into `Emission::targets`.
struct Layout<'a> {
    labels: BTreeMap<SmolStr, (usize, Location)>,
    emissions: Vec<Emission<'a>>,
    /// Instruction count before the implicit `HALT`.
    code_len: usize,
}

fn layout<'a>(
    items: &'a [Located<TextItem>],
    symbols: &Symbols,
) -> Result<Layout<'a>, ImageGenError> {
    let mut labels = BTreeMap::new();
    let mut emissions = Vec::new();
    let mut pc = 0usize;

    for located in items {
        let location = located.location;
        match &located.item {
            TextItem::Label(name) => {
                if labels.insert(name.clone(), (pc, location)).is_some() {
                    return Err(ImageGenError::DuplicateLabel {
                        label: name.clone(),
                        location,
                    });
                }
            }
            TextItem::Insn(insn) => {
                let targets = if insn.op == OpCode::Fexcite {
                    Some(excite_targets(
                        excite_name(insn, location)?,
                        location,
                        symbols,
                    )?)
                } else {
                    None
                };
                let count = targets.as_ref().map_or(1, Vec::len);
                emissions.push(Emission {
                    insn,
                    location,
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
    reasons: &'a ReasonTable,
) -> impl Iterator<Item = Result<(Insn, Location), ImageGenError>> + 'a {
    layout.emissions.iter().flat_map(move |emission| {
        let count = emission.targets.as_ref().map_or(1, Vec::len);
        (0..count).map(move |offset| {
            let excite_index = emission.targets.as_ref().map(|targets| targets[offset]);
            emit_insn(
                emission.insn,
                emission.location,
                emission.start_pc + offset,
                &layout.labels,
                symbols,
                reasons,
                excite_index,
            )
            .map(|insn| (insn, emission.location))
        })
    })
}

fn excite_name(insn: &AsmInsn, location: Location) -> Result<&SmolStr, ImageGenError> {
    match insn.operands.first() {
        Some(AsmOperand::Index(name)) => Ok(name),
        _ => Err(ImageGenError::MissingIndexOperand {
            op: insn.op,
            location,
        }),
    }
}

fn excite_targets(
    name: &SmolStr,
    location: Location,
    symbols: &Symbols,
) -> Result<Vec<u32>, ImageGenError> {
    if let Some(members) = symbols.groups.get(name) {
        return Ok(members.clone());
    }
    if let Some(&muscle) = symbols.muscles.get(name) {
        return Ok(vec![muscle]);
    }
    Err(ImageGenError::UnknownExciteTarget {
        name: name.clone(),
        location,
    })
}

fn emit_insn(
    asm: &AsmInsn,
    location: Location,
    pc: usize,
    labels: &BTreeMap<SmolStr, (usize, Location)>,
    symbols: &Symbols,
    reasons: &ReasonTable,
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
                    None => resolve_index(asm.op, name, location, symbols)?,
                };
            }
            AsmOperand::Target(name) => {
                let (target, _) =
                    *labels
                        .get(name)
                        .ok_or_else(|| ImageGenError::UndefinedLabel {
                            label: name.clone(),
                            location,
                        })?;
                let offset = jump_offset(pc, target)
                    .ok_or(ImageGenError::JumpOffsetOverflow { pc, location })?;
                insn.imm = offset as u32;
            }
            AsmOperand::Text(_) => insn.imm = reasons.index(location),
        }
    }

    Ok(insn)
}

fn resolve_index(
    op: OpCode,
    name: &SmolStr,
    location: Location,
    symbols: &Symbols,
) -> Result<u32, ImageGenError> {
    let (table, unknown) = match op {
        OpCode::Fsense => (
            &symbols.sensors_f,
            ImageGenError::UnknownSensorF {
                name: name.clone(),
                location,
            },
        ),
        OpCode::Isense => (
            &symbols.sensors_i,
            ImageGenError::UnknownSensorI {
                name: name.clone(),
                location,
            },
        ),
        OpCode::Fvget | OpCode::Fvset => (
            &symbols.vars_f,
            ImageGenError::UnknownVarF {
                name: name.clone(),
                location,
            },
        ),
        OpCode::Ivget | OpCode::Ivset => (
            &symbols.vars_i,
            ImageGenError::UnknownVarI {
                name: name.clone(),
                location,
            },
        ),
        // No other opcode declares an `Index` operand. Treat the impossible
        // combination as a malformed instruction rather than a panic.
        _ => {
            return Err(ImageGenError::MissingIndexOperand { op, location });
        }
    };

    if let Some(&index) = table.get(name) {
        return Ok(index);
    }
    if symbols.groups.contains_key(name) {
        return Err(ImageGenError::GroupOutsideExcite {
            name: name.clone(),
            location,
        });
    }
    Err(unknown)
}

/// Append the implicit `HALT` (§5) when the source does not end in one.
/// Returns whether it had to, so the caller can keep the location vector in
/// step with the code.
fn append_trailing_halt(code: &mut Vec<Insn>) -> bool {
    if code.last().map(|insn| insn.op) == Some(OpCode::Halt) {
        return false;
    }
    code.push(Insn::new(OpCode::Halt));
    true
}

fn check_labels(
    labels: &BTreeMap<SmolStr, (usize, Location)>,
    code_len: usize,
) -> Result<(), ImageGenError> {
    for (label, &(index, location)) in labels {
        if index >= code_len {
            return Err(ImageGenError::LabelOutOfRange {
                label: label.clone(),
                index,
                code_len,
                location,
            });
        }
    }
    Ok(())
}

/// `docs/binfmt.md` §9.3 I6: `target = pc + 1 + off` must stay in `[0, code_len)`.
/// The arithmetic comes from `jump_target`, the single shared function.
fn check_jumps(code: &[Insn], locations: &[Location]) -> Result<(), ImageGenError> {
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
                    location: locations[pc],
                });
            }
        }
    }
    Ok(())
}

/// `docs/isa.md` §4.8 / §10.7: `WAIT` is a non-negative finite duration no
/// greater than the header's `max_wait`.
fn check_waits(
    code: &[Insn],
    locations: &[Location],
    max_wait: Seconds,
) -> Result<(), ImageGenError> {
    for (pc, insn) in code.iter().enumerate() {
        if insn.op != OpCode::Wait {
            continue;
        }
        let seconds = f32::from_bits(insn.imm);
        match Seconds::new(seconds) {
            Some(seconds) if seconds <= max_wait => {}
            _ => {
                return Err(ImageGenError::InvalidWait {
                    pc,
                    seconds,
                    location: locations[pc],
                });
            }
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

/// `docs/asm.md` §6 第 6 条: a boolean dataflow over the CFG. `FEXCITE` sets pending,
/// `FLUSH` clears it, and a suspend point or `HALT` may only be reached clean.
/// Branches join with **union** (pending on any incoming edge is pending), which
/// is why a linear scan is not enough.
fn check_pending(code: &[Insn], locations: &[Location]) -> Result<(), ImageGenError> {
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

        if matches!(
            insn.op,
            OpCode::Yield | OpCode::Wait | OpCode::Intr | OpCode::Halt
        ) && let PendingState::Set(excite_pc) = state
        {
            return Err(ImageGenError::PendingNotFlushed {
                excite_pc,
                suspend_pc: pc,
                location: locations[pc],
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
