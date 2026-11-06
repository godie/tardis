//! Layer b of the respond-mode safety: validate a generated reply before
//! synthesis.
//!
//! Layer a ([`crate::processor::decision`]) classifies the request; this
//! layer trusts nothing about the model. Even when layer a says Answer,
//! the LLM can volunteer a promise, so the reply is scanned for a date,
//! an acceptance word, or an amount before it is allowed anywhere near
//! the speakers. A blocked reply is still emitted as an `Output` event —
//! silent in the room, visible on screen — and the runner skips
//! synthesis.
//!
//! Lexical and erring towards blocking: a reply that merely *quotes* the
//! amount asked about is blocked too. No exceptions in v1.

use crate::processor::processor::BlockReason;

/// The scanned result for one generated reply.
#[derive(Debug, Clone, PartialEq)]
pub struct Guarded {
    /// `true` only when no block reason applied.
    pub allowed: bool,
    /// Every reason the reply is not allowed to be spoken.
    pub reasons: Vec<BlockReason>,
}

/// Classify one generated reply: blocked on a date, an acceptance, an
/// amount, or nothing at all to say.
pub fn validate_reply(reply: &str) -> Guarded {
    let mut reasons = Vec::new();
    if reply.trim().is_empty() {
        reasons.push(BlockReason::Empty);
        return Guarded {
            allowed: false,
            reasons,
        };
    }

    let lowered = reply.trim().replace('¿', "").to_lowercase();
    let trimmed = lowered.trim();

    const DATE_WORDS: [&str; 11] = [
        "mañana",
        "manana",
        "tomorrow",
        "next week",
        "la semana que viene",
        "el lunes",
        "el martes",
        "el miércoles",
        "el jueves",
        "el viernes",
        "el domingo",
    ];
    if DATE_WORDS.iter().any(|word| trimmed.contains(word)) {
        reasons.push(BlockReason::Date);
    }

    const ACCEPTANCE_WORDS: [&str; 9] = [
        "i'll",
        "i will",
        "lo haré",
        "lo hare",
        "lo haría",
        "claro",
        "of course",
        "deal",
        "trato",
    ];
    if ACCEPTANCE_WORDS.iter().any(|word| trimmed.contains(word)) {
        reasons.push(BlockReason::Acceptance);
    }

    if trimmed.contains('€') || trimmed.contains('$') || trimmed.contains('%') {
        reasons.push(BlockReason::Money);
    }
    const MONEY_WORDS: [&str; 10] = [
        "cost", "price", "pagar", "cuánto", "cuanto", "how much", "dolar", "dólar", "euro", "peso",
    ];
    if MONEY_WORDS.iter().any(|word| trimmed.contains(word)) {
        reasons.push(BlockReason::Money);
    }

    Guarded {
        allowed: reasons.is_empty(),
        reasons,
    }
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn reasons_of(reply: &str) -> Vec<BlockReason> {
        validate_reply(reply).reasons
    }

    #[test]
    fn a_plain_answer_is_allowed() {
        let guarded = validate_reply("Se está cerrando a las cinco, te despido en la puerta.");
        assert!(guarded.allowed, "reasons: {:?}", guarded.reasons);
        assert!(guarded.reasons.is_empty());
    }

    /// Dates, weekdays, and booking windows block — in either language
    /// the app carries.
    #[test]
    fn a_reply_with_a_date_or_window_is_blocked() {
        for reply in [
            "Claro, mañana lo reviso",
            "I'll have it ready tomorrow",
            ", wait for me next week",
            "The office opens el lunes",
        ] {
            assert!(
                reasons_of(reply).contains(&BlockReason::Date),
                "{reply:?} must be blocked as a date"
            );
        }
    }

    /// Acceptance words live in layer b, not layer a: here they block.
    #[test]
    fn a_reply_with_an_acceptance_word_is_blocked() {
        for reply in [
            "I'll take care of it",
            "Lo haré en cuanto llegue",
            ", it is a deal",
            "Sí, trato hecho",
        ] {
            assert!(
                reasons_of(reply).contains(&BlockReason::Acceptance),
                "{reply:?} must be blocked as an acceptance"
            );
        }
    }

    /// An amount is blocked by glyph or by an amount word.
    #[test]
    fn a_reply_with_an_amount_is_blocked() {
        for reply in [
            "Would cost 1.500 pesos would be the ticket",
            "The ticket costs 50 $",
            "It would be the 10% more",
            "The total is 30 €",
        ] {
            assert!(
                reasons_of(reply).contains(&BlockReason::Money),
                "{reply:?} must be blocked as an amount"
            );
        }
    }

    /// Even written without the answer's own words, quoting the amount
    /// the host asked about blocks. No exceptions in v1.
    #[test]
    fn a_reply_quoting_the_amount_from_the_question_is_blocked_too() {
        assert!(
            reasons_of("She asked how much the registration costs the pass")
                .contains(&BlockReason::Money),
        );
    }

    #[test]
    fn an_empty_or_whitespace_reply_is_blocked_as_empty() {
        assert_eq!(reasons_of(""), vec![BlockReason::Empty]);
        assert_eq!(reasons_of("   \n\t"), vec![BlockReason::Empty]);
    }
}
