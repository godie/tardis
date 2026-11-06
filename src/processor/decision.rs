//! Layer a of the respond-mode safety: classify the incoming utterance
//! from the host, then decide whether to act.
//!
//! Pure and table-tested. One rule per decision, all lexical:
//!
//! - a host name and a question → [`Decision::Answer`];
//! - a host name and a *commitment request* → [`Decision::Suggest`] —
//!   never `Answer`, whatever the phrasing looks like. Commitments are
//!   date, money, or "can you … by …" shape (dates, money, "can you …
//!   by …");
//! - everything else → [`Decision::Ignore`] (no host, no question,
//!   somebody else's name).
//!
//! Acceptance words (`claro`, `I'll`, …) are NOT classified here: in the
//! *incoming* question they are just speech. They matter when they show
//! up in the **generated reply**, and that is layer b's job
//! ([`crate::processor::guard`]) — which is what keeps a plain
//! "¿claro que sí?" from being demoted to a suggestion for no reason.

use crate::processor::processor::RespondPolicy;

/// What layer a decided to do with an utterance.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Answer the host: generate a reply, pass it through layer b, and
    /// speak it if the guard allows.
    Answer,
    /// Surface a suggestion in the UI, never synthesized: a commitment
    /// request and the host's own words determine what is suggested. It
    /// is carried to the UI as a blocked output; the runner does not
    /// speak it.
    Suggest {
        /// Human-readable why: "date or booking", "money or amount", …
        reason: String,
    },
    /// Do not act at all: no reply generated, no output event beyond
    /// the transcript.
    Ignore,
}

/// Does the utterance look like a question?
///
/// A trailing `?` (after trimming) counts, and so does a **leading**
/// `¿`, because the Spanish source script opens questions with it and
/// speech recognisers frequently drop the closing punctuation.
pub fn is_question(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.ends_with('?') || trimmed.starts_with('¿') {
        return true;
    }
    let lowered = trimmed.to_lowercase();
    // Interrogative openers cover the recogniser case too: the closing
    // `?` is not the only signal the words already carry.
    const OPENERS: [&str; 12] = [
        "what ", "when ", "where ", "how ", "who ", "why ", "qué ", "que ", "cómo ", "como ",
        "cuándo ", "cuando ",
    ];
    OPENERS.iter().any(|opener| lowered.starts_with(opener))
}

/// Is the utterance asking for a commitment?
///
/// Lexical, deliberately single-case: a phrase only has to *look* like
/// a commitment to be demoted to a suggestion — layer a errs towards
/// not speaking. The categories the core plan names:
///
/// - dates/booking: `mañana`, `el lunes`, weekday, `next week`, `tomorrow`.
/// - money/amounts: currency glyphs, currency words, `cost`, `price`.
/// - "can you … by …" patterns — both languages the app carries.
pub fn is_commitment_request(text: &str) -> bool {
    let lowered = text.trim().replace('¿', "").to_lowercase();
    let trimmed = lowered.trim();

    // Dates and booking windows.
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
        return true;
    }

    // Money and amounts: a currency glyph anywhere, or an amount word.
    if trimmed.contains('€') || trimmed.contains('$') || trimmed.contains('%') {
        return true;
    }
    const MONEY_WORDS: [&str; 12] = [
        "cost", "price", "pagar", "cuánto", "cuanto", "how much", "dolar", "dólar", "euro", "peso",
        "monto", "amount",
    ];
    if MONEY_WORDS.iter().any(|word| trimmed.contains(word)) {
        return true;
    }

    // "can you … by …" / "puedes … para …" in both shipped languages.
    if (trimmed.contains("can you") && trimmed.contains(" by "))
        || (trimmed.contains("puedes") && trimmed.contains(" para "))
    {
        return true;
    }
    false
}

/// Does the host's name appear as a word in the utterance?
///
/// Case-insensitive with an optional word boundary: `"Ana"` matches in
/// "ana, ¿…?"", not inside "banana". Trailing punctuation does not stop
/// the match.
pub fn host_name_mentioned(text: &str, host_names: &[String]) -> bool {
    let lowered = text.to_lowercase();
    host_names.iter().any(|name| {
        let name = name.trim().to_lowercase();
        if name.is_empty() {
            return false;
        }
        lowered.match_indices(&name).any(|(pos, _)| {
            let before_ok = pos == 0
                || !lowered[..pos]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_alphanumeric);
            let after = pos + name.len();
            let after_ok = after >= lowered.len()
                || !lowered[after..]
                    .chars()
                    .next()
                    .is_some_and(char::is_alphanumeric);
            before_ok && after_ok
        })
    })
}

/// Classify one uttered utterance against the respond policy.
///
/// Order is fixed and fail-fast: empty → Ignore; name required → Ignore;
/// commitment → Suggest; question → Answer; else Ignore.
pub fn decide_should_respond(text: &str, policy: &RespondPolicy) -> Decision {
    if text.trim().is_empty() {
        return Decision::Ignore;
    }
    if policy.require_name && !host_name_mentioned(text, &policy.host_names) {
        return Decision::Ignore;
    }
    if is_commitment_request(text) {
        // `allow_commitments` is rejected by config validation in v1;
        // acting on it here would let a hand-edited settings file speak
        // on stage. Treat everything commitment-shaped as a suggestion,
        // whatever the flag says.
        return Decision::Suggest {
            reason: "commitment request: needs the host's confirmation before acting".to_string(),
        };
    }
    if is_question(text) {
        return Decision::Answer;
    }
    Decision::Ignore
}

// ===== Unit tests ========================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processor::processor::RespondPolicy;

    /// The policy v1 ships: name and question required.
    fn responding_policy() -> RespondPolicy {
        RespondPolicy {
            host_names: vec!["Ana".to_string()],
            answer_questions_only: true,
            require_name: true,
            allow_commitments: false,
        }
    }

    /// The plan's decision table: name + question = Answer, name only /
    /// question only / another name / empty = Ignore.
    #[test]
    fn name_plus_question_answers_and_everything_else_ignores() {
        let policy = responding_policy();
        let cases = [
            ("Ana, what time is it?", Decision::Answer),
            ("ana how is the weather?", Decision::Answer),
            ("¿ana, cómo estás?", Decision::Answer),
            ("Ana", Decision::Ignore),
            ("¿Qué hora es?", Decision::Ignore),
            ("Hola, ¿cómo estás?", Decision::Ignore),
            ("   ", Decision::Ignore),
            ("Banana anaconda Split", Decision::Ignore),
        ];
        for (text, expected) in cases {
            assert_eq!(
                decide_should_respond(text, &policy),
                expected,
                "decision for {text:?} must match the plan's table"
            );
        }
    }

    /// The safety rule: a commitment request is Suggest, never Answer,
    /// even named and properly interrogative.
    #[test]
    fn a_commitment_request_with_name_and_question_is_a_suggestion() {
        let policy = responding_policy();
        let cases = [
            "Ana, can you confirm the price by Friday?",
            "Ana, mañana te lo pago",
            "ana, ¿cuánto cuesta el pasaje?",
            "Ana, next week hay que juntar 500 €",
        ];
        for text in cases {
            match decide_should_respond(text, &policy) {
                Decision::Suggest { .. } => {}
                other => panic!("{text:?} must be Suggest, got {other:?}"),
            }
        }
    }

    /// A leading `¿` alone counts as a question, for recognisers that
    /// drop the closing punctuation.
    #[test]
    fn a_leading_spanish_question_mark_marks_a_question() {
        assert!(is_question("¿qué hora es, ana?"));
        assert!(is_question("¿ana, cómo estás"));
        assert!(!is_question("ana, digame como llego a casa"));
    }

    /// The classifier reads only the host language it ships with:
    /// anything else is silence, not a guess.
    #[test]
    fn a_transcript_in_another_script_is_ignored() {
        let policy = responding_policy();
        assert_eq!(decide_should_respond("你好吗？", &policy), Decision::Ignore);
        assert_eq!(decide_should_respond("🙂🙂🙂", &policy), Decision::Ignore);
    }

    /// The demotion only happens on the *outgoing* reply, so an
    /// acceptance word in the question must not turn a plain question
    /// into a suggestion.
    #[test]
    fn an_acceptance_word_in_the_question_does_not_block_the_decision() {
        let policy = responding_policy();
        assert_eq!(
            decide_should_respond("Ana, digo, ¿claro que sí?", &policy),
            Decision::Answer
        );
    }

    /// Word-boundary: the host's name inside a longer word does not
    /// count as a mention.
    #[test]
    fn the_host_name_must_match_on_a_word_boundary() {
        assert!(host_name_mentioned("ana, ¿cómo va?", &["ana".to_string()]));
        assert!(!host_name_mentioned(
            "banana, ¿cómo va?",
            &["ana".to_string()]
        ));
        assert!(!host_name_mentioned(
            "quien es anabel",
            &["ana".to_string()]
        ));
    }
}
