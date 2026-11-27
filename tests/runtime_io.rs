use flux_lang::{
    parser,
    semantic::{prepare, Mode, Policy},
    OpenedInput, RuntimeInput,
};
use std::io::{self, Cursor, Write};

struct Input {
    payload: Vec<u8>,
    paths: Vec<String>,
}
impl RuntimeInput for Input {
    fn open(&mut self, path: &str) -> io::Result<OpenedInput> {
        self.paths.push(path.to_owned());
        Ok(OpenedInput {
            reader: Box::new(Cursor::new(self.payload.clone())),
            byte_len: None,
        })
    }
}

#[test]
fn typed_runtime_injects_input_and_preserves_completed_output_on_failure() {
    let source = r#"
<< "io";
err :: { | finish; | operational ioerr; };
unused := io.read "must-not-read";
!{
    result:()^err := ^{
        value := ^= ! io.read "virtual/input" { e : ^: operational e; };
        ^= ! io.print value { e : ^: operational e; };
        ^: finish
    };
    result
};
"#;
    let program = parser::parse(source).unwrap();
    let prepared = prepare(&program, Mode::Script, Policy::TypedOnly).unwrap();
    let mut input = Input {
        payload: b"hello".to_vec(),
        paths: Vec::new(),
    };
    let mut output = Vec::new();
    let failure = prepared
        .run_with_io(&mut input, &mut output)
        .expect_err("execution should fail");
    assert_eq!(input.paths, ["virtual/input"]);
    assert_eq!(output, b"hello\n");
    assert!(failure.message.contains("R0101") && failure.message.contains("finish"));
}

struct ClosedOutput;
impl Write for ClosedOutput {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "output stream closed",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn output_sink_failure_is_a_typed_io_error() {
    let typed = r#"
<< "io";
err :: { | sink s; };
!{ ^{
    ^= ! io.print "sent" {
        writeerr path : ^: sink path;
        e : ^: sink "unexpected";
    };
    ()
} };
"#;
    let program = parser::parse(typed).unwrap();
    let prepared = prepare(&program, Mode::Script, Policy::TypedOnly).unwrap();
    let error = prepared
        .run_with_output(&mut ClosedOutput)
        .expect_err("execution should fail");
    assert!(error.message.contains("sink stdout"), "{}", error.message);
}
