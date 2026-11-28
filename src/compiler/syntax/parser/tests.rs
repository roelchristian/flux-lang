use super::*;

#[test]
fn entry_start_identifies_opening_bang_after_declarations() {
    let source = "x := 2; -- explanatory comment\n    !{ x };";
    assert_eq!(parse(source).unwrap().entry_start, source.find("!{"));
    assert_eq!(parse("x := 2;").unwrap().entry_start, None);
    assert_eq!(parse("! { (); };").unwrap().entry_start, Some(0));
}

#[test]
fn arity_missing_surplus_and_strict_prefix() {
    assert!(parse("p(x,y:n):n := + x y; n := p 2 3;").is_ok());
    for bad in [
        "n := + 2;",
        "n := + 2 3 4;",
        "n := 2 + 3;",
        "p(x,y:n):n := + x y; n := p(2,3);",
    ] {
        assert!(parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn callable_hoisting_and_lexical_shadowing() {
    let p = parse("a(x:n):n := b x; b(x:n):n := x; k(avg:n):n := avg;                        y := { a := 4; f(x:n):n := + a x; f 3 };").unwrap();
    assert!(matches!(&p.defs[2], Stmt::Def { body: Expr::Name(n), .. } if n == "avg"));
    assert!(parse("n := { f(x:n):n := + x k; k := 2; f 1 };").is_err());
    assert!(parse("n := { k := 1; k := 2; k };").is_err());
}

#[test]
fn module_arity_and_terminal_anonymous_effect() {
    let p = parse("x := &say.print; << \"io\" : say; !{ ! @ x 1; };").unwrap();
    assert_eq!(p.imports, vec!["io"]);
    assert!(matches!(&p.defs[0], Stmt::Def { body: Expr::Ref(n), .. } if n == "io.print"));
    assert!(matches!(p.entry, Some(Expr::Action(_, None))));
    assert!(parse("!{ (); }; x := 1;").is_err());
    assert!(parse("!{ (); }; !{ (); };").is_err());
    assert!(parse("x := ! 1;").is_err());
    assert!(parse("!{ ! !> 1; };").is_err());
}

#[test]
fn declaration_types_and_guards() {
    assert!(parse("pt :: { x:q; y:q; }; x := pt { x:=2; y:=3; };                        n:#[s n] := #[ (\"x\",1) ];                        f(x,y:(n,n),k:n):n := ? { > k 1 : .1 x; _ : .2 y; };").is_ok());
    assert!(parse(
        "pt :: { x:q; }; identity(t:#pt):#pt := t; \
                       !{ t:#pt := #[ pt { x:=2; } ]; # (:# identity t) };"
    )
    .is_ok());
    assert!(parse("f(x:q,y):q := + x y;").is_err());
    assert!(parse(">> (f); f(x:q) := * x x;").is_err());
    assert!(parse("x := ? { 1b : 1; };").is_err());
    assert!(parse("x := {. x:=3; };").is_err());
}

#[test]
fn aesthetic_core_forms_parse_with_lexical_pipeline_scope() {
    let p = parse(
        r#"<< csv;
                          << "lib/df.flx" : frame;
                          a := ?{ 1b : .{ x := 7; }; _ : .{ x := 8; }; };
                          p := |{ 2; |{ $; + $ 1 }; * $ 3 };
                          q := \x -> + x 1;
                          r := \(x,y:q) -> + x y;
                          s := frame.extend &rev [1 2];
                          !{ ! csv.write "out.csv" []; };
                       "#,
    )
    .unwrap();
    assert_eq!(p.imports, vec!["csv", "df"]);
    assert!(matches!(&p.defs[1], Stmt::Def { body: Expr::Pipeline(es), .. } if es.len()==3));
    assert!(matches!(
        &p.defs[2],
        Stmt::Def {
            body: Expr::Lambda(..),
            ..
        }
    ));
    assert!(matches!(
        &p.defs[3],
        Stmt::Def {
            body: Expr::TypedLambda(..),
            ..
        }
    ));
    assert!(matches!(&p.defs[4], Stmt::Def { body: Expr::Call(n,..), .. } if n=="df.extend"));
    let Stmt::Def {
        body: Expr::Cases(rows, _),
        ..
    } = &p.defs[0]
    else {
        panic!()
    };
    assert!(matches!(&rows[0].1, Expr::Record(_)));
}

#[test]
fn new_forms_reject_missing_operands_and_illegal_stage_binding() {
    for s in [
        "v := $;",
        "v := |{ $; 1 };",
        "v := |{ 1 };",
        "v := |{ 1; };",
        "v := |{ 1; $; };",
        "v := |{ 1; + $ };",
        "v := |{ 1; + $ 2 3 };",
        "v := take 2;",
        "v := drop 2 [1] [2];",
        "v := slice 1 [2];",
        "v := .{ a:=2; a:=3; };",
        "v := {. a:=2; };",
        r"v := \(x:q,y) -> x;",
        "<< io; << \"lib/io.flx\";",
        "<< \"lib/bad-name.flx\";",
    ] {
        assert!(parse(s).is_err(), "unexpected acceptance: {s}");
    }
    assert!(parse("take(n:n):n := n; y := take 2;").is_ok());
}
#[test]
fn fallible_block_requires_tail_but_allows_statements() {
    assert!(parse("x := ^{ 1; () };").is_ok());
    assert!(parse("x := ^{ 1; };").is_err());
    assert!(parse("x := ^{};").is_err());
    assert!(parse("!{ (); };").is_ok());
}
#[test]
fn annotation_ast_preserves_caret_arrow_precedence() {
    use TypeAnnotationKind::*;
    let source = "?q^?err n -> !(q^err) -> [q^err]";
    let t = parse_type_annotation(source).unwrap();
    assert_eq!(t.span, 0..source.len());
    let Function(args, tail) = t.kind else {
        panic!()
    };
    assert_eq!(args.len(), 2);
    let Result(ok, err) = &args[0].kind else {
        panic!()
    };
    assert_eq!(&source[ok.span.clone()], "?q");
    assert_eq!(&source[err.span.clone()], "?err");
    assert!(matches!(ok.kind, Optional(_)));
    assert!(matches!(err.kind, Optional(_)));
    let Function(args, ret) = tail.kind else {
        panic!()
    };
    let Action(inner) = &args[0].kind else {
        panic!()
    };
    assert!(matches!(inner.kind, Result(..)));
    assert_eq!(&source[inner.span.clone()], "(q^err)");
    let Sequence(inner) = &ret.kind else { panic!() };
    assert!(matches!(inner.kind, Result(..)));
    let bad = parse_type_annotation("q^err^e").unwrap_err();
    assert_eq!(bad.start, 5);
    assert!(bad.message.contains("parentheses"));
}

#[test]
fn declarations_store_canonical_types_with_source_offsets() {
    use TypeAnnotationKind::*;
    let source = "row :: { x:q; }; err :: { | bad (q^err); }; f(x,y:~a,g:.[z]):z := 0;";
    let source = source.replace(".[z]", ".{data:#[z q]; tag:?{s:z};}");
    let p = parse(&source).unwrap();
    let Stmt::Schema { fields, .. } = &p.defs[0] else {
        panic!()
    };
    assert_eq!(&source[fields[0].1.span.clone()], "q");
    let Stmt::Sum { variants, .. } = &p.defs[1] else {
        panic!()
    };
    assert!(matches!(variants[0].payload[0].kind, Result(..)));
    assert_eq!(&source[variants[0].payload[0].span.clone()], "(q^err)");
    let Stmt::Def {
        param_types: ts,
        result_type: Some(out),
        ..
    } = &p.defs[2]
    else {
        panic!()
    };
    assert_eq!(&source[ts[0].as_ref().unwrap().span.clone()], "~a");
    assert_eq!(ts[0], ts[1]);
    let Record(fields) = &ts[2].as_ref().unwrap().kind else {
        panic!()
    };
    assert!(matches!(fields[0].1.kind, PositionalTable(_)));
    assert_eq!(&source[fields[0].1.span.clone()], "#[z q]");
    let Optional(inner) = &fields[1].1.kind else {
        panic!()
    };
    assert!(matches!(inner.kind, Dictionary(..)));
    assert_eq!(&source[out.span.clone()], "z");
    assert!(matches!(parse_type_annotation("_").unwrap().kind, Hole));
}

#[test]
fn malformed_annotation_reports_original_token() {
    let source = "-- comment
f(x:q^err^e):q := 0;";
    let problem = parse(source).unwrap_err();
    assert_eq!(problem.start, source.rfind('^').unwrap());
    assert!(problem.message.contains("parentheses"));
}
#[test]
fn source_module_links_only_exports_and_rebases_origins() {
    let library_src = ">> (f,row,err); f(x:q):q := + x private; private := 2; row :: { x:q; }; err :: { | bad n; | zero; };";
    let lib = parse_module(library_src, 200, Some("library"), &[]).unwrap();
    let members = lib.module_members();
    assert_eq!(members.len(), 5);
    assert!(members.iter().all(|m| m.linked.as_bytes()[0] == 0));
    assert!(members.iter().all(|m| m.name != "private"));
    let imports = [ImportedModule {
        path: "./dep.flx".into(),
        members,
    }];
    let source = r#"<< "./dep.flx" : d; private := 9; x:d.row := d.row { x := d.f 2q; }; rr:e^d.err := ^{ ^: d.bad 4n }; e :: { val:n; }; y := ^= rr { d.bad n : e { val := n; }; d.zero : e { val := 0n; }; };"#;
    let mut root = parse_module(source, 600, Some("root"), &imports).unwrap();
    assert!(parse_module(
        r#"<< "./dep.flx" : d; x := d.private;"#,
        0,
        Some("other"),
        &imports
    )
    .is_err());
    assert!(parse_module(
        r#"<< "./dep.flx" : d; x := d.f 1 2;"#,
        0,
        Some("other"),
        &imports
    )
    .is_err());
    let Stmt::Def {
        result_type: Some(t),
        ..
    } = &root.defs[1]
    else {
        panic!()
    };
    let TypeAnnotationKind::Named(n) = &t.kind else {
        panic!()
    };
    assert_eq!(
        n,
        &imports[0]
            .members
            .iter()
            .find(|m| m.name == "row")
            .unwrap()
            .linked
    );
    assert_eq!(&source[t.span.start - 600..t.span.end - 600], "d.row");
    root.merge_libraries([lib]);
    let origins = root.expression_origins();
    assert!(origins
        .iter()
        .any(|(_, s)| s.start >= 200 && s.end <= 200 + library_src.len()));
    assert!(origins
        .iter()
        .any(|(_, s)| s.start >= 600 && s.end <= 600 + source.len()));
    let res = crate::resolve::resolve(&root).unwrap();
    assert!(res.groups.len() >= 8);
}

#[test]
fn linked_nominal_scope_blocks_native_and_foreign_leakage() {
    assert!(parse_module("f(x:ioerr):ioerr := x;", 0, Some("m"), &[]).is_ok());
    assert!(parse_module("f(x:csverr):csverr := x;", 0, Some("m"), &[]).is_err());
    assert!(parse_module(r#"<< "csv"; f(x:csverr):csverr := x;"#, 0, Some("m"), &[]).is_ok());
    let p = parse_module(
        r#"row :: { x:q; }; a := { row :: { y:s; }; r:row := row {y:="ok";}; r };"#,
        100,
        Some("m"),
        &[],
    )
    .unwrap();
    let Stmt::Def {
        body: Expr::Block(ss, _),
        ..
    } = &p.defs[1]
    else {
        panic!()
    };
    let Stmt::Def {
        result_type: Some(t),
        ..
    } = &ss[1]
    else {
        panic!()
    };
    assert!(matches!(&t.kind,TypeAnnotationKind::Named(n) if n=="row"));
    let smuggled = format!("x := 0; y := {}m:x;", char::from(0));
    assert!(parse_module(&smuggled, 0, Some("m"), &[]).is_err());
}

#[test]
fn import_discovery_preserves_original_paths_aliases_and_offsets() {
    let src = r#"-- comment
           x := "! << fake;";
           << "./folder/lib.flx" : dep;
           << csv;"#;
    let imports = discover_imports(src, 500).unwrap();
    assert_eq!(imports[0].path, "./folder/lib.flx");
    assert_eq!(imports[0].alias, "dep");
    assert_eq!(imports[0].start, 500 + src.find(r#"<< "./folder"#).unwrap());
    assert_eq!(imports[1].path, "csv");
    assert_eq!(imports[1].alias, "csv");
}
#[test]
fn exact_numerals_keep_inline_threshold_and_raw_wide_spelling() {
    let source = concat!(
        "max := 170141183460469231731687303715884105727; ",
        "min := -170141183460469231731687303715884105728; ",
        "wide := 170141183460469231731687303715884105728; ",
        "below := -170141183460469231731687303715884105729; ",
        "deci := -001.23000d; ",
        "typed := + 340282366920938463463374607431768211456n 0n;"
    );
    let p = parse(source).unwrap();
    let body = |i: usize| match &p.defs[i] {
        Stmt::Def { body, .. } => body,
        _ => panic!(),
    };
    assert_eq!(body(0), &Expr::Int(i128::MAX));
    assert_eq!(body(1), &Expr::Int(i128::MIN));
    assert_eq!(
        body(2),
        &Expr::Num("170141183460469231731687303715884105728".into())
    );
    assert_eq!(
        body(3),
        &Expr::Num("-170141183460469231731687303715884105729".into())
    );
    assert_eq!(body(4), &Expr::Num("-001.23000d".into()));
    let Expr::Call(name, args) = body(5) else {
        panic!()
    };
    assert_eq!(name, "+");
    assert_eq!(args.len(), 2);
    assert!(matches!(&args[0],Expr::Num(raw) if raw.ends_with('n')));
    let spans = p.expression_origins();
    assert!(spans
        .iter()
        .any(|(e, s)| *e == body(4) && &source[s.clone()] == "-001.23000d"));
}

#[test]
fn wide_literals_do_not_change_prefix_arity_or_lexical_constraints() {
    let wide = "170141183460469231731687303715884105728";
    for bad in [
        format!("x := + {wide};"),
        format!("x := + {wide} 1 2;"),
        format!("x := :d {wide} 2;"),
        format!("x := -{wide}n;"),
        format!("x := {wide}e4d;"),
        format!("x := {wide}.0z;"),
    ] {
        assert!(parse(&bad).is_err(), "{bad}");
    }
    assert!(parse(&format!("x := :d (+ {wide} 2);")).is_ok());
    assert!(parse(&format!("x := - {wide} 2;")).is_ok());
}

#[test]
fn tacit_lowering_hygiene_and_origin_alignment() {
    let source = "identity := &.;\nvalue(x:z):z := x;\nmain := &{ @* + value (@? - .) };\n";
    let p = parse(source).unwrap();
    let Stmt::Def {
        body: Expr::Lambda(params, _),
        ..
    } = &p.defs[2]
    else {
        panic!("expected an ordinary synthetic lambda");
    };
    assert!(params[0].starts_with('\0'));
    let positions = p.expression_origins();
    assert!(positions
        .iter()
        .any(|(_, span)| &source[span.clone()] == "&."));
    assert!(positions.iter().any(
        |(node, span)| matches!(node, Expr::Ref(name) if name == "value")
            && &source[span.clone()] == "value"
    ));
    assert!(positions
        .iter()
        .any(|(_, span)| &source[span.clone()] == "(@? - .)"));
    let p = parse("fnn(x,y:z):z := - x y; alias := &{ (fnn) };").unwrap();
    assert!(matches!(&p.defs[1], Stmt::Def { body: Expr::Ref(name), .. } if name == "fnn"));
}

#[test]
fn tacit_rejects_missing_extra_and_outside_forms_with_bounded_depth() {
    for source in [
        "x := .;",
        "x := @. . .;",
        "x := @* + . .;",
        "x := @? - .;",
        "x := &{};",
        "x := &{ . . };",
        "x := &{ @. . };",
        "x := &{ @* + . };",
        "x := &{ @? - };",
        "x := &{ $ };",
        "x := &{ ! };",
        "x := &{ 0 };",
        "x := & .;",
    ] {
        assert!(parse(source).is_err(), "{source}");
    }
    let source = format!("f := &{{ {} . }};", "@. . ".repeat(300));
    assert!(parse(&source).unwrap_err().message.contains("depth limit"));
}

#[test]
fn imports_after_tacit_delimiters_are_discovered() {
    let text = "identity := &{ @. . . };\n<< \"./external.flx\" : ext;\n";
    let imports = discover_imports(text, 71).unwrap();
    assert_eq!(imports.len(), 1);
    assert_eq!(imports[0].start, text.find("<<").unwrap() + 71);
    assert_eq!(imports[0].path, "./external.flx");
}

#[test]
fn formatting_layout_preserves_original_calls_grouping_and_trivia() {
    let source = concat!(
        "n := + (* 2 3) -- between operands\n  :q 4;\n",
        "f := &{ @* + (@. .x .y) @? - . };\n"
    );
    let p = parse_with_layout(source).unwrap();
    let ordinary = parse(source).unwrap();
    assert!(ordinary.formatting_applications().is_empty());
    assert_eq!(ordinary.defs, p.defs);
    let forms = p
        .formatting_applications()
        .iter()
        .map(|a| {
            (
                &source[a.operator.clone()],
                a.operands
                    .iter()
                    .map(|r| &source[r.clone()])
                    .collect::<Vec<_>>(),
                a.tacit,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        forms,
        [
            ("*", vec!["2", "3"], false),
            (":q", vec!["4"], false),
            ("+", vec!["(* 2 3)", ":q 4"], false),
            ("@.", vec![".x", ".y"], true),
            ("@?", vec!["-", "."], true),
            ("@*", vec!["+", "(@. .x .y)", "@? - ."], true),
        ]
    );
    let app = &p.formatting_applications()[2];
    assert_eq!(
        &source[app.span.clone()],
        "+ (* 2 3) -- between operands\n  :q 4"
    );
}

#[test]
fn generic_sum_headers_payloads_and_match_origins() {
    let source = "tree[~a] :: { | leaf ~a; | node tree[~a] tree[~a]; }; f(t:tree[q]):q := ?= t { leaf x : x; node _ _ : 0q; };";
    let p = parse(source).unwrap();
    let Stmt::Sum {
        type_params,
        variants,
        ..
    } = &p.defs[0]
    else {
        panic!("expected generic sum");
    };
    assert_eq!(type_params, &["~a"]);
    assert_eq!(variants[1].payload.len(), 2);
    assert!(matches!(&variants[1].payload[0].kind,
            TypeAnnotationKind::NamedApply(name, args) if name == "tree" && args.len() == 1));
    let Stmt::Def {
        body: Expr::Match(_, arms),
        ..
    } = &p.defs[1]
    else {
        panic!("expected match");
    };
    assert_eq!(&source[arms[0].pattern.span.clone()], "leaf x");
    assert!(p
        .expression_origins()
        .iter()
        .any(|(body, span)| std::ptr::eq(*body, &arms[0].body) && &source[span.clone()] == "x"));
    let formatted = crate::format_source(source).unwrap();
    assert!(formatted.contains("tree[~a]"));
    assert!(formatted.contains("tree[q]"));
    parse(&formatted).unwrap();
    assert_eq!(crate::format_source(&formatted).unwrap(), formatted);
}

#[test]
fn rejects_duplicate_pattern_binders_and_sum_parameters() {
    for (source, fragment) in [
        ("f(x) := ?= x { (a,a) : a; };", "duplicate match binder"),
        (
            "f(x) := ?= x { .{ a := _; a := _; } : (); };",
            "duplicate match field",
        ),
        (
            "f(x) := ?= x { lib.unknown : (); };",
            "unknown or unexported match constructor",
        ),
        (
            "tree[~a,~a] :: { | leaf ~a; };",
            "duplicate sum type parameter",
        ),
        ("tree[~a] :: { field:q; };", "only on nominal sums"),
    ] {
        assert!(
            parse(source).unwrap_err().message.contains(fragment),
            "{source}"
        );
    }
    assert!(parse_type_annotation("tree [q]").is_err());
}

#[test]
fn explicit_open_row_annotation_parses_without_changing_closed_records() {
    use TypeAnnotationKind::*;
    let p = parse("keep(t:#.{ field:q; | ~r }):#.{ | ~r } := t;").unwrap();
    let Stmt::Def {
        param_types,
        result_type,
        ..
    } = &p.defs[0]
    else {
        panic!("expected function declaration");
    };
    let Table(t) = &param_types[0].as_ref().unwrap().kind else {
        panic!("expected table")
    };
    let OpenRecord(fields, row) = &t.kind else {
        panic!("expected open record")
    };
    assert_eq!(fields[0].0, "field");
    assert!(matches!(fields[0].1.kind, Primitive(TypePrimitive::Q)));
    assert_eq!(row, "~r");
    let Table(t) = &result_type.as_ref().unwrap().kind else {
        panic!("expected table")
    };
    assert!(matches!(&t.kind, OpenRecord(fields, row) if fields.is_empty() && row == "~r"));
    assert!(
        matches!(parse_type_annotation(".{ value:q }").unwrap().kind, Record(fields) if fields.len() == 1)
    );

    let input = "keep(t:#.{ field:q; x:n; | ~r }):#.{ | ~r } := t;";
    let formatted = crate::format_source(input).unwrap();
    let reparsed = parse(&formatted).unwrap();
    assert_eq!(p.defs.len(), reparsed.defs.len());
    assert!(formatted.contains("| ~r"));
    assert!(formatted.contains("field : q;"));
    assert!(formatted.contains("x     : n;"));
    assert_eq!(crate::format_source(&formatted).unwrap(), formatted);
}

#[test]
fn rejects_malformed_open_row_syntax() {
    for input in [
        ".{ | }",
        ".{ | r }",
        ".{ x:q | ~r }",
        ".{ x:q; | ~r; }",
        ".{ | ~r | ~t }",
        ".{ | ~r x:q; }",
        ".{ x:q; x:s; | ~r }",
        ".{ x:q; .. ~r }",
    ] {
        assert!(parse_type_annotation(input).is_err(), "{input}");
    }
}

#[test]
fn type_links_rewrite_open_row_fields_without_rewriting_the_tail() {
    use TypeAnnotationKind::*;
    let lib = parse_module(">> (item); item :: { v:q; };", 0, Some("dep"), &[]).unwrap();
    let members = lib.module_members();
    let linked = members[0].linked.clone();
    let imports = [ImportedModule {
        path: "./dep.flx".into(),
        members,
    }];
    let src =
        r#"<< "./dep.flx" : d; keep(x:.{ value:d.item; | ~r }):.{ value:d.item; | ~r } := x;"#;
    let program = parse_module(src, 200, Some("main"), &imports).unwrap();
    let Stmt::Def {
        result_type: Some(ty),
        ..
    } = &program.defs[0]
    else {
        panic!("expected annotated definition");
    };
    let OpenRecord(fields, row) = &ty.kind else {
        panic!("expected open record")
    };
    assert!(matches!(&fields[0].1.kind, Named(name) if name == &linked));
    assert_eq!(row, "~r");
    assert!(ty.span.start >= 200);
}

#[test]
fn built_in_array_type_needs_no_import_even_in_modules() {
    let p = parse_module(
        ">> (keep); keep(a:array[~a]):array[~a] := a; array := 1z;",
        0,
        Some("test"),
        &[],
    )
    .unwrap();
    let Stmt::Def {
        result_type: Some(ty),
        ..
    } = &p.defs[0]
    else {
        panic!("expected function")
    };
    assert!(
        matches!(&ty.kind, TypeAnnotationKind::NamedApply(name, args)
            if name == "array" && args.len() == 1)
    );
    assert!(parse("array := 1z;").is_ok());
    for bad in ["array :: { x:q; };", "array[~a] :: { | leaf ~a; };"] {
        assert!(parse(bad).unwrap_err().message.contains("built-in type"));
    }
}

#[test]
fn callable_arity_hoists_and_lowers_with_hygiene_and_origins() {
    let source = "early := pick 2z; pick/1: z z -> z := (&+);";
    let p = parse(source).unwrap();
    let Stmt::Def {
        body: Expr::Call(_, args),
        ..
    } = &p.defs[0]
    else {
        panic!("forward call not hoisted");
    };
    assert_eq!(args.len(), 1);
    let Stmt::Def {
        params,
        param_types,
        result_type,
        body,
        ..
    } = &p.defs[1]
    else {
        panic!("expected lowered callable");
    };
    assert_eq!(params.len(), 1);
    assert!(params[0].starts_with('\0'));
    assert!(matches!(
        param_types[0].as_ref().unwrap().kind,
        TypeAnnotationKind::Primitive(TypePrimitive::Z)
    ));
    assert!(
        matches!(&result_type.as_ref().unwrap().kind, TypeAnnotationKind::Function(args, _) if args.len() == 1)
    );
    let Expr::Call(op, parts) = body else {
        panic!("expected generated application")
    };
    assert_eq!(op, "@");
    assert!(matches!(&parts[0], Expr::Ref(name) if name == "+"));
    assert!(matches!(&parts[1], Expr::Name(name) if name == &params[0]));
    let origins = p.expression_origins();
    assert!(origins
        .iter()
        .any(|(e, range)| std::ptr::eq(*e, body) && &source[range.clone()] == "(&+)"));
    assert!(origins
        .iter()
        .any(|(e, range)| std::ptr::eq(*e, &parts[1]) && &source[range.clone()] == "1"));
}

#[test]
fn callable_arity_is_honored_in_nested_groups_and_module_interfaces() {
    let p = parse("main(x) := { answer := foo 1z 2z; foo/2:=&+; answer };").unwrap();
    let Stmt::Def {
        body: Expr::Block(decls, _),
        ..
    } = &p.defs[0]
    else {
        panic!("local block")
    };
    assert!(
        matches!(&decls[0], Stmt::Def { body: Expr::Call(name, args), .. }
            if name == "foo" && args.len() == 2)
    );
    let lib = parse_module(">> (pick); pick/2:z -> z -> z:=&+;", 100, Some("lib"), &[]).unwrap();
    assert_eq!(lib.module_members()[0].kind, MemberKind::Callable(2));
    let interface = [ImportedModule {
        path: "./lib.flx".into(),
        members: lib.module_members(),
    }];
    let root = parse_module(
        r#"<< "./lib.flx" : m; x := m.pick 3z 7z;"#,
        200,
        Some("main"),
        &interface,
    )
    .unwrap();
    assert!(
        matches!(&root.defs[0], Stmt::Def { body: Expr::Call(_, args), .. } if args.len() == 2)
    );
}

#[test]
fn callable_arity_rejects_non_addressable_headers_and_right_hand_sides() {
    for (source, fragment) in [
        ("f /2 := &+;", "adjacent"),
        ("f/ 2 := &+;", "adjacent"),
        ("f/0 := &+;", "positive"),
        ("f/2n := &+;", "unsuffixed"),
        ("f/65 := &+;", "64"),
        ("f/2:q -> q := &+;", "at least 2"),
        ("f/1:_ -> q := &.;", "complete type"),
        ("f/1 | num ~a := &.;", "complete named function"),
        ("f/1 := 1z;", "right-hand side"),
        ("f/1 := (&.,1z);", "right-hand side"),
    ] {
        assert!(
            parse(source).unwrap_err().message.contains(fragment),
            "{source}"
        );
    }
    assert!(parse("x := / 4z 2z; f/1 := (&.);").is_ok());
}

#[test]
fn tilde_variables_are_adjacent_only_in_all_type_positions() {
    let p = parse(
        "tree[~a] :: { | leaf ~a; | node tree[~a] tree[~a]; }; keep/1: ~a -> ~a | eq ~a := &.;",
    )
    .unwrap();
    let Stmt::Sum {
        type_params,
        variants,
        ..
    } = &p.defs[0]
    else {
        panic!("sum")
    };
    assert_eq!(type_params, &["~a"]);
    assert_eq!(variants[0].payload[0].source(), "~a");
    let Stmt::Def { start, .. } = &p.defs[1] else {
        panic!("definition")
    };
    assert_eq!(p.constraints[start], vec![("eq".into(), "~a".into())]);
    for source in [
        "id(x:~ a):~a := x;",
        "id(x:~-- split
a):~a := x;",
        "f(x:.{ | ~ r }):q := 0q;",
        "tree[~ a] :: { | leaf ~a; };",
        "tree[~a] :: { | leaf ~ a; };",
        "f(x:~a):~a | eq ~ a := x;",
    ] {
        assert!(
            parse(source)
                .unwrap_err()
                .message
                .contains("adjacent tilde"),
            "{source}"
        );
    }
    assert!(parse("no(b:b):b := ~b; e := ~= 0.1r 1r 1r; c := 'a';").is_ok());
}

#[test]
fn sequence_symbols_share_registry_arity_and_remain_referenceable() {
    let p = parse(
            "a := [: 2n [1 2 3]; b := :] 1n a; c := sect 1n 2n a; d := ,. [a b]; e := |: d; f := @/ &+ e; g := &|:; h := &{ ,. };"
        ).unwrap();
    for (index, (name, count)) in [
        ("[:", 2),
        (":]", 2),
        ("sect", 3),
        (",.", 1),
        ("|:", 1),
        ("@/", 2),
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            matches!(&p.defs[index], Stmt::Def { body: Expr::Call(op, args), .. }
                if op == name && args.len() == count),
            "{name}"
        );
    }
    assert!(matches!(&p.defs[6], Stmt::Def { body: Expr::Ref(op), .. } if op == "|:"));
    assert!(matches!(&p.defs[7], Stmt::Def { body: Expr::Ref(op), .. } if op == ",."));
    assert!(parse("row := .{ field := 2; }; x := (1,.field row); y := (1,.{ a := 2; });").is_ok());
    assert!(parse("bad := @/ &+;")
        .unwrap_err()
        .message
        .contains("missing operand 2"));
}
