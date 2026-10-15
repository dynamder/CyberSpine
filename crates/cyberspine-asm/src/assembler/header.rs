use smol_str::SmolStr;
use winnow::{
    ModalResult, Parser,
    ascii::{dec_int, dec_uint, float, space0, space1},
    combinator::{eof, iterator, opt, preceded, repeat, terminated},
    error::ContextError,
    token::take_while,
};

type Line<'i> = &'i str;

#[derive(Debug, Clone, PartialEq)]
pub struct GroupDef {
    pub name: SmolStr,
    pub members: Vec<SmolStr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FixedHeader {
    pub layer: u16,
    pub clock_hz: u32,
    pub fuel: u32,
    pub max_wait: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AsmHeader {
    pub fixed: FixedHeader,
    pub muscles: Vec<SmolStr>,
    pub muscle_group: Vec<GroupDef>,
    pub sensors_f: Vec<SmolStr>,
    pub sensors_i: Vec<SmolStr>,
    pub vars_f: Vec<(SmolStr, f32)>,
    pub vars_i: Vec<(SmolStr, i32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HeaderStmt {
    Layer(u16),
    Clock { hz: u32 },
    Fuel(u32),
    MaxWait(f32),
    Muscle(SmolStr),
    Group(GroupDef),
    Sensorf(SmolStr),
    SensorI(SmolStr),
    Varf { name: SmolStr, value: f32 },
    VarI { name: SmolStr, value: i32 },
    Text,
}

fn ident<'i>(input: &mut Line<'i>) -> ModalResult<&'i str> {
    (
        take_while(1.., |c: char| c.is_ascii_alphabetic() || c == '_'),
        take_while(0.., |c: char| c.is_ascii_alphanumeric() || c == '_'),
    )
        .take()
        .parse_next(input)
}

fn name_arg<'i>(input: &mut Line<'i>) -> ModalResult<SmolStr> {
    ident.map(SmolStr::new).parse_next(input)
}

fn eq(input: &mut Line<'_>) -> ModalResult<()> {
    (space0, '=', space0).void().parse_next(input)
}

fn header_stmt<'i>(input: &mut Line<'i>) -> ModalResult<HeaderStmt> {
    let dir = preceded('.', ident).parse_next(input)?;
    let stmt = match dir {
        "layer" => HeaderStmt::Layer(preceded(space1, dec_uint).parse_next(input)?),
        "clock" => {
            let _ = (preceded(space1, "hz"), eq).parse_next(input)?;
            HeaderStmt::Clock {
                hz: dec_uint.parse_next(input)?,
            }
        }
        "fuel" => HeaderStmt::Fuel(preceded(space1, dec_uint).parse_next(input)?),
        "max_wait" => HeaderStmt::MaxWait(preceded(space1, float).parse_next(input)?),
        "muscle" => HeaderStmt::Muscle(preceded(space1, name_arg).parse_next(input)?),
        "group" => HeaderStmt::Group(group.parse_next(input)?),
        "sensor_f" => HeaderStmt::Sensorf(preceded(space1, name_arg).parse_next(input)?),
        "sensor_i" => HeaderStmt::SensorI(preceded(space1, name_arg).parse_next(input)?),
        "var_f" => {
            let name = preceded(space1, name_arg).parse_next(input)?;
            let value = preceded(eq, float).parse_next(input)?;
            HeaderStmt::Varf { name, value }
        }
        "var_i" => {
            let name = preceded(space1, name_arg).parse_next(input)?;
            let value = preceded(eq, dec_int).parse_next(input)?;
            HeaderStmt::VarI { name, value }
        }
        "text" => HeaderStmt::Text,
        _ => return Err(winnow::error::ErrMode::Backtrack(ContextError::new())),
    };

    (space0, eof).parse_next(input)?;
    Ok(stmt)
}

fn group<'i>(input: &mut Line<'i>) -> ModalResult<GroupDef> {
    let group_name = preceded(space1, name_arg).parse_next(input)?;
    let _ = eq.parse_next(input)?;

    let first = name_arg.parse_next(input)?;
    let rest: Vec<SmolStr> =
        repeat(0.., preceded((space0, ',', space0), name_arg)).parse_next(input)?;
    let mut members = vec![first];
    members.extend(rest);

    Ok(GroupDef {
        name: group_name,
        members,
    })
}
