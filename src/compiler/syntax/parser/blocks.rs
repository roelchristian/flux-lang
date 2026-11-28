//! Scoped statement sequences and pure, action and fallible blocks.

use super::{Expr, Origin, PResult, Parser, Stmt};

impl Parser {
    fn scoped_block(
        &mut self,
        effect: bool,
        sequence: bool,
        implicit_unit: bool,
        auto_propagate: bool,
    ) -> PResult<(Vec<Stmt>, Option<Box<Expr>>)> {
        let (bindings, types) = self.prepass(self.pos, false)?;
        self.scopes.push(bindings);
        self.type_scopes
            .push(types.into_iter().map(|n| (n.clone(), n)).collect());
        let result = self.block_sequence(effect, sequence, implicit_unit, auto_propagate);
        self.scopes.pop();
        self.type_scopes.pop();
        result
    }

    pub(super) fn block(&mut self, effect: bool) -> PResult<Expr> {
        let (stmts, last) = self.scoped_block(effect, effect, effect, false)?;
        Ok(if effect {
            Expr::Action(stmts, last)
        } else {
            Expr::Block(stmts, last)
        })
    }

    pub(super) fn fallible(&mut self, effect: bool) -> PResult<Expr> {
        let (stmts, last) = self.scoped_block(effect, true, false, false)?;
        Ok(Expr::Fallible(stmts, last))
    }

    pub(super) fn combined_action(&mut self, start: usize) -> PResult<Expr> {
        let (stmts, last) = self.scoped_block(true, true, true, true)?;
        let body = Expr::FallibleActionBody(stmts, last);
        self.origins
            .push(Origin::new(&body, Some(start..self.toks[self.pos - 1].end)));
        Ok(Expr::Action(Vec::new(), Some(Box::new(body))))
    }

    fn block_sequence(
        &mut self,
        effect: bool,
        sequence: bool,
        implicit_unit: bool,
        auto_propagate: bool,
    ) -> PResult<(Vec<Stmt>, Option<Box<Expr>>)> {
        let mut stmts = Vec::new();
        loop {
            if self.peek() == "}" {
                if !implicit_unit {
                    return self.err("block requires a final expression (write () for unit)");
                }
                self.pos += 1;
                return Ok((stmts, None));
            }
            if self.def_at(self.pos) {
                stmts.push(self.definition(effect)?);
                continue;
            }
            let start = self.offset();
            let immediate_execution = auto_propagate && self.peek() == "!";
            let mut body = self.expr(effect)?;
            if sequence && self.eat(";") {
                if immediate_execution && matches!(&body, Expr::Call(op, _) if op == "!") {
                    body = Expr::AutoPropagate(Box::new(body));
                    self.origins
                        .push(Origin::new(&body, Some(start..self.toks[self.pos - 1].end)));
                }
                stmts.push(Stmt::Effect {
                    name: None,
                    body,
                    start,
                });
                continue;
            }
            self.expect("}")?;
            return Ok((stmts, Some(Box::new(body))));
        }
    }
}
