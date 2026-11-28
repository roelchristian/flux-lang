//! Delimiter-aware declaration discovery and callable arity hoisting.

use super::{Binding, PResult, Parser, Prelude, DIRECT_ARITY_LIMIT};
use crate::{
    lexer::{is_ident, Delimiter},
    Error,
};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

impl Parser {
    pub(super) fn end_of_group(&self, from: usize) -> Option<usize> {
        let mut ends = vec![Delimiter::opening(self.at(from))?];
        for i in from + 1..self.toks.len() {
            let s = self.at(i);
            if let Some(c) = Delimiter::opening(s) {
                ends.push(c);
            } else if let Some(end) = Delimiter::closing(s) {
                if ends.pop() != Some(end) {
                    return None;
                }
                if ends.is_empty() {
                    return Some(i);
                }
            }
        }
        None
    }
    // Find a delimiter without looking through nested parenthesis, sequence or brace groups.
    pub(super) fn find(&self, mut i: usize, wanted: &[&str], stops: &[&str]) -> Option<usize> {
        while i < self.toks.len() {
            let s = self.at(i);
            if wanted.contains(&s) {
                return Some(i);
            }
            if stops.contains(&s) || Delimiter::closing(s).is_some() {
                return None;
            }
            if Delimiter::opening(s).is_some() {
                i = self.end_of_group(i)? + 1;
            } else {
                i += 1;
            }
        }
        None
    }
    fn type_head(&self, i: usize) -> Option<usize> {
        if self.at(i + 1) == "::" {
            return Some(i + 1);
        }
        if self.at(i + 1) != "[" || self.toks[i].end != self.toks[i + 1].start {
            return None;
        }
        let end = self.end_of_group(i + 1)?;
        (self.at(end + 1) == "::").then_some(end + 1)
    }
    pub(super) fn arity_header_at(&self, i: usize) -> bool {
        self.at(i + 1) == "/" && matches!(self.at(i + 3), ":=" | ":" | ":#" | "|")
    }
    pub(super) fn declared_arity(&self, i: usize) -> PResult<(usize, Range<usize>)> {
        let [name, slash, count] = &self.toks[i..i + 3] else {
            unreachable!("arity candidates have at least three tokens");
        };
        if name.end != slash.start || slash.end != count.start {
            return Err(Error::new(
                slash.start,
                "write callable arity adjacent as `name/arity`",
            ));
        }
        let arity = count.text.bytes().all(|byte| byte.is_ascii_digit())
            .then(|| count.text.parse::<usize>().ok())
            .flatten()
            .filter(|n| (1..=DIRECT_ARITY_LIMIT).contains(n))
            .ok_or_else(|| Error::new(count.start, format!(
                "callable arity must be an unsuffixed positive integer from 1 to {DIRECT_ARITY_LIMIT}"
            )))?;
        Ok((arity, count.start..count.end))
    }
    pub(super) fn def_at(&self, i: usize) -> bool {
        if !is_ident(self.at(i)) {
            return false;
        }
        match self.at(i + 1) {
            ":=" | "::" => true,
            "[" => self.type_head(i).is_some(),
            "/" => self.arity_header_at(i),
            ":" | ":#" => self.find(i + 2, &[":="], &[";"]).is_some(),
            "(" => self.end_of_group(i + 1).is_some_and(|j| {
                self.at(j + 1) == ":="
                    || (matches!(self.at(j + 1), ":" | ":#")
                        && self.find(j + 2, &[":="], &[";"]).is_some())
            }),
            _ => false,
        }
    }
    pub(super) fn prepass(&self, mut i: usize, root: bool) -> PResult<Prelude> {
        let mut names = HashSet::new();
        let mut type_names = HashSet::new();
        let mut hoisted = HashMap::new();
        loop {
            let n = self.at(i).to_owned();
            if matches!(n.as_str(), "" | "}") {
                break;
            }
            if self.def_at(i) {
                let type_head = self.type_head(i);
                let is_type = type_head.is_some();
                if is_type && n == "array" {
                    return Err(Error::new(
                        self.toks[i].start,
                        "`array` is a built-in type name and cannot be redeclared",
                    ));
                }
                let is_sum =
                    type_head.is_some_and(|h| self.at(h + 1) == "{" && self.at(h + 2) == "|");
                let head = type_head.unwrap_or(i + 1);
                if is_type && !type_names.insert(n.clone()) {
                    return Err(Error::new(
                        self.toks[i].start,
                        format!("duplicate type `{n}`"),
                    ));
                }
                let b = match self.at(head) {
                    "::" if is_sum => {
                        let mut j = head + 2;
                        while self.at(j) == "|" {
                            let (v, next) = self.variant_at(j)?;
                            if !names.insert(v.name.clone()) {
                                return Err(Error::new(
                                    v.start,
                                    format!("duplicate binding `{}`", v.name),
                                ));
                            }
                            hoisted.insert(v.name, Binding::Constructor(v.payload.len()));
                            j = next;
                        }
                        None
                    }
                    "::" => Some(Binding::Record),
                    "/" => Some(Binding::Callable(self.declared_arity(i)?.0)),
                    "(" => {
                        let end = self.end_of_group(i + 1).ok_or_else(|| {
                            Error::new(self.toks[i].start, "unclosed parameter list")
                        })?;
                        let mut j = i + 2;
                        let mut count = usize::from(j < end);
                        while j < end {
                            if self.at(j) == "," {
                                count += 1;
                            }
                            if Delimiter::opening(self.at(j)).is_some() {
                                j = self.end_of_group(j).unwrap_or(j);
                            }
                            j += 1;
                        }
                        Some(Binding::Callable(count))
                    }
                    _ if root => Some(Binding::Value),
                    _ => None,
                };
                if !is_sum && !names.insert(n.clone()) {
                    return Err(Error::new(
                        self.toks[i].start,
                        format!("duplicate binding `{n}`"),
                    ));
                }
                if let Some(b) = b {
                    hoisted.insert(n, b);
                }
            }
            let Some(end) = self.find(i, &[";"], &[]) else {
                break;
            };
            i = end + 1;
        }
        Ok((hoisted, type_names))
    }
}
