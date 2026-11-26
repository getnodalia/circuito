// SPDX-License-Identifier: MIT OR Apache-2.0
//! What the electrical rules check says, in the interface's language. The engine returns stable
//! codes and English messages that name the parts and nets; the app words each rule itself so the
//! list reads in Spanish too, and keeps the engine's message for the detail.

use circuito_erc::{Rule, Severity, Violation};

use crate::i18n::tr;

/// Every rule, in code order.
pub const ALL: [Rule; 25] = [
    Rule::PinUnconnected,
    Rule::NetSinglePin,
    Rule::NoConnectConnected,
    Rule::OutputConflict,
    Rule::PowerOutputConflict,
    Rule::PinTypeConflict,
    Rule::PowerInputUndriven,
    Rule::NoGround,
    Rule::DuplicateRefdes,
    Rule::MissingValue,
    Rule::InvalidValue,
    Rule::UnknownModel,
    Rule::UnknownSymbol,
    Rule::PartRejected,
    Rule::FloatingInput,
    Rule::VoltageSourceLoop,
    Rule::CurrentSourceCutset,
    Rule::NoDcPath,
    Rule::NetNameConflict,
    Rule::TouchWithoutJunction,
    Rule::FloatingLabel,
    Rule::AmbiguousLabel,
    Rule::FloatingPowerPort,
    Rule::DanglingJunction,
    Rule::DanglingNoConnect,
];

/// The rule's title in English. A new rule in the engine fails to compile here until it has one.
pub fn english(rule: Rule) -> &'static str {
    match rule {
        Rule::PinUnconnected => "Pin connected to nothing",
        Rule::NetSinglePin => "Net with a single pin",
        Rule::NoConnectConnected => "Pin marked no-connect is connected",
        Rule::OutputConflict => "Two outputs drive one net",
        Rule::PowerOutputConflict => "Two power outputs on one net",
        Rule::PinTypeConflict => "Pin types that conflict",
        Rule::PowerInputUndriven => "Power input nothing drives",
        Rule::NoGround => "No ground reference",
        Rule::DuplicateRefdes => "Reference used twice",
        Rule::MissingValue => "Part without a value",
        Rule::InvalidValue => "Invalid value",
        Rule::UnknownModel => "Unknown model",
        Rule::UnknownSymbol => "Symbol not in the library",
        Rule::PartRejected => "Part left out of the netlist",
        Rule::FloatingInput => "Input nothing drives",
        Rule::VoltageSourceLoop => "Voltage sources in a loop",
        Rule::CurrentSourceCutset => "Current source with no return path",
        Rule::NoDcPath => "No DC path to ground",
        Rule::NetNameConflict => "Net with two names",
        Rule::TouchWithoutJunction => "Wire touches without a junction",
        Rule::FloatingLabel => "Label touches nothing",
        Rule::AmbiguousLabel => "Label on several wires",
        Rule::FloatingPowerPort => "Power port touches nothing",
        Rule::DanglingJunction => "Junction joins nothing",
        Rule::DanglingNoConnect => "No-connect flag not on a pin",
    }
}

/// The rule's title in the current language.
pub fn title(rule: Rule) -> &'static str {
    tr(english(rule))
}

/// The severity as a word in the current language.
pub fn severity_word(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => tr("Error"),
        Severity::Warning => tr("Warning"),
        Severity::Info => tr("Note"),
    }
}

/// The things a finding involves, `R1.2, net OUT`.
pub fn items(v: &Violation) -> String {
    v.items.join(", ")
}

/// The part reference an item names: `R1.2` and `R1` both give `R1`; `net OUT` gives nothing.
pub fn part_of(item: &str) -> Option<&str> {
    if item.starts_with("net ") || item.is_empty() {
        return None;
    }
    Some(item.split('.').next().unwrap_or(item)).filter(|r| !r.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_rule_has_a_distinct_title_and_is_listed_once() {
        let mut titles: Vec<&str> = ALL.iter().map(|r| english(*r)).collect();
        titles.sort_unstable();
        titles.dedup();
        assert_eq!(titles.len(), ALL.len());
        let mut codes: Vec<&str> = ALL.iter().map(|r| r.code()).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), ALL.len(), "a rule is listed twice");
    }

    #[test]
    fn items_name_parts() {
        assert_eq!(part_of("R1.2"), Some("R1"));
        assert_eq!(part_of("V1"), Some("V1"));
        assert_eq!(part_of("net OUT"), None);
        assert_eq!(part_of(""), None);
    }
}
