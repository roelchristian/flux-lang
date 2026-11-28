//! Layout over original lexemes, with parser-supplied prefix expression boundaries.
//! Never render a reconstructed AST: even filtered punctuation and comments
//! belong to the user's original source.
use crate::{
    lexer::{lex, Token},
    parser::FormattingApplication,
    Error,
};

const INDENT: usize = 4;
const COMPLEX_WIDTH: usize = 64;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Delimiter {
    Brace,
    Bracket,
    Paren,
}

fn opening(text: &str) -> Option<Delimiter> {
    match text {
        "{" | "!{" | "^{" | "?{" | ".{" | "|{" | "&{" | "#{" | "{." => Some(Delimiter::Brace),
        "[" | "#[" => Some(Delimiter::Bracket),
        "(" => Some(Delimiter::Paren),
        _ => None,
    }
}

fn closing(text: &str) -> Option<Delimiter> {
    match text {
        "}" => Some(Delimiter::Brace),
        "]" => Some(Delimiter::Bracket),
        ")" => Some(Delimiter::Paren),
        _ => None,
    }
}

#[derive(Default)]
struct Layout {
    before: bool,
    after: bool,
    pad: usize,
}

struct Group {
    start: usize,
    end: usize,
    delimiter: Delimiter,
    direct: Vec<usize>,
}

fn groups(tokens: &[Token]) -> Vec<Group> {
    let mut stack: Vec<Group> = vec![];
    let mut found = vec![];
    for (i, token) in tokens.iter().enumerate() {
        if let Some(close) = closing(&token.text) {
            if stack.last().is_some_and(|group| group.delimiter == close) {
                let mut group = stack.pop().unwrap();
                group.end = i;
                found.push(group);
            }
        } else {
            if let Some(parent) = stack.last_mut() {
                parent.direct.push(i);
            }
            if let Some(delimiter) = opening(&token.text) {
                stack.push(Group {
                    start: i,
                    end: 0,
                    delimiter,
                    direct: vec![],
                });
            }
        }
    }
    found
}

fn original_gap<'a>(source: &'a str, tokens: &[Token], i: usize) -> &'a str {
    let prev = i.checked_sub(1).map_or(0, |p| tokens[p].end);
    &source[prev..tokens.get(i).map_or(source.len(), |t| t.start)]
}

fn blank_line(gap: &str) -> bool {
    // CRLF is one logical break; bare CR also functions as a lexer break.
    let mut breaks = 0;
    let mut cr = false;
    for b in gap.bytes() {
        if b == b'\r' || b == b'\n' && !cr {
            breaks += 1;
            if breaks == 2 {
                return true;
            }
        }
        cr = b == b'\r';
    }
    false
}

fn line_break(gap: &str) -> bool {
    gap.contains(['\n', '\r'])
}

fn spacing(source: &str, tokens: &[Token], i: usize) -> usize {
    let a = &tokens[i - 1].text;
    let b = &tokens[i].text;
    let first = a.as_str();
    let second = b.as_str();
    let adjacent = tokens[i - 1].end == tokens[i].start;

    // Never fuse or separate reference/type punctuation whose parse may
    // depend on raw source adjacency, or manufacture a composite lexeme.
    if matches!(first, "&" | "#" | "!" | "?" | ":#") && adjacent {
        return 0;
    }
    if matches!(second, ";" | "," | ")" | "]") || matches!(first, "(" | "[" | "#[") {
        return usize::from(!adjacent && !original_gap(source, tokens, i).trim().is_empty());
    }
    if matches!(second, ":" | ":#") && adjacent {
        return 0;
    }
    if matches!(
        first,
        "{" | "!{" | "^{" | "?{" | ".{" | "|{" | "&{" | "#{" | "{."
    ) && second == "}"
    {
        return usize::from(!adjacent);
    }
    if matches!(second, "(" | "[") && adjacent && first.as_bytes()[0].is_ascii_lowercase() {
        return 0; // A declaration's parameter list or a nominal type application.
    }
    1
}

fn nominal_bracket(left: &Token, right: &Token) -> bool {
    right.text == "[" && left.text.as_bytes()[0].is_ascii_lowercase()
}

fn logical_lines(mut text: &str, mut each: impl FnMut(&str, bool)) {
    while let Some(i) = text.find(['\r', '\n']) {
        each(&text[..i], true);
        let skip = usize::from(text[i..].starts_with("\r\n")) + 1;
        text = &text[i + skip..];
    }
    each(text, false);
}

struct Output {
    text: String,
    line: bool,
}

impl Output {
    fn newline(&mut self) {
        if !self.text.is_empty() && !self.line {
            self.text.push('\n');
        }
        self.line = true;
    }
    fn blank(&mut self) {
        self.newline();
        if !self.text.is_empty() && !self.text.ends_with("\n\n") {
            self.text.push('\n');
        }
    }
    fn write(&mut self, text: &str, depth: usize, pad: usize) {
        let n = if self.line {
            self.line = false;
            depth * INDENT
        } else {
            pad
        };
        self.text.extend(std::iter::repeat_n(' ', n));
        self.text.push_str(text);
    }
}

fn comments(output: &mut Output, gap: &str, from_depth: usize, to_depth: usize) {
    let mut first = true;
    let mut pending_blank = false;
    logical_lines(gap, |line, newline| {
        // In a lexer gap, non-whitespace can only be a line comment.
        let s = line.trim_matches([' ', '\t']);
        if !s.is_empty() {
            if pending_blank {
                output.blank();
            }
            output.write(s, if first { from_depth } else { to_depth }, 1);
            // A comment is always followed by end-of-line, even at EOF.
            output.newline();
            pending_blank = false;
        } else if newline && !first {
            pending_blank = true;
        }
        if newline {
            output.newline();
        }
        first = false;
    });
    if pending_blank {
        output.blank();
    }
}

fn token_start(tokens: &[Token], byte: usize, base: usize) -> Option<usize> {
    tokens
        .binary_search_by_key(&byte.checked_sub(base)?, |t| t.start)
        .ok()
}

fn apply_prefix(
    source: &str,
    tokens: &[Token],
    widths: &[usize],
    base: usize,
    applications: &[FormattingApplication],
    layout: &mut [Layout],
    delta: &mut [isize],
) {
    let mut seen = std::collections::BTreeSet::new();
    for application in applications {
        let Some(head) = token_start(tokens, application.operator.start, base) else {
            continue;
        };
        if !seen.insert((head, application.operands.last().map(|r| r.end))) {
            continue;
        }
        let Some(last) = application.operands.last() else {
            continue;
        };
        let Some(end) = last.end.checked_sub(base) else {
            continue;
        };
        if end > tokens.last().map_or(0, |t| t.end) {
            continue;
        }
        let end = tokens.partition_point(|t| t.end <= end);
        if end <= head {
            continue;
        }
        if head > 0
            && tokens[head - 1].text == ":="
            && line_break(original_gap(source, tokens, head))
        {
            // Keep an explicitly hung right-hand side under its declaration.
            delta[head] += 1;
            delta[end] -= 1;
        }
        let already_vertical = application.operands.iter().any(|operand| {
            token_start(tokens, operand.start, base)
                .is_some_and(|start| line_break(original_gap(source, tokens, start)))
        });
        if !already_vertical && widths[end] - widths[head] <= COMPLEX_WIDTH {
            continue;
        }
        for (index, operand) in application.operands.iter().enumerate() {
            let Some(start) = token_start(tokens, operand.start, base) else {
                continue;
            };
            let Some(end_byte) = operand.end.checked_sub(base) else {
                continue;
            };
            let end = tokens.partition_point(|t| t.end <= end_byte);
            if start <= head
                || end <= start
                || (tokens[start - 1].text == "#"
                    || nominal_bracket(&tokens[start - 1], &tokens[start]))
                    && tokens[start - 1].end == tokens[start].start
            {
                continue;
            }
            if application.tacit
                && index == 0
                && end == start + 1
                && !line_break(original_gap(source, tokens, start))
            {
                // Keep a short named combiner beside the tacit sign.
                continue;
            }
            layout[start].before = true;
            delta[start] += 1;
            delta[end] -= 1;
        }
    }
}

fn structure(
    source: &str,
    tokens: &[Token],
    widths: &[usize],
    layout: &mut [Layout],
    delta: &mut [isize],
) {
    for group in groups(tokens) {
        if group.end <= group.start + 1 {
            continue;
        }
        let open = tokens[group.start].text.as_str();
        if group.start > 0
            && tokens[group.start - 1].text == ":="
            && line_break(original_gap(source, tokens, group.start))
        {
            delta[group.start] += 1;
            delta[group.end + 1] -= 1;
        }
        let vertical = group
            .direct
            .iter()
            .copied()
            .chain([group.end])
            .any(|i| line_break(original_gap(source, tokens, i)) || layout[i].before);
        let expand = match group.delimiter {
            Delimiter::Brace if open == "&{" => {
                vertical || widths[group.end + 1] - widths[group.start] > COMPLEX_WIDTH
            }
            Delimiter::Brace => {
                // Preserve the traditional compact scalar entry. Everything
                // substantial, including nested single forms, gets margins.
                !(open == "!{"
                    && group.start == 0
                    && group.end == 2
                    && !line_break(original_gap(source, tokens, 1))
                    && !line_break(original_gap(source, tokens, 2)))
            }
            Delimiter::Paren => {
                let comma = group.direct.iter().any(|&i| tokens[i].text == ",");
                let signature = tokens
                    .get(group.end + 1)
                    .is_some_and(|t| matches!(t.text.as_str(), ":" | ":=" | ":#"));
                vertical
                    || comma
                        && signature
                        && widths[group.end + 1] - widths[group.start] > COMPLEX_WIDTH
            }
            Delimiter::Bracket => vertical,
        };
        if !expand {
            continue;
        }
        layout[group.start].after = true;
        layout[group.end].before = true;
        delta[group.start + 1] += 1;
        delta[group.end] -= 1;

        if group.delimiter != Delimiter::Brace {
            for &i in &group.direct {
                if tokens[i].text == "," {
                    layout[i].after = true;
                }
            }
            continue;
        }
        for &i in &group.direct {
            if tokens[i].text == ";" {
                layout[i].after = true;
            }
        }

        // Treat a consistent group of field definitions, guards, or recovery
        // arms as a related series. Never line up unrelated local statements.
        if matches!(open, "!{" | "^{" | "|{" | "&{") {
            continue;
        }
        let mut fields = vec![];
        let mut start = group.start + 1;
        let mut invalid = false;
        for segment in group.direct.split_inclusive(|&i| tokens[i].text == ";") {
            let Some(&last) = segment.last() else {
                continue;
            };
            // An explicit open-record tail occupies its own line, but is
            // not a field and should not interrupt alignment of the prefix.
            if open == ".{"
                && matches!(segment, [bar, tail] if tokens[*bar].text == "|"
                    && tokens[*tail].text.strip_prefix('\'').is_some_and(|name|
                        name.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                        && !name.ends_with('\'')))
            {
                break;
            }
            let mut margins = segment
                .iter()
                .copied()
                .filter(|&i| matches!(tokens[i].text.as_str(), ":" | ":=" | ":#"));
            if let Some(mark) = margins.next() {
                if margins.next().is_some()
                    || mark <= start
                    || segment
                        .iter()
                        .any(|&i| i < mark && opening(&tokens[i].text).is_some())
                    || (start + 1..=mark).any(|i| original_gap(source, tokens, i).contains("--"))
                {
                    invalid = true;
                    break;
                }
                let width = widths[mark] - widths[start] - spacing(source, tokens, start);
                fields.push((mark, width));
            } else {
                invalid = true;
                break;
            }
            start = last + 1;
        }
        if !invalid && fields.len() > 1 {
            let max = fields.iter().map(|&(_, width)| width).max().unwrap();
            if max <= INDENT * 8 {
                for (mark, width) in fields {
                    layout[mark].pad = max - width + 1;
                }
            }
        }
    }
}

fn render(source: &str, tokens: &[Token], layout: &[Layout], delta: &[isize]) -> String {
    let mut output = Output {
        text: String::with_capacity(source.len()),
        line: true,
    };
    let mut depth = 0isize;
    for (i, token) in tokens.iter().enumerate() {
        let gap = original_gap(source, tokens, i);
        let previous_depth = depth;
        depth += delta[i];
        let wrapped = layout[i].before
            || i.checked_sub(1)
                .is_some_and(|p| layout[p].after || tokens[p].text == ";" && depth == 0);
        if gap.contains("--") {
            // An inline comment belongs to the preceding line, even when the
            // next lexeme must begin a new structural line.
            let comment_depth = if closing(&token.text).is_some() {
                previous_depth
            } else {
                depth
            };
            comments(
                &mut output,
                gap,
                previous_depth as usize,
                comment_depth as usize,
            );
        } else if wrapped || line_break(gap) {
            if blank_line(gap) {
                output.blank();
            } else {
                output.newline();
            }
        }
        let pad = if i == 0 {
            0
        } else {
            spacing(source, tokens, i).max(layout[i].pad)
        };
        output.write(&token.text, depth as usize, pad);
    }
    comments(
        &mut output,
        original_gap(source, tokens, tokens.len()),
        depth as usize,
        depth as usize,
    );
    output.newline();
    if output.text.ends_with("\n\n") {
        output.text.pop();
    }
    output.text
}

pub(crate) fn format(
    source: &str,
    base: usize,
    applications: &[FormattingApplication],
) -> Result<String, Error> {
    let tokens = lex(source)?;
    let mut widths = vec![0];
    for (i, token) in tokens.iter().enumerate() {
        let preceding = if i == 0 {
            0
        } else {
            spacing(source, &tokens, i)
        };
        widths.push(widths[i] + token.text.len() + preceding);
    }
    let mut layout: Vec<Layout> = (0..tokens.len()).map(|_| Layout::default()).collect();
    let mut delta = vec![0; tokens.len() + 1];
    apply_prefix(
        source,
        &tokens,
        &widths,
        base,
        applications,
        &mut layout,
        &mut delta,
    );
    structure(source, &tokens, &widths, &mut layout, &mut delta);
    let result = render(source, &tokens, &layout, &delta);
    let new = lex(&result);
    if !new.is_ok_and(|new| {
        tokens
            .iter()
            .map(|t| &t.text)
            .eq(new.iter().map(|t| &t.text))
            && tokens.windows(2).zip(new.windows(2)).all(|(old, new)| {
                if (old[0].text == "&" && old[1].text == ".")
                    || old[0].text == "#"
                    || nominal_bracket(&old[0], &old[1])
                {
                    (old[0].end == old[1].start) == (new[0].end == new[1].start)
                } else {
                    true
                }
            })
    }) {
        return Err(Error::new(
            base,
            "formatting would change one or more tokens",
        ));
    }
    Ok(result)
}
