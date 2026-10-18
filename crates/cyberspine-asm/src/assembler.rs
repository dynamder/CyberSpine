pub mod header;
pub mod text;

use cyberspine_vm::binfmt::image::Image;
use thiserror::Error;
use winnow::{ModalResult, Parser, combinator::trace, token::take_while};

pub type Line<'i> = &'i str;

fn ident<'i>(input: &mut Line<'i>) -> ModalResult<&'i str> {
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

#[derive(Debug, Clone)]
pub struct AsmHeader;

#[derive(Debug, Clone)]
pub struct AsmInsn;

#[derive(Debug, Clone)]
pub struct Asm {
    pub header: AsmHeader,
    pub insn: AsmInsn,
}

#[derive(Debug, Error)]
pub enum AsmParseError {}

pub fn parse(asm_str: &mut str) -> Result<Asm, AsmParseError> {
    unimplemented!()
}

fn comments<'i>(input: &mut Line<'i>) -> ModalResult<()> {
    trace("comments", ('#', take_while(0.., |_| true)).void()).parse_next(input)
}
#[derive(Debug, Error)]
pub enum ImageGenError {}

pub fn generate_image(asm: Asm) -> Result<Image, ImageGenError> {
    unimplemented!()
}

fn parse_header(asm_str: &mut str) -> Result<AsmHeader, AsmParseError> {
    unimplemented!()
}
