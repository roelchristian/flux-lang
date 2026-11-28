use crate::{
    parser::Program,
    semantic::{prepare, Mode, Policy},
    Error,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Inspection {
    pub type_name: String,
    pub schema: Option<String>,
}

pub fn check(program: &Program) -> Result<(), Error> {
    prepare(program, Mode::Script, Policy::TypedOnly).map(|_| ())
}

pub fn inspect_entry(program: &Program) -> Result<Inspection, Error> {
    prepare(program, Mode::Inspection, Policy::TypedOnly)?
        .inspection()
        .cloned()
        .ok_or_else(|| Error::new(0, "no expression to inspect"))
}
