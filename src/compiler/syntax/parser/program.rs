//! Parser entry points, module interfaces and whole-program assembly.

use super::{
    Binding, Expr, ImportDecl, ImportedModule, MemberKind, ModuleMember, PResult, Parser, Program,
    Stmt,
};
use crate::{
    builtins,
    lexer::{decode_literal, is_ident, lex},
    standard_errors, Error,
};
use std::collections::{BTreeMap, HashMap, HashSet};

fn parser_for(
    source: &str,
    base: usize,
    namespace: Option<&str>,
    imports: &[ImportedModule],
) -> PResult<Parser> {
    let eof = source
        .len()
        .checked_add(base)
        .ok_or_else(|| Error::new(base, "source location exceeds addressable size"))?;
    let mut toks = lex(source).map_err(|e| Error::new(base.saturating_add(e.start), e.message))?;
    for t in &mut toks {
        t.start += base;
        t.end += base;
    }
    Ok(Parser {
        toks,
        pos: 0,
        eof,
        scopes: Vec::new(),
        type_scopes: Vec::new(),
        standard: standard_errors::variants("ioerr")
            .iter()
            .map(|v| (v.name.to_owned(), Binding::Constructor(v.payload.len())))
            .collect(),
        native_type_names: HashSet::from(["ioerr".to_owned(), "array".to_owned()]),
        aliases: HashMap::new(),
        source_interfaces: imports.to_vec(),
        links: HashMap::new(),
        namespace: namespace.map(str::to_owned),
        depth: 0,
        constraints: BTreeMap::new(),
        origins: Vec::new(),
        formatting_applications: None,
    })
}

/// Parse an independently scoped module, with globally rebased locations.
pub fn parse_module(
    source: &str,
    base: usize,
    namespace: Option<&str>,
    imports: &[ImportedModule],
) -> PResult<Program> {
    parser_for(source, base, namespace, imports)?.program()
}

/// Like `parse`, collecting original call ranges for source formatting.
pub fn parse_with_layout(source: &str) -> PResult<Program> {
    parse_module_with_layout(source, 0, None, &[])
}

/// Like `parse_module`, collecting layout for this source file only.
pub(crate) fn parse_module_with_layout(
    source: &str,
    base: usize,
    namespace: Option<&str>,
    imports: &[ImportedModule],
) -> PResult<Program> {
    let mut p = parser_for(source, base, namespace, imports)?;
    p.formatting_applications = Some(Vec::new());
    p.program()
}

/// Discover only top-level imports before binding interfaces. No imported
/// members or callable arities are needed for this lexical pass.
pub fn discover_imports(source: &str, base: usize) -> PResult<Vec<ImportDecl>> {
    let mut p = parser_for(source, base, None, &[])?;
    let mut imports = Vec::new();
    let mut i = 0;
    while !p.at(i).is_empty() {
        if p.at(i) == "<<" {
            let start = p.toks[i].start;
            p.pos = i + 1;
            let (path, alias, _) = p.import_decl()?;
            i = p.pos;
            imports.push(ImportDecl { path, alias, start });
        } else if let Some(end) = p.find(i, &[";"], &[]) {
            i = end + 1;
        } else {
            break;
        }
    }
    Ok(imports)
}

pub fn parse(source: &str) -> PResult<Program> {
    parse_module(source, 0, None, &[])
}

impl Parser {
    fn program(mut self) -> PResult<Program> {
        let (hoisted, type_names) = self.prepass(0, true)?;
        self.links = hoisted
            .keys()
            .map(|n| (n.clone(), self.own_name(n)))
            .collect();
        self.scopes.push(hoisted);
        self.type_scopes.push(
            type_names
                .iter()
                .map(|n| (n.clone(), self.own_name(n)))
                .collect(),
        );
        let mut used_interfaces = vec![false; self.source_interfaces.len()];
        let mut used_aliases = HashSet::new();
        let mut p = Program {
            defs: Vec::new(),
            imports: Vec::new(),
            entry: None,
            entry_start: None,
            constraints: BTreeMap::new(),
            exports: Vec::new(),
            origins: Vec::new(),
            members: Vec::new(),
            formatting_applications: Vec::new(),
        };
        let mut exports = Vec::new();
        // Resolve top-level interfaces before any expression is parsed;
        // qualified callable arity is part of the interface, not the inferred type.
        let mut i = 0;
        while !self.at(i).is_empty() {
            let Some(end) = self.find(i, &[";"], &[]) else {
                break;
            };
            if self.at(i) == "<<" {
                self.pos = i + 1;
                let (raw, alias, start) = self.import_decl()?;
                if !used_aliases.insert(alias.clone()) {
                    return Err(Error::new(
                        start,
                        format!("duplicate module alias `{alias}`"),
                    ));
                }
                if let Some((at, module)) = self
                    .source_interfaces
                    .iter()
                    .enumerate()
                    .find(|(i, m)| !used_interfaces[*i] && m.path == raw)
                {
                    used_interfaces[at] = true;
                    let members = module.members.clone();
                    for member in members {
                        let surface = format!("{alias}.{}", member.name);
                        let value = match member.kind {
                            MemberKind::Callable(n) => Some(Binding::Callable(n)),
                            MemberKind::Value => Some(Binding::Value),
                            MemberKind::Record => Some(Binding::Record),
                            MemberKind::Constructor(n) => Some(Binding::Constructor(n)),
                            MemberKind::Sum => None,
                        };
                        if let Some(b) = value {
                            self.scope().insert(surface.clone(), b);
                            self.links.insert(surface.clone(), member.linked.clone());
                        }
                        if matches!(member.kind, MemberKind::Record | MemberKind::Sum) {
                            self.type_scopes[0].insert(surface, member.linked);
                        }
                    }
                } else {
                    let native = std::path::Path::new(&raw)
                        .file_stem()
                        .and_then(|x| x.to_str())
                        .unwrap_or("");
                    if self.namespace.is_some() && (raw.starts_with("./") || raw.starts_with("../"))
                    {
                        return Err(Error::new(
                            start,
                            format!("source module `{raw}` requires a loaded interface"),
                        ));
                    }
                    let items = builtins::module_items(native)
                        .ok_or_else(|| Error::new(start, format!("unknown module `{raw}`")))?;
                    self.aliases.insert(alias.clone(), native.to_owned());
                    for (name, arity) in items {
                        self.scope()
                            .insert(format!("{alias}.{name}"), Binding::Callable(arity));
                    }
                    for &owner in standard_owners(native) {
                        self.native_type_names.insert(owner.to_owned());
                        for v in standard_errors::variants(owner) {
                            self.standard
                                .insert(v.name.to_owned(), Binding::Constructor(v.payload.len()));
                        }
                    }
                    p.imports.push(native.to_owned());
                }
            }
            i = end + 1;
        }
        self.pos = 0;
        while !self.peek().is_empty() {
            if self.eat("<<") {
                self.import_decl()?;
            } else if self.eat(">>") {
                self.expect("(")?;
                loop {
                    exports.push(self.ident()?);
                    if !self.eat(",") {
                        break;
                    }
                }
                self.expect(")")?;
                self.expect(";")?;
            } else if matches!(self.peek(), "!" | "!{" | "!^{") {
                let start = self.offset();
                let e = self.expr(false)?;
                if !matches!(e, Expr::Action(..)) {
                    return Err(Error::new(start, "expected an anonymous entry action"));
                }
                self.expect(";")?;
                if !self.peek().is_empty() {
                    return self.err("anonymous entry action must be last and unique");
                }
                p.entry = Some(e);
                p.entry_start = Some(start);
            } else if self.def_at(self.pos) {
                p.defs.push(self.definition(false)?);
            } else {
                return self.err(format!(
                    "expected declaration or anonymous entry action, found {}",
                    self.found()
                ));
            }
        }
        let mut seen = HashSet::new();
        for n in &exports {
            if !seen.insert(n.clone()) {
                return self.err(format!("duplicate export `{n}`"));
            }
            let linked = self.own_name(n);
            let mut found = false;
            for d in &p.defs {
                let kind = match d {
                    Stmt::Def {
                        name,
                        params,
                        result_type,
                        param_types,
                        ..
                    } if name == &linked => {
                        if result_type.is_none() || !param_types.iter().all(Option::is_some) {
                            return self.err(format!("export `{n}` requires a complete signature"));
                        }
                        if params.is_empty() {
                            MemberKind::Value
                        } else {
                            MemberKind::Callable(params.len())
                        }
                    }
                    Stmt::Schema { name, .. } if name == &linked => MemberKind::Record,
                    Stmt::Sum { name, variants, .. } if name == &linked => {
                        for v in variants {
                            let surface = if self.namespace.is_some() {
                                v.name.rsplit_once(':').unwrap().1
                            } else {
                                &v.name
                            };
                            p.members.push(ModuleMember {
                                name: surface.to_owned(),
                                linked: v.name.clone(),
                                kind: MemberKind::Constructor(v.payload.len()),
                            });
                        }
                        MemberKind::Sum
                    }
                    _ => continue,
                };
                found = true;
                p.members.push(ModuleMember {
                    name: n.clone(),
                    linked: linked.clone(),
                    kind,
                });
            }
            if !found {
                return self.err(format!("unknown export `{n}`"));
            }
        }
        p.constraints = self.constraints;
        p.exports = exports;
        p.origins = self.origins;
        p.formatting_applications = self.formatting_applications.unwrap_or_default();
        Ok(p)
    }
    fn import_decl(&mut self) -> PResult<(String, String, usize)> {
        let t = self.next()?;
        let raw = if t.text.starts_with('"') {
            decode_literal(&t)?
        } else if is_ident(&t.text) {
            t.text
        } else {
            return Err(Error::new(
                t.start,
                "expected module name or quoted module path",
            ));
        };
        let stem = std::path::Path::new(&raw)
            .file_stem()
            .and_then(|x| x.to_str())
            .unwrap_or("");
        let alias = if self.eat(":") {
            self.ident()?
        } else {
            if !is_ident(stem) {
                return Err(Error::new(
                    t.start,
                    "module stem is not a valid default alias; supply an explicit alias",
                ));
            }
            stem.to_owned()
        };
        self.expect(";")?;
        Ok((raw, alias, t.start))
    }
}

pub(crate) fn standard_owners(module: &str) -> &'static [&'static str] {
    match module {
        "io" => &["ioerr", "csverr"],
        "csv" => &["ioerr", "csverr"],
        "math" => &["numerr"],
        "num" => &["solvererr"],
        "fs" | "sys" => &["ioerr"],
        "rx" => &["rxerr"],
        "dt" => &["dterr"],
        "json" => &["jsonerr"],
        _ => &[],
    }
}
