pub mod header;

use cyberspine_vm::binfmt::image::Image;
use thiserror::Error;

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

#[derive(Debug, Error)]
pub enum ImageGenError {}

pub fn generate_image(asm: Asm) -> Result<Image, ImageGenError> {
    unimplemented!()
}

fn parse_header(asm_str: &mut str) -> Result<AsmHeader, AsmParseError> {
    unimplemented!()
}
