use super::*;
use crate::parser::{parse, MatchPatternKind};

#[test]
fn orders_deps_and_allows_function_sccs() {
    let p = parse("a := f 0; f(x:n):n := ? (= x 0) b (g x); g(x:n):n := f x; b := 3;").unwrap();
    assert_eq!(
        resolve(&p).unwrap().groups,
        vec![vec![3], vec![1, 2], vec![0]]
    );
}

#[test]
fn rejects_value_cycles_at_the_value_declaration() {
    let src = "f(x:n):n := a;\na := f 0;";
    let p = parse(src).unwrap();
    let e = resolve(&p).unwrap_err();
    assert_eq!(e.start, src.find("a :=").unwrap());
    assert!(e.message.contains("cyclic value dependency"));
    assert!(resolve(&parse("a := a;").unwrap()).is_err());
}

#[test]
fn resolves_declaration_site_shadowing_and_stage_captures() {
    let p = parse(
        "avg := 1; p := { f(x:n):z := + avg x; avg := 2; f 0 }; q := |{ 1; |{ $; + $ 1 }; + $ 1 };",
    )
    .unwrap();
    let r = resolve(&p).unwrap();
    let Stmt::Def {
        body: Expr::Block(stmts, _),
        ..
    } = &p.defs[1]
    else {
        panic!()
    };
    let Stmt::Def {
        body: Expr::Call(_, args),
        ..
    } = &stmts[0]
    else {
        panic!()
    };
    assert_eq!(r.reference(&args[0]), r.top[0]);
    assert_ne!(r.reference(&args[0]), r.declaration(&stmts[1]));
    let Stmt::Def {
        body: Expr::Pipeline(outer),
        ..
    } = &p.defs[2]
    else {
        panic!()
    };
    let Expr::Pipeline(inner) = &outer[1] else {
        panic!()
    };
    assert_eq!(r.reference(&inner[0]), r.placeholder(&outer[1]));
    let Expr::Call(_, ops) = &inner[1] else {
        panic!()
    };
    assert_eq!(r.reference(&ops[0]), r.placeholder(&inner[1]));
    assert_ne!(r.placeholder(&inner[1]), r.placeholder(&outer[1]));
}

#[test]
fn stable_ids_and_alias_identity_on_fresh_parse() {
    let source = r#"<< "statistics" : st;
        f(x:[q]):?q := st.mean x;
        g(y:[q]):?q := avg y;
        fit := \(x:q) -> |{ x; + $ x };
    "#;
    let p = parse(source).unwrap();
    let q = parse(source).unwrap();
    let r = resolve(&p).unwrap();
    let s = resolve(&q).unwrap();
    assert_eq!(r.symbols, s.symbols);
    assert_eq!(r.groups, s.groups);
    let (Stmt::Def { body: a, .. }, Stmt::Def { body: b, .. }) = (&p.defs[0], &p.defs[1]) else {
        panic!()
    };
    assert_eq!(r.reference(a), r.reference(b));
    assert_eq!(r.symbol(r.reference(a).unwrap()).name, "avg");
}

#[test]
fn rejects_invalid_external_arity_on_synthetic_ast() {
    let mut p = parse("x := + 1 2;").unwrap();
    let Stmt::Def {
        body: Expr::Call(_, operands),
        ..
    } = &mut p.defs[0]
    else {
        panic!()
    };
    operands.pop();
    assert!(resolve(&p)
        .unwrap_err()
        .message
        .contains("expects 2 operands"));
}

#[test]
fn expression_origins_follow_owned_ast_and_preserve_subranges() {
    let source = r#"
       t := #[ (1,2) (3,4) ];
       !{
           n := ?{ 1b : (+ 5 (- 8 7)); _ : 2; };
           |{ n; + $ 1 }
       };
    "#;
    let moved = Box::new(parse(source).unwrap().clone());
    let r = resolve(&moved).unwrap();
    let show = |e: &Expr| r.span(e).map(|range| &source[range]);
    let Stmt::Def {
        body: table @ Expr::Call(_, args),
        ..
    } = &moved.defs[0]
    else {
        panic!()
    };
    assert_eq!(show(table), Some("#[ (1,2) (3,4) ]"));
    assert_eq!(show(&args[0]), None);
    let Expr::Array(rows) = &args[0] else {
        panic!()
    };
    assert_eq!(show(&rows[0]), Some("(1,2)"));
    let entry @ Expr::Action(stmts, Some(tail)) = moved.entry.as_ref().unwrap() else {
        panic!()
    };
    assert!(show(entry).unwrap().starts_with("!{"));
    let Stmt::Def {
        body: Expr::Cases(pairs, _),
        ..
    } = &stmts[0]
    else {
        panic!()
    };
    let (guard, value) = &pairs[0];
    assert_eq!(show(guard), Some("1b"));
    assert_eq!(show(value), Some("(+ 5 (- 8 7))"));
    let Expr::Call(_, ops) = value else { panic!() };
    assert_eq!(show(&ops[0]), Some("5"));
    assert_eq!(show(&ops[1]), Some("(- 8 7)"));
    assert_eq!(show(tail), Some("|{ n; + $ 1 }"));
}

#[test]
fn changed_ast_does_not_inherit_unverified_expression_ranges() {
    let mut p = parse("q := + 1 2;").unwrap();
    let Stmt::Def {
        body: Expr::Call(_, operands),
        ..
    } = &mut p.defs[0]
    else {
        panic!()
    };
    operands[1] = Expr::Int(9);
    let r = resolve(&p).unwrap();
    let Stmt::Def {
        body: call @ Expr::Call(_, operands),
        ..
    } = &p.defs[0]
    else {
        panic!()
    };
    assert_eq!(r.span(call), None);
    assert_eq!(r.span(&operands[1]), None);
    assert_eq!(r.offset(&operands[1]), Some(0));
}
#[test]
fn rejects_known_eager_local_reads_before_ready() {
    for (source, binding) in [
        ("!{ a := f (); f(x:()):z := a; (); };", "a"),
        (
            "!{ a := f (); x := 8; f(u:()):z := g u; g(u:()):z := x; (); };",
            "x",
        ),
        ("!{ rf := &f; a := @ rf (); f(x:()):z := a; (); };", "a"),
        (
            "!{ boxed := (&f,()); a := @ (.1 boxed) (); f(x:()):z := a; () };",
            "a",
        ),
        (
            "!{ r := .{ fn := &f; }; a := @ (.fn r) (); f(x:()):z := a; () };",
            "a",
        ),
        ("!{ a := ! !{ f () }; f(x:()):z := a; (); };", "a"),
    ] {
        let p = parse(source).unwrap();
        let error = resolve(&p).unwrap_err();
        assert!(
            error.message.contains(&format!("local `{binding}`")),
            "{source}: {error:?}"
        );
    }
}

#[test]
fn accepts_latent_reads_ready_calls_and_fresh_recursive_frames() {
    for source in [
        "!{ hold := &f; x := 8; f(u:()):z := x; a := @ hold (); (); };",
        r"!{ hold := \u -> f u; x := 8; f(u:()):z := x; a := @ hold (); (); };",
        "!{ a := ? 0b (f ()) 3; f(u:()):z := a; (); };",
        "!{ a := ?? (f ()) (?1 3); f(u:()):z := a; (); };",
        "!{ f(n:z):z := ? (= n 0) 0 (g (- n 1)); g(n:z):z := f n; a := f 2; (); };",
        "f(n:z):z := { y := ? (= n 0) 1 (f (- n 1)); y }; a := f 2;",
    ] {
        let p = parse(source).unwrap();
        assert!(resolve(&p).is_ok(), "{source}: {:?}", resolve(&p));
    }
}

#[test]
fn sums_hoist_constructor_arity_and_recover_lexical_binders() {
    let src = "use(u:()):q^err := ^{ ^: bad 7 \"oops\" }; err :: { | empty; | bad n s; }; result := ^= (use ()) { bad x _ : x; empty : 0; };";
    let p = parse(src).unwrap();
    let r = resolve(&p).unwrap();
    assert_eq!(r.groups[0], vec![1]);
    let Stmt::Sum { variants, .. } = &p.defs[1] else {
        panic!()
    };
    let ctor = r.variant(&variants[1]).unwrap();
    assert_eq!(r.symbol(ctor).kind, SymbolKind::Constructor(2));
    assert_eq!(r.constructor_owner(ctor), r.declaration(&p.defs[1]));
    let Stmt::Def {
        body: Expr::Recover(_, arms),
        ..
    } = &p.defs[2]
    else {
        panic!()
    };
    assert_eq!(r.recovery_constructor(&arms[0]), Some(ctor));
    assert_eq!(
        r.recovery_bindings(&arms[0]),
        &[r.reference(&arms[0].body).unwrap()]
    );
    assert_eq!(r.span(&arms[0].body).map(|range| &src[range]), Some("x"));
    assert_eq!(r.recovery_constructor(&arms[1]), r.variant(&variants[0]));
    assert!(parse("e :: { | b n; }; x := b;").is_err());
    assert!(parse("e :: { | b n n; }; x := ^= ^{0} { b y y : 1; };").is_err());
}

#[test]
fn standard_variants_require_import_and_are_shadowable() {
    let alias = parse(r#"<< "io" : fs; e := fileerr (missing "path");"#).unwrap();
    let alias_res = resolve(&alias).unwrap();
    let Stmt::Def { body, .. } = &alias.defs[0] else {
        panic!()
    };
    assert_eq!(
        alias_res.reference(body),
        alias_res.standard_constructor("csverr", "fileerr")
    );
    assert!(parse("x := missing \"x\";").is_ok());
    assert!(parse("x := invalidjson \"x\";").is_err());
    let p = parse("<< csv; x := missing \"x\"; y := { missing := 2; missing };").unwrap();
    let r = resolve(&p).unwrap();
    let Stmt::Def { body, .. } = &p.defs[0] else {
        panic!()
    };
    let id = r.reference(body);
    assert_eq!(id, r.standard_constructor("ioerr", "missing"));
    assert_eq!(r.constructor_owner(id.unwrap()), r.standard_owner("ioerr"));
    let Stmt::Def {
        body: Expr::Block(ss, Some(tail)),
        ..
    } = &p.defs[1]
    else {
        panic!()
    };
    assert_eq!(r.reference(tail), r.declaration(&ss[0]));
}

#[test]
fn nested_match_binders_have_arm_local_identity() {
    let p = parse("tree[~a] :: { | leaf ~a; | node tree[~a] tree[~a]; }; f(t:tree[q]):q := ?= t { leaf x : x; node (leaf x) _ : x; _ : 0q; };").unwrap();
    let res = resolve(&p).unwrap();
    let Stmt::Def {
        body: Expr::Match(_, arms),
        ..
    } = &p.defs[1]
    else {
        panic!("expected match");
    };
    let MatchPatternKind::Constructor(_, first) = &arms[0].pattern.kind else {
        panic!("expected constructor");
    };
    let MatchPatternKind::Constructor(_, second) = &arms[1].pattern.kind else {
        panic!("expected constructor");
    };
    let MatchPatternKind::Constructor(_, nested) = &second[0].kind else {
        panic!("expected nested constructor");
    };
    assert!(res.match_constructor(&arms[0].pattern).is_some());
    assert_eq!(
        res.match_constructor(&arms[0].pattern),
        res.match_constructor(&second[0])
    );
    let l = res.match_binding(&first[0]).unwrap();
    let r = res.match_binding(&nested[0]).unwrap();
    assert_ne!(l, r);
    assert_eq!(res.reference(&arms[0].body), Some(l));
    assert_eq!(res.reference(&arms[1].body), Some(r));
}

#[test]
fn eager_resolution_follows_only_the_statically_selected_match_arm() {
    let source = "color :: { | red; | blue; }; !{ safe := ?= COLOR { red : 1z; blue : read (); }; delayed := 7z; read(x:()):z := delayed; () };";
    let good = parse(&source.replace("COLOR", "red")).unwrap();
    resolve(&good).unwrap();
    let bad = parse(&source.replace("COLOR", "blue")).unwrap();
    assert!(resolve(&bad)
        .unwrap_err()
        .message
        .contains("before its initializer"));
}

#[test]
fn checks_eager_reads_through_known_match_payload_bindings() {
    for (header, value, pattern) in [
        ("", "(&f,())", "(g,())"),
        ("box :: { | wrap (()->z); };", "wrap &f", "wrap g"),
        ("", "?1 &f", "?1 g"),
        ("", ".{ get := &f; }", ".{ get := g; }"),
    ] {
        let fallback = if pattern == "?1 g" { "_ : 0z;" } else { "" };
        let eager = format!("{header} !{{ a := ?= {value} {{ {pattern} : @ g (); {fallback} }}; f(x:()):z := a; () }};");
        let p = parse(&eager).unwrap();
        assert!(
            resolve(&p)
                .unwrap_err()
                .message
                .contains("before its initializer"),
            "{eager}"
        );

        let deferred = format!(
            r"{header} !{{ a := ?= {value} {{ {pattern} : \x -> @ g x; }}; f(x:()):z := @ a x; () }};"
        );
        resolve(&parse(&deferred).unwrap()).unwrap();
    }
}
