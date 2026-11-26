// SPDX-License-Identifier: MIT OR Apache-2.0
//! Parameter expressions: `{2*pi*f0}`, `{sqrt(a^2 + b^2)}`, `{r1 || r2}` is not supported.
//!
//! Grammar: `+ - * / ^ **` (power is right-associative and binds tighter than unary minus on its
//! left), parentheses, numbers with SI suffixes, parameter names (case-insensitive), the constant
//! `pi`, and the functions `sqrt abs exp ln log log10 sin cos tan asin acos atan atan2 sinh cosh
//! tanh min max pow floor ceil`. Nesting depth, length and result are bounded.

use circuito_core::value::parse_value;
use std::collections::BTreeMap;

const MAX_LEN: usize = 4096;
const MAX_DEPTH: usize = 64;

/// Evaluate `text` with the given parameters (keys upper case).
///
/// # Errors
/// A message naming what is wrong: unknown name, bad number, arity, non-finite result.
pub fn evaluate(text: &str, params: &BTreeMap<String, f64>) -> Result<f64, String> {
    if text.len() > MAX_LEN {
        return Err(format!("expression is longer than {MAX_LEN} characters"));
    }
    let mut p = Parser {
        s: text.as_bytes(),
        pos: 0,
        params,
        depth: 0,
        src: text,
    };
    let v = p.expr()?;
    p.ws();
    if p.pos < p.s.len() {
        return Err(format!(
            "unexpected `{}` in expression `{}`",
            p.rest(),
            text.trim()
        ));
    }
    if !v.is_finite() {
        return Err(format!(
            "expression `{}` is not a finite number (division by zero or overflow?)",
            text.trim()
        ));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
    params: &'a BTreeMap<String, f64>,
    depth: usize,
    src: &'a str,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.s.get(self.pos).is_some_and(u8::is_ascii_whitespace) {
            self.pos += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }
    fn rest(&self) -> String {
        self.src
            .get(self.pos..)
            .unwrap_or("")
            .chars()
            .take(12)
            .collect()
    }
    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            Err("expression is nested too deeply".to_string())
        } else {
            Ok(())
        }
    }

    fn expr(&mut self) -> Result<f64, String> {
        self.enter()?;
        let mut v = self.term()?;
        loop {
            self.ws();
            match self.peek() {
                Some(b'+') => {
                    self.pos += 1;
                    v += self.term()?;
                }
                Some(b'-') => {
                    self.pos += 1;
                    v -= self.term()?;
                }
                _ => break,
            }
        }
        self.depth -= 1;
        Ok(v)
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut v = self.unary()?;
        loop {
            self.ws();
            match self.peek() {
                Some(b'*') if self.s.get(self.pos + 1) != Some(&b'*') => {
                    self.pos += 1;
                    v *= self.unary()?;
                }
                Some(b'/') => {
                    self.pos += 1;
                    let d = self.unary()?;
                    if d == 0.0 {
                        return Err("division by zero in expression".to_string());
                    }
                    v /= d;
                }
                _ => break,
            }
        }
        Ok(v)
    }

    fn unary(&mut self) -> Result<f64, String> {
        self.ws();
        match self.peek() {
            Some(b'-') => {
                self.pos += 1;
                self.enter()?;
                let v = self.unary()?;
                self.depth -= 1;
                Ok(-v)
            }
            Some(b'+') => {
                self.pos += 1;
                self.enter()?;
                let v = self.unary()?;
                self.depth -= 1;
                Ok(v)
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<f64, String> {
        let base = self.atom()?;
        self.ws();
        let is_pow = match (self.peek(), self.s.get(self.pos + 1)) {
            (Some(b'^'), _) => {
                self.pos += 1;
                true
            }
            (Some(b'*'), Some(b'*')) => {
                self.pos += 2;
                true
            }
            _ => false,
        };
        if is_pow {
            self.enter()?;
            let e = self.unary()?;
            self.depth -= 1;
            return Ok(base.powf(e));
        }
        Ok(base)
    }

    fn atom(&mut self) -> Result<f64, String> {
        self.ws();
        match self.peek() {
            Some(b'(') => {
                self.pos += 1;
                let v = self.expr()?;
                self.ws();
                if self.peek() != Some(b')') {
                    return Err("missing `)` in expression".to_string());
                }
                self.pos += 1;
                Ok(v)
            }
            Some(c) if c.is_ascii_digit() || c == b'.' => self.number(),
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => self.name(),
            Some(_) => Err(format!(
                "unexpected `{}` in expression `{}`",
                self.rest(),
                self.src.trim()
            )),
            None => Err(format!("expression `{}` ends too early", self.src.trim())),
        }
    }

    fn number(&mut self) -> Result<f64, String> {
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == b'.') {
            self.pos += 1;
        }
        // Exponent only when followed by digits (so `2e` is a suffix error, `1e-3` is a number).
        if matches!(self.peek(), Some(b'e' | b'E')) {
            let mut j = self.pos + 1;
            if matches!(self.s.get(j), Some(b'+' | b'-')) {
                j += 1;
            }
            if self.s.get(j).is_some_and(u8::is_ascii_digit) {
                while self.s.get(j).is_some_and(u8::is_ascii_digit) {
                    j += 1;
                }
                self.pos = j;
            }
        }
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.pos += 1;
        }
        let tok = self.src.get(start..self.pos).unwrap_or("");
        parse_value(tok).map_err(|e| e.to_string())
    }

    fn name(&mut self) -> Result<f64, String> {
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_')
        {
            self.pos += 1;
        }
        let name = self.src.get(start..self.pos).unwrap_or("").to_uppercase();
        self.ws();
        if self.peek() == Some(b'(') {
            self.pos += 1;
            let mut args = Vec::new();
            self.ws();
            if self.peek() == Some(b')') {
                self.pos += 1;
            } else {
                loop {
                    args.push(self.expr()?);
                    self.ws();
                    match self.peek() {
                        Some(b',') => self.pos += 1,
                        Some(b')') => {
                            self.pos += 1;
                            break;
                        }
                        _ => {
                            return Err(format!(
                                "missing `)` after the arguments of `{}`",
                                name.to_lowercase()
                            ))
                        }
                    }
                    if args.len() > 16 {
                        return Err("too many function arguments".into());
                    }
                }
            }
            return call(&name, &args);
        }
        if let Some(v) = self.params.get(&name) {
            return Ok(*v);
        }
        if name == "PI" {
            return Ok(std::f64::consts::PI);
        }
        Err(format!(
            "unknown parameter `{}` (define it with .param before use)",
            name.to_lowercase()
        ))
    }
}

fn call(name: &str, a: &[f64]) -> Result<f64, String> {
    let one = |f: fn(f64) -> f64| -> Result<f64, String> {
        match a {
            [x] => Ok(f(*x)),
            _ => Err(format!(
                "{} takes 1 argument, got {}",
                name.to_lowercase(),
                a.len()
            )),
        }
    };
    let two = |f: fn(f64, f64) -> f64| -> Result<f64, String> {
        match a {
            [x, y] => Ok(f(*x, *y)),
            _ => Err(format!(
                "{} takes 2 arguments, got {}",
                name.to_lowercase(),
                a.len()
            )),
        }
    };
    match name {
        "SQRT" => {
            if a.first().is_some_and(|x| *x < 0.0) {
                return Err("sqrt of a negative number".into());
            }
            one(f64::sqrt)
        }
        "ABS" => one(f64::abs),
        "EXP" => one(f64::exp),
        "LN" | "LOG" => {
            if a.first().is_some_and(|x| *x <= 0.0) {
                return Err(format!("{} of a non-positive number", name.to_lowercase()));
            }
            one(f64::ln)
        }
        "LOG10" => {
            if a.first().is_some_and(|x| *x <= 0.0) {
                return Err("log10 of a non-positive number".into());
            }
            one(f64::log10)
        }
        "SIN" => one(f64::sin),
        "COS" => one(f64::cos),
        "TAN" => one(f64::tan),
        "ASIN" => one(f64::asin),
        "ACOS" => one(f64::acos),
        "ATAN" => one(f64::atan),
        "SINH" => one(f64::sinh),
        "COSH" => one(f64::cosh),
        "TANH" => one(f64::tanh),
        "FLOOR" => one(f64::floor),
        "CEIL" => one(f64::ceil),
        "ATAN2" => two(f64::atan2),
        "POW" => two(f64::powf),
        "MIN" => two(f64::min),
        "MAX" => two(f64::max),
        _ => Err(format!("unknown function `{}`", name.to_lowercase())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> Result<f64, String> {
        let mut p = BTreeMap::new();
        p.insert("R1".to_string(), 1000.0);
        p.insert("C".to_string(), 1e-7);
        evaluate(s, &p)
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(ev("1+2*3").unwrap(), 7.0);
        assert_eq!(ev("(1+2)*3").unwrap(), 9.0);
        assert_eq!(ev("2^3^2").unwrap(), 512.0);
        assert_eq!(ev("2**3").unwrap(), 8.0);
        assert_eq!(ev("-2^2").unwrap(), -4.0);
        assert_eq!(ev("10/4").unwrap(), 2.5);
        assert_eq!(ev("4.7k*2").unwrap(), 9400.0);
        assert_eq!(ev("1e-3+1m").unwrap(), 2e-3);
    }

    #[test]
    fn parameters_and_functions() {
        assert_eq!(ev("r1*2").unwrap(), 2000.0);
        assert!((ev("1/(2*pi*r1*c)").unwrap() - 1_591.549_430_918_953_4).abs() < 1e-9);
        assert_eq!(ev("sqrt(16)+max(1,2)").unwrap(), 6.0);
        assert_eq!(ev("min(3, 2)").unwrap(), 2.0);
        assert!((ev("log10(1000)").unwrap() - 3.0).abs() < 1e-12);
    }

    #[test]
    fn errors_are_messages() {
        for bad in [
            "",
            "1+",
            "(1",
            "1)",
            "foo",
            "foo(1)",
            "sqrt(-1)",
            "1/0",
            "ln(0)",
            "sin()",
            "1 2",
            "2$",
            "max(1)",
            "1e999*10",
            "unknown+1",
        ] {
            let e = ev(bad);
            assert!(e.is_err(), "{bad:?} -> {e:?}");
            assert!(!e.unwrap_err().is_empty());
        }
        assert!(ev(&"(".repeat(1000)).is_err());
        assert!(ev(&"-".repeat(1000)).is_err());
        assert!(ev(&format!("1{}", "+1".repeat(5000))).is_err());
    }
}
