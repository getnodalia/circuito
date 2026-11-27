// SPDX-License-Identifier: MIT OR Apache-2.0
//! The S-expression syntax of KiCad files: lists, bare atoms and quoted strings.
//!
//! Strings use `\"`, `\\`, `\n`, `\r` and `\t` escapes. Comments are not part of the syntax.
//! The reader is iterative on the parse stack, so nesting depth is a limit and not a stack risk.

use crate::error::{Error, Result};

/// Largest input accepted, in bytes.
pub const MAX_BYTES: usize = 256 * 1024 * 1024;
/// Deepest nesting accepted.
pub const MAX_DEPTH: usize = 128;
/// Most nodes accepted in one file.
pub const MAX_NODES: usize = 40_000_000;

/// One S-expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// A bare word or number: `smd`, `0.25`, `F.Cu`.
    Atom(String),
    /// A quoted string.
    Str(String),
    /// A parenthesised list.
    List(Vec<Node>),
}

impl Node {
    /// The text of an atom or string.
    pub fn text(&self) -> Option<&str> {
        match self {
            Node::Atom(s) | Node::Str(s) => Some(s),
            Node::List(_) => None,
        }
    }

    /// The items of a list.
    pub fn items(&self) -> &[Node] {
        match self {
            Node::List(v) => v,
            _ => &[],
        }
    }

    /// The first item's text when this is a list: `segment` in `(segment ...)`.
    pub fn head(&self) -> Option<&str> {
        self.items().first().and_then(Node::text)
    }

    /// The first child list whose head is `name`.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.items()
            .iter()
            .skip(1)
            .find(|n| matches!(n, Node::List(_)) && n.head() == Some(name))
    }

    /// Every child list whose head is `name`.
    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.items()
            .iter()
            .skip(1)
            .filter(move |n| matches!(n, Node::List(_)) && n.head() == Some(name))
    }

    /// The arguments of a list: everything after the head.
    pub fn args(&self) -> &[Node] {
        self.items().get(1..).unwrap_or(&[])
    }

    /// The text of the first argument of the child `name`: `0.25` in `(width 0.25)`.
    pub fn arg_text(&self, name: &str) -> Option<&str> {
        self.child(name)?.args().first()?.text()
    }

    /// The first argument of the child `name` as a number.
    pub fn arg_f64(&self, name: &str) -> Option<f64> {
        self.arg_text(name)?
            .parse()
            .ok()
            .filter(|v: &f64| v.is_finite())
    }
}

/// Parse a whole document: exactly one top-level list.
pub fn parse(text: &str) -> Result<Node> {
    if text.len() > MAX_BYTES {
        return Err(Error::TooLarge {
            what: "file",
            detail: format!("{} bytes exceed the limit of {MAX_BYTES}", text.len()),
        });
    }
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut stack: Vec<Vec<Node>> = Vec::new();
    let mut root: Option<Node> = None;
    let mut nodes = 0usize;
    let syntax = |line: usize, detail: &str| Error::Syntax {
        line,
        detail: detail.to_string(),
    };
    while let Some(&c) = bytes.get(i) {
        match c {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b' ' | b'\t' | b'\r' => i += 1,
            b'(' => {
                if root.is_some() {
                    return Err(syntax(line, "text after the end of the document"));
                }
                if stack.len() >= MAX_DEPTH {
                    return Err(Error::TooLarge {
                        what: "file",
                        detail: format!("line {line}: nesting deeper than {MAX_DEPTH}"),
                    });
                }
                stack.push(Vec::new());
                i += 1;
            }
            b')' => {
                let list = stack
                    .pop()
                    .ok_or_else(|| syntax(line, "`)` without a matching `(`"))?;
                i += 1;
                let node = Node::List(list);
                match stack.last_mut() {
                    Some(parent) => parent.push(node),
                    None => root = Some(node),
                }
            }
            b'"' => {
                let parent = stack
                    .last_mut()
                    .ok_or_else(|| syntax(line, "a string outside the top-level list"))?;
                i += 1;
                let mut out: Vec<u8> = Vec::new();
                loop {
                    let b = *bytes
                        .get(i)
                        .ok_or_else(|| syntax(line, "a string is not closed"))?;
                    i += 1;
                    match b {
                        b'"' => break,
                        b'\\' => {
                            let e = *bytes
                                .get(i)
                                .ok_or_else(|| syntax(line, "a string is not closed"))?;
                            i += 1;
                            out.push(match e {
                                b'n' => b'\n',
                                b'r' => b'\r',
                                b't' => b'\t',
                                other => other,
                            });
                        }
                        b'\n' => {
                            line += 1;
                            out.push(b);
                        }
                        _ => out.push(b),
                    }
                }
                nodes += 1;
                parent.push(Node::Str(String::from_utf8_lossy(&out).into_owned()));
            }
            _ => {
                let start = i;
                while let Some(&b) = bytes.get(i) {
                    if matches!(b, b' ' | b'\t' | b'\r' | b'\n' | b'(' | b')' | b'"') {
                        break;
                    }
                    i += 1;
                }
                let parent = stack
                    .last_mut()
                    .ok_or_else(|| syntax(line, "text outside the top-level list"))?;
                let word = bytes.get(start..i).unwrap_or(&[]);
                nodes += 1;
                parent.push(Node::Atom(String::from_utf8_lossy(word).into_owned()));
            }
        }
        if nodes > MAX_NODES {
            return Err(Error::TooLarge {
                what: "file",
                detail: format!("more than {MAX_NODES} items"),
            });
        }
    }
    if !stack.is_empty() {
        return Err(syntax(line, "a `(` is never closed"));
    }
    root.ok_or_else(|| syntax(line, "the document is empty"))
}

/// Write a string the way KiCad does: quoted, with `"` and `\` escaped.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lists_atoms_and_strings() {
        let n = parse("(a (b 1 \"x y\") (c))").unwrap();
        assert_eq!(n.head(), Some("a"));
        assert_eq!(n.child("b").unwrap().args()[1].text(), Some("x y"));
        assert!(n.child("c").is_some() && n.child("d").is_none());
    }

    #[test]
    fn string_escapes_round_trip() {
        let n = parse(r#"(a "q\"uote\\ and\nnewline")"#).unwrap();
        let s = n.args()[0].text().unwrap().to_string();
        assert_eq!(s, "q\"uote\\ and\nnewline");
        assert_eq!(quote(&s), r#""q\"uote\\ and\nnewline""#);
    }

    #[test]
    fn hostile_text_is_an_error() {
        for bad in [
            "", "(", ")", "(a", "a", "(a) b", "(a) (b)", "(a \"x", "\"x\"", "(a))",
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        let deep = "(".repeat(MAX_DEPTH + 1) + &")".repeat(MAX_DEPTH + 1);
        assert!(matches!(parse(&deep), Err(Error::TooLarge { .. })));
        let ok = "(".repeat(MAX_DEPTH) + &")".repeat(MAX_DEPTH);
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn numbers_come_back_finite_or_not_at_all() {
        let n = parse("(a (w 0.25) (x nan) (y 1e999) (z abc))").unwrap();
        assert_eq!(n.arg_f64("w"), Some(0.25));
        assert_eq!(n.arg_f64("x"), None);
        assert_eq!(n.arg_f64("y"), None);
        assert_eq!(n.arg_f64("z"), None);
    }
}
