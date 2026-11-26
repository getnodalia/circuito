// SPDX-License-Identifier: MIT OR Apache-2.0
//! The code pane's state and what it needs that is not drawing: the text, what is wrong with it
//! in the interface's language, and the colouring of its words.

use circuito_script::{BuildError, Diagnostic, Problem};

use crate::i18n::{tr, trf};

/// The text form of the open circuit and where it stands.
#[derive(Clone, Debug, Default)]
pub struct Script {
    /// What the pane shows and the person edits.
    pub text: String,
    /// What is wrong with the text. While there is anything here the drawing keeps the last
    /// circuit that was understood.
    pub problems: Vec<Diagnostic>,
    /// The text reads well but could not be drawn.
    pub error: Option<BuildError>,
    /// The drawing changed since the text was written: write the text again.
    pub regenerate: bool,
    /// When (in seconds of the window's clock) the text was last typed in, until it is read.
    pub typed_at: Option<f64>,
}

impl Script {
    /// A pane that will write the text for the drawing on first use.
    pub fn new() -> Script {
        Script {
            regenerate: true,
            ..Script::default()
        }
    }

    /// Whether the text is right as it stands.
    pub fn is_clean(&self) -> bool {
        self.problems.is_empty() && self.error.is_none()
    }
}

/// What is wrong, in the interface's language.
pub fn message(problem: &Problem) -> String {
    match problem {
        Problem::Unbalanced => tr("A parenthesis or quote is not closed").to_owned(),
        Problem::BadRefdes { text } => trf(
            "`{}` is not a part name: start with a letter, then letters, digits or _",
            &[text],
        ),
        Problem::UnknownKind { refdes } => trf(
            "Cannot tell what kind of part `{}` is; write it as `{}:KIND`, for example `{}:R`",
            &[refdes, refdes, refdes],
        ),
        Problem::UnknownPart { name } => trf("There is no part kind `{}`", &[name]),
        Problem::DuplicateRefdes { refdes } => {
            trf("There is already a part called `{}`", &[refdes])
        }
        Problem::NetCount {
            refdes,
            expected,
            found,
            pins,
        } => trf(
            "{} needs {} nets (pins {}) after an optional value, but the line has {}",
            &[refdes, &expected.to_string(), pins, &found.to_string()],
        ),
        Problem::UnknownPin { refdes, key, pins } => {
            trf("{} has no pin or field `{}` (pins: {})", &[refdes, key, pins])
        }
        Problem::PinTwice { refdes, key } => {
            trf("Pin `{}` of {} is given twice", &[key, refdes])
        }
        Problem::NoValue { refdes } => trf("{} has no value", &[refdes]),
        Problem::BadValue { refdes, text, why } => {
            trf("{}: `{}` is not a valid value ({})", &[refdes, text, why])
        }
        Problem::BadLayout { text } => trf(
            "`{}` is not a position: write `@x,y` with whole numbers, then optionally r90, r180, r270 or mirror",
            &[text],
        ),
        Problem::ModelUsage => tr("Write a model as `model NAME KIND param=value ...`").to_owned(),
        Problem::UnknownModelKind { kind } => trf(
            "There is no model kind `{}` (D, NPN, PNP, NMOS, PMOS, SW, OPAMP)",
            &[kind],
        ),
        Problem::BadNumber { text } => trf("`{}` is not a number", &[text]),
        Problem::AnalysisUsage { usage } => trf("Write it as `{}`", &[usage]),
        Problem::BadAnalysis { why } => why.clone(),
        Problem::TooMuch => tr("The text is too long").to_owned(),
    }
}

/// Why a text that reads well could not be drawn, in the interface's language.
pub fn build_message(e: &BuildError) -> String {
    match e {
        BuildError::TooBig => tr("The circuit has too many parts").to_owned(),
        BuildError::UnknownPart(id) => trf("There is no part kind `{}`", &[id]),
        BuildError::BadPins(r) => trf("{} does not give one net per pin", &[r]),
        BuildError::Layout => tr("The circuit could not be drawn").to_owned(),
    }
}

/// One problem as a line of the list under the code: where, then what.
pub fn line_of(d: &Diagnostic) -> String {
    trf("Line {}: {}", &[&d.line.to_string(), &message(&d.problem)])
}

/// What a word of the text is, for colouring it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Comment,
    /// `title`, `model` and the `.op`, `.dc`, `.ac` and `.tran` lines.
    Keyword,
    /// The part at the start of a line.
    Part,
    /// `pin=` and `field=`.
    Key,
    /// A value, a number or a position.
    Number,
    /// A net.
    Net,
    Plain,
}

/// A coloured stretch of the text, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub class: Class,
}

fn looks_numeric(word: &str) -> bool {
    let w = word.trim_start_matches(['+', '-']);
    w.chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == '.')
        || word.contains('(') && word.chars().next().is_some_and(char::is_alphabetic)
        || word.starts_with('@')
}

/// The text cut into coloured stretches that cover all of it, in order.
pub fn highlight(text: &str) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let mut push = |start: usize, end: usize, class: Class| {
        if end > start {
            spans.push(Span { start, end, class });
        }
    };
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let body_end = line_start + line.trim_end_matches(['\n', '\r']).len();
        let comment_at = comment_start(line).map(|i| line_start + i);
        let code_end = comment_at.unwrap_or(body_end);
        let mut pos = line_start;
        let mut first = true;
        let mut keyword_line = false;
        while pos < code_end {
            // Whitespace.
            let ws = text[pos..code_end]
                .find(|c: char| !c.is_whitespace())
                .map_or(code_end, |i| pos + i);
            push(pos, ws, Class::Plain);
            pos = ws;
            if pos >= code_end {
                break;
            }
            // One word: to the next blank outside parentheses and quotes.
            let mut end = pos;
            let mut depth = 0usize;
            let mut quoted = false;
            for (i, c) in text[pos..code_end].char_indices() {
                end = pos + i + c.len_utf8();
                match c {
                    '"' => quoted = !quoted,
                    '(' if !quoted => depth += 1,
                    ')' if !quoted => depth = depth.saturating_sub(1),
                    c if c.is_whitespace() && depth == 0 && !quoted => {
                        end = pos + i;
                        break;
                    }
                    _ => {}
                }
            }
            let word = text.get(pos..end).unwrap_or("");
            let class = if first {
                let lower = word.to_ascii_lowercase();
                if matches!(lower.as_str(), "title" | "model")
                    || (lower.starts_with('.') && lower.len() > 1)
                {
                    keyword_line = true;
                    Class::Keyword
                } else {
                    Class::Part
                }
            } else if keyword_line {
                if looks_numeric(word) {
                    Class::Number
                } else {
                    Class::Plain
                }
            } else if let Some(eq) = word.find('=').filter(|&eq| eq > 0) {
                push(pos, pos + eq + 1, Class::Key);
                let rest = &word[eq + 1..];
                push(
                    pos + eq + 1,
                    end,
                    if looks_numeric(rest) {
                        Class::Number
                    } else {
                        Class::Net
                    },
                );
                pos = end;
                first = false;
                continue;
            } else if looks_numeric(word) {
                Class::Number
            } else {
                Class::Net
            };
            push(pos, end, class);
            pos = end;
            first = false;
        }
        push(code_end.max(pos), body_end, Class::Comment);
        push(body_end, line_start + line.len(), Class::Plain);
        line_start += line.len();
    }
    spans
}

/// Where the comment of a line begins (a `#` or `//` outside quotes), as a byte offset.
fn comment_start(line: &str) -> Option<usize> {
    let mut quoted = false;
    let mut prev_slash = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return Some(i),
            '/' if !quoted => {
                if let Some(p) = prev_slash {
                    return Some(p);
                }
                prev_slash = Some(i);
                continue;
            }
            _ => {}
        }
        prev_slash = None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(text: &str) -> Vec<(&str, Class)> {
        highlight(text)
            .into_iter()
            .filter(|s| s.class != Class::Plain)
            .map(|s| (&text[s.start..s.end], s.class))
            .collect()
    }

    #[test]
    fn the_spans_cover_the_text_without_gaps() {
        let t = "title RC filter\nR1 1k in out // load\nC1 100n out gnd\n.ac dec 20 10 100k\n\n";
        let spans = highlight(t);
        let mut at = 0;
        for s in &spans {
            assert_eq!(s.start, at, "gap before {s:?}");
            assert!(t.is_char_boundary(s.start) && t.is_char_boundary(s.end));
            at = s.end;
        }
        assert_eq!(at, t.len());
    }

    #[test]
    fn words_get_their_class() {
        let got = classes("R1 1k in out // load\n");
        assert_eq!(
            got,
            vec![
                ("R1", Class::Part),
                ("1k", Class::Number),
                ("in", Class::Net),
                ("out", Class::Net),
                ("// load", Class::Comment),
            ]
        );
        let got = classes("Q1:NPN c=out b=in e=gnd\n.op\n");
        assert!(got.contains(&("c=", Class::Key)) && got.contains(&("out", Class::Net)));
        assert!(got.contains(&(".op", Class::Keyword)));
    }

    #[test]
    fn a_hash_inside_quotes_is_not_a_comment() {
        let got = classes("R1 \"a#b\" x y\n");
        assert!(!got.iter().any(|(_, c)| *c == Class::Comment), "{got:?}");
    }

    #[test]
    fn odd_text_is_coloured_without_panicking() {
        for t in ["", "\n", "é ü ñ (((", "\"", "R1 \"unclosed", "///", "#"] {
            let _ = highlight(t);
        }
    }

    #[test]
    fn every_problem_has_a_message() {
        let parsed = circuito_script::parse("R1 1k a\nX\nQ9:NOPE a b c\nR1 1k a b\n.dc\n");
        assert!(!parsed.problems.is_empty());
        for d in &parsed.problems {
            assert!(!message(&d.problem).is_empty());
            assert!(line_of(d).starts_with(&trf("Line {}: {}", &[&d.line.to_string(), ""])));
        }
    }
}
