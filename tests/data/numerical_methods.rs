use super::mathematical_library::{close, outputs, specimen};
use super::script::{self, Script};

const IMPORTS: &str = r#"<< "math"; << "num";"#;
const METHODS: &str = r#"
opts := .{ max:=4096n; tol:=1e-12r; };
f(x:r):r := - (* x x) 2r;
df(x:r):r := * 2r x;
square(x:r):r := * x x;
line(x:r):r := - x 2r;
constant(x:r):r := 1r;
zero(x:r):r := 0r;
positive(x:r):r := + (* x x) 1r;
crash(x:r):r := / x 0r;
at_endpoint(x:r):r := ?{ = x 2r : 0r; _ : crash x; };
"#;

fn check_values(program: &Script, values: &[f64]) {
    program.check_hir();
    for mode in [None, Some("run-hir")] {
        let result = program.run(mode);
        assert!(
            result.status.success() && result.stderr.is_empty(),
            "{mode:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8(result.stdout).unwrap();
        assert_eq!(output.lines().count(), values.len(), "{output}");
        for (text, &value) in output.lines().zip(values) {
            close(
                text.strip_prefix("ok ").unwrap_or_else(|| panic!("{text}")),
                value,
            );
        }
    }
}

#[test]
fn bounded_methods_converge_and_accept_checked_callbacks() {
    let program = outputs(
        IMPORTS,
        METHODS,
        &[
            "num.bisect &f 0r 2r opts",
            "num.newton &f &df 1r opts",
            "num.integrate &square 0r 1r opts",
            "num.integrate &math.sin 0r math.pi opts",
            "num.interp [0r 2r 5r] [1r 5r 11r] 3r",
            "num.diff &square 3r 0.125r",
            "num.bisect &line 2r 3r opts",
            "num.bisect &line 1r 2r opts",
            "num.interp [0r 2r 5r] [1r 5r 11r] 5r",
            "@@ &num.diff (&square,3r,0.125r)",
            "@@ &num.newton (&f,&df,1r,opts)",
            "num.bisect &line 2r 2r opts",
            "num.bisect &at_endpoint 2r 3r opts",
            "num.newton &line &crash 2r opts",
            "num.integrate &crash 1r 1r opts",
            "num.integrate &square 1r 0r opts",
            "num.interp [-1.7e308r 1.7e308r] [-1.7e308r 1.7e308r] 0r",
            "num.diff &line 0r 1.7e308r",
        ],
    );
    check_values(
        &program,
        &[
            2.0_f64.sqrt(),
            2.0_f64.sqrt(),
            1.0 / 3.0,
            2.0,
            7.0,
            6.0,
            2.0,
            2.0,
            11.0,
            6.0,
            2.0_f64.sqrt(),
            2.0,
            2.0,
            2.0,
            0.0,
            -1.0 / 3.0,
            0.0,
            1.0,
        ],
    );
}

#[test]
fn a_solver_failure_is_a_recoverable_nominal_result() {
    outputs(
        IMPORTS,
        METHODS,
        &[
            "num.bisect &positive 0r 2r opts",
            "num.newton &constant &zero 1r opts",
            "num.diff &square 3r 0r",
            "num.diff &square 3r -0.25r",
            "num.bisect &f 0r 2r .{tol:=0r;max:=10n;}",
            "num.bisect &f 0r 2r .{tol:=1e-12r;max:=0n;}",
            "num.interp [0r] [1r] 0r",
            "num.interp [0r 1r] [1r 2r 3r] 0.5r",
            "num.interp [0r 0r] [1r 2r] 0r",
            "^= num.newton &constant &zero 1r opts { singular : 37r; _ : 0r; }",
            "num.diff &square 1.7e308r 1.7e308r",
            "num.interp [0r 1r] [1r 2r] 2r",
            "num.bisect &line 3r 2r opts",
            "num.bisect &line 3r 3r opts",
            "num.diff &square 1e20r 1r",
        ],
    )
    .accepts(concat!(
        "err badbracket\nerr singular\nerr badparams\nerr badparams\nerr badparams\n",
        "err badparams\nerr badparams\nerr badparams\nerr badparams\n37\nerr nonfinite\n",
        "err badparams\nerr badbracket\nerr badbracket\nerr badparams\n"
    ));
}

#[test]
fn solver_max_stops_search_and_oversized_requests_cannot_wrap() {
    outputs(
        IMPORTS,
        METHODS,
        &[
            "num.bisect &f 0r 2r .{tol:=1e-14r;max:=1n;}",
            "num.newton &f &df 1r .{tol:=1e-14r;max:=1n;}",
            "num.integrate &math.sin 0r math.pi .{tol:=1e-14r;max:=1n;}",
            "num.bisect &f 0r 2r .{tol:=1e-14r;max:=99999999999999999999999999999999999n;}",
        ],
    )
    .accepts("err noconverge\nerr noconverge\nerr budget\nerr budget\n");
}

#[test]
fn numerical_callbacks_cannot_hide_effects_or_swallow_fatal_errors() {
    for line in [
        r#"x := num.diff (\x -> print x) 1r 0.1r;"#,
        r#"x := num.diff (\x -> ^{ x }) 1r 0.1r;"#,
        "x := num.diff &square 1q 0.1r;",
        "x := num.bisect &f 0r 2r .{tol:=1e-10r;extra:=1n;max:=100n;};",
    ] {
        let source = format!("{IMPORTS}\n{METHODS}\n{line}");
        let code = if line.contains("^{") {
            "E0401"
        } else {
            "E0200"
        };
        script::rejects(&specimen(&source), &source, &[code]);
    }
    let callback = specimen(
        r#"
<< "num";
crash(x:r):r := / x 0r;
!^{
    ! print "before";
    recovered := ^= num.diff &crash 1r 0.25r { _ : 0r; };
    ! print recovered;
};
"#,
    );
    callback.check_hir();
    for mode in [None, Some("run-hir")] {
        let result = callback.run(mode);
        let diagnostic = String::from_utf8_lossy(&result.stderr);
        assert!(
            !result.status.success() && result.stdout == b"before\n",
            "{result:?}"
        );
        assert!(
            diagnostic.contains("division by zero") && diagnostic.contains("test.flx:3:"),
            "{diagnostic}"
        );
    }
}
