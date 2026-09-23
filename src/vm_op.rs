pub enum Op {
    //Constants and transport
    Fmovi {
        rd: usize,
        imm: f32,
    },
    Imovi {
        rd: usize,
        imm: i32,
    },
    Fmov {
        rd: usize,
        ra: usize,
    },
    Imov {
        rd: usize,
        ra: usize,
    },
    //Float binary operation
    Fadd {
        rd: usize,
        ra: usize,
        rb: usize,
    },
    Fsub {
        rd: usize,
        ra: usize,
        rb: usize,
    },
    Fmul {
        rd: usize,
        ra: usize,
        rb: usize,
    },
    Fdiv {
        rd: usize,
        ra: usize,
        rb: usize,
    },
    Fmin {
        rd: usize,
        ra: usize,
        rb: usize,
    },
    Fmax {
        rd: usize,
        ra: usize,
        rb: usize,
    },
    //Float unary operation
    Fneg {
        rd: usize,
        ra: usize,
    },
    Fabs {
        rd: usize,
        ra: usize,
    },
    Fsqrt {
        rd: usize,
        ra: usize,
    },
    Fsin {
        rd: usize,
        ra: usize,
    },
    Fcos {
        rd: usize,
        ra: usize,
    },
    Ffrac {
        rd: usize,
        ra: usize,
    },
    //Float Ternary operation
    Fmadd {
        rd: usize,
        ra: usize,
        rb: usize,
        rc: usize,
    },
    Flerp {
        rd: usize,
        ra: usize,
        rb: usize,
        rc: usize,
    },
    Fclamp {
        rd: usize,
        ra: usize,
        rb: usize,
        rc: usize,
    },
    //Integer Ops and conversion
    Iadd {
        rd: usize,
        ra: usize,
        rb: usize,
    },
    Isub {
        rd: usize,
        ra: usize,
        rb: usize,
    },
    Imul {
        rd: usize,
        ra: usize,
        rb: usize,
    },
    Itof {
        rd: usize,
        ra: usize,
    },
    Ftoi {
        rd: usize,
        ra: usize,
    },
    //Control flow
    Jmp {
        imm: isize,
    },
    Ijz {
        ra: usize,
        imm: isize,
    },
    Ijnz {
        ra: usize,
        imm: isize,
    },
    Flt {
        ra: usize,
        rb: usize,
        imm: isize,
    },
    Fle {
        ra: usize,
        rb: usize,
        imm: isize,
    },
    Fgt {
        ra: usize,
        rb: usize,
        imm: isize,
    },
    Fge {
        ra: usize,
        rb: usize,
        imm: isize,
    },
    Ilt {
        ra: usize,
        rb: usize,
        imm: isize,
    },
    Ile {
        ra: usize,
        rb: usize,
        imm: isize,
    },
    Igt {
        ra: usize,
        rb: usize,
        imm: isize,
    },
    Ige {
        ra: usize,
        rb: usize,
        imm: isize,
    },
    Ieq {
        ra: usize,
        rb: usize,
        imm: isize,
    },
    Ine {
        ra: usize,
        rb: usize,
        imm: isize,
    },
    //Input
    Fsense {
        rd: usize,
        imm: usize,
    },
    Isense {
        rd: usize,
        imm: usize,
    },
    //Excite
    Fexcite {},
}
