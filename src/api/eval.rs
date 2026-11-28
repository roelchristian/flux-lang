use crate::{
    parser::Program,
    semantic::{prepare, Mode, Policy},
    Error,
};

#[derive(Debug)]
pub struct Output {
    pub stdout: String,
    pub status: u8,
}

#[derive(Debug)]
pub struct ReplOutput {
    pub value: String,
    pub stdout: String,
}

pub fn run(program: &Program) -> Result<Output, Error> {
    prepare(program, Mode::Script, Policy::TypedOnly)?.run()
}

pub fn view_entry(program: &Program) -> Result<ReplOutput, Error> {
    prepare(program, Mode::Inspection, Policy::TypedOnly)?.view()
}
