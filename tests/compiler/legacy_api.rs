//! Exercise the opt-in public API as an external library consumer.
#[test]
fn legacy_oracle_remains_available_to_external_consumers() {
    use flux_lang::oracle::{check, run, Output};

    let program = flux_lang::parser::parse("x := + 2 3; !{ (); };").unwrap();
    check(&program).unwrap();
    let Output { stdout, status } = run(&program).unwrap();
    assert_eq!(status, 0);
    assert!(stdout.is_empty());
}
