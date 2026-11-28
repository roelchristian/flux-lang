//! Internal typed HIR interpreter: frames, closures, dispatch and host entry points.
use super::{function, subtract_work, Function, RuntimeInput, VariantValue, V};
use crate::{
    hir::{Definition, HirProgram, Span},
    resolve::SymbolId,
    types::Type,
};
use std::{
    collections::HashMap,
    io::{BufRead, Write},
    rc::Rc,
};
mod application;
mod control;
mod evidence;
mod execution;
mod host;
mod primitives;

pub(super) use control::{error, invariant, succeeded, Control, Failure, R};
use evidence::{contains_open_row, RowInstances};
pub(super) use evidence::{fun_arg, inner_type};
pub use host::{
    run, run_with_host, run_with_host_options, run_with_io, run_with_options, run_with_output,
    view_entry, view_with_host, view_with_host_options, view_with_io, view_with_options,
    view_with_output,
};
pub(crate) use host::{
    run_options_prevalidated, run_with_host_options_prevalidated, view_options_prevalidated,
    view_with_host_options_prevalidated,
};
pub(super) use primitives::operation_arity;

#[derive(Default)]
struct Frame {
    parent: Option<usize>,
    ready: HashMap<SymbolId, V>,
    type_scope: Option<usize>,
    substs: HashMap<u32, Type>,
    row_substs: Option<Box<RowInstances>>,
}

pub(in crate::hir_eval) struct Eval<'a> {
    pub(in crate::hir_eval) p: &'a HirProgram,
    frames: Vec<Frame>,
    pub(in crate::hir_eval) input: &'a mut dyn RuntimeInput,
    pub(in crate::hir_eval) output: &'a mut dyn Write,
    pub(in crate::hir_eval) stdin: &'a mut dyn BufRead,
    pub(in crate::hir_eval) args: &'a [String],
    pub(in crate::hir_eval) fuel: usize,
    pub(in crate::hir_eval) depth: u16,
    pub(in crate::hir_eval) max_depth: u16,
    effects: u16,
}

impl Eval<'_> {
    fn frame(&mut self, parent: usize) -> usize {
        let id = self.frames.len();
        self.frames.push(Frame {
            parent: Some(parent),
            ready: HashMap::new(),
            type_scope: self.frames[parent].type_scope,
            substs: HashMap::new(),
            row_substs: None,
        });
        id
    }
    fn put(&mut self, env: usize, s: SymbolId, v: V) {
        self.frames[env].ready.insert(s, v);
    }
    fn load(&self, mut env: usize, s: SymbolId, span: Span) -> R<V> {
        loop {
            if let Some(v) = self.frames[env].ready.get(&s) {
                return Ok(v.clone());
            }
            env = self.frames[env]
                .parent
                .ok_or_else(|| error(span, "value used before initialization"))?;
        }
    }
    fn define_fun(&mut self, d: &Definition, env: usize) {
        let code = Function::User {
            formals: d.formals.clone().into(),
            body: d.body,
            env,
            declared: d.scheme.ty.clone(),
            open_rows: contains_open_row(&d.scheme.ty),
        };
        self.put(env, d.symbol, function(code));
    }
    fn make_variant(&self, id: SymbolId, fields: Vec<V>, span: Span) -> R<V> {
        let info = self
            .p
            .constructors
            .get(&id)
            .ok_or_else(|| invariant(span, "checked constructor identity is absent"))?;
        if fields.len() != info.payload.len() {
            return Err(invariant(span, "checked constructor payload arity differs"));
        }
        Ok(V::Variant(Rc::new(VariantValue {
            constructor: id,
            name: info.name.clone(),
            fields: fields.into(),
        })))
    }
    pub(in crate::hir_eval) fn tick(&mut self, span: Span) -> R<()> {
        self.charge_work(1, span)
    }
    pub(in crate::hir_eval) fn charge_work(&mut self, units: usize, span: Span) -> R<()> {
        subtract_work(&mut self.fuel, units).map_err(|message| error(span, message))
    }
}
