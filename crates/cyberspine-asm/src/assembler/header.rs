use smol_str::SmolStr;
use thiserror::Error;
use winnow::{
    ModalResult, Parser,
    ascii::{dec_int, dec_uint, float, space0, space1},
    combinator::{eof, opt, preceded, repeat, terminated, trace},
    error::ContextError,
};

use crate::assembler::comments;

use super::Line;
use super::ident;

//TODO: make the error aggregatable to collect all errors in the asm_file
#[derive(Debug, Error)]
pub enum FixedHeaderError {
    #[error("Duplicate fields in fixed header: {0}")]
    Duplicate(SmolStr),
    #[error("Missing fields in fixed header: {0}")]
    Missing(SmolStr),
}

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
pub struct ParsingFixedHeader {
    layer: Option<u16>,
    clock_hz: Option<u32>,
    fuel: Option<u32>,
    max_wait: Option<f32>,
}

impl ParsingFixedHeader {
    pub fn new() -> Self {
        ParsingFixedHeader {
            layer: None,
            clock_hz: None,
            fuel: None,
            max_wait: None,
        }
    }

    pub fn layer(&mut self, value: u16) -> Result<(), FixedHeaderError> {
        if self.layer.is_some() {
            return Err(FixedHeaderError::Duplicate(SmolStr::new("layer")));
        }

        self.layer = Some(value);
        Ok(())
    }

    pub fn clock_hz(&mut self, value: u32) -> Result<(), FixedHeaderError> {
        if self.clock_hz.is_some() {
            return Err(FixedHeaderError::Duplicate(SmolStr::new("clock_hz")));
        }

        self.clock_hz = Some(value);
        Ok(())
    }

    pub fn fuel(&mut self, value: u32) -> Result<(), FixedHeaderError> {
        if self.fuel.is_some() {
            return Err(FixedHeaderError::Duplicate(SmolStr::new("fuel")));
        }

        self.fuel = Some(value);
        Ok(())
    }

    pub fn max_wait(&mut self, value: f32) -> Result<(), FixedHeaderError> {
        if self.max_wait.is_some() {
            return Err(FixedHeaderError::Duplicate(SmolStr::new("max_wait")));
        }

        self.max_wait = Some(value);
        Ok(())
    }

    pub fn build(self) -> Result<FixedHeader, FixedHeaderError> {
        if self.layer.is_none() {
            return Err(FixedHeaderError::Missing(SmolStr::new("layer")));
        }
        if self.clock_hz.is_none() {
            return Err(FixedHeaderError::Missing(SmolStr::new("clock_hz")));
        }
        if self.fuel.is_none() {
            return Err(FixedHeaderError::Missing(SmolStr::new("fuel")));
        }
        if self.max_wait.is_none() {
            return Err(FixedHeaderError::Missing(SmolStr::new("max_wait")));
        }

        Ok(FixedHeader {
            layer: self.layer.unwrap(),
            clock_hz: self.clock_hz.unwrap(),
            fuel: self.fuel.unwrap(),
            max_wait: self.max_wait.unwrap(),
        })
    }
}

impl Default for ParsingFixedHeader {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AsmHeader {
    pub fixed: FixedHeader,
    pub muscles: Vec<SmolStr>,
    pub muscle_groups: Vec<GroupDef>,
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

fn name_arg<'i>(input: &mut Line<'i>) -> ModalResult<SmolStr> {
    trace("name_arg", ident.map(SmolStr::new)).parse_next(input)
}

fn eq(input: &mut Line<'_>) -> ModalResult<()> {
    trace("eq", (space0, '=', space0).void()).parse_next(input)
}

fn header_stmt<'i>(input: &mut Line<'i>) -> ModalResult<HeaderStmt> {
    let dir = trace("header_prefix", preceded('.', ident)).parse_next(input)?;
    let stmt = match dir.to_lowercase().as_str() {
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
        "group" => HeaderStmt::Group(trace("group", group).parse_next(input)?),
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

    // (space0, eof).parse_next(input)?;
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

pub fn header(input: &mut &str) -> ModalResult<AsmHeader> {
    let mut parsing_fixed_header = ParsingFixedHeader::default();
    let mut muscles: Vec<SmolStr> = Vec::new();
    let mut muscle_groups: Vec<GroupDef> = Vec::new();
    let mut sensors_f: Vec<SmolStr> = Vec::new();
    let mut sensors_i: Vec<SmolStr> = Vec::new();
    let mut vars_f: Vec<(SmolStr, f32)> = Vec::new();
    let mut vars_i: Vec<(SmolStr, i32)> = Vec::new();

    for mut line in input.lines() {
        let header_stmt = terminated(
            terminated(
                opt(trace("header_stmt", header_stmt)),
                (space0, opt(comments)),
            ),
            eof,
        )
        .parse_next(&mut line)?;

        if header_stmt.is_none() {
            continue;
        }

        match header_stmt.unwrap() {
            HeaderStmt::Layer(layer) => parsing_fixed_header.layer(layer).unwrap(),
            HeaderStmt::Clock { hz } => parsing_fixed_header.clock_hz(hz).unwrap(),
            HeaderStmt::Fuel(fuel) => parsing_fixed_header.fuel(fuel).unwrap(),
            HeaderStmt::MaxWait(max_wait) => parsing_fixed_header.max_wait(max_wait).unwrap(),
            HeaderStmt::Muscle(muscle) => muscles.push(muscle),
            HeaderStmt::Group(group_def) => muscle_groups.push(group_def),
            HeaderStmt::Sensorf(sensor_f) => sensors_f.push(sensor_f),
            HeaderStmt::SensorI(sensor_i) => sensors_i.push(sensor_i),
            HeaderStmt::Varf { name, value } => vars_f.push((name, value)),
            HeaderStmt::VarI { name, value } => vars_i.push((name, value)),
            HeaderStmt::Text => break,
        }
    }

    Ok(AsmHeader {
        fixed: parsing_fixed_header.build().unwrap(),
        muscles,
        muscle_groups,
        sensors_f,
        sensors_i,
        vars_f,
        vars_i,
    })
}
