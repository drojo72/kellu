//! Token budget planning: estimate what a prompt costs and compact it before it is sent.
//!
//! Rules (see `docs/ROADMAP.md`, "Token and cost management"):
//!
//! * Planning never calls a model. Spending tokens to save tokens defeats the purpose.
//! * Compaction loses the cheapest information first and is deterministic.
//! * If the prompt still does not fit, it is refused. A truncated task invites an invented answer.
//!
//! The estimate only plans. There is no exact tokenizer for every service, so it is deliberately
//! pessimistic; the service's own count is what a ledger records, and the ratio between the two,
//! measured per model (see `usage`), scales the estimate.

use kellu_providers::{ChatRequest, Message};

/// Estimated tokens for `text`.
///
/// ASCII costs a quarter token per character (typical for code and English); anything else costs
/// half a token, because non-Latin scripts, accents and symbols split into more tokens. The result
/// is rounded up, so an empty string is the only text that costs nothing.
#[must_use]
pub fn estimate_tokens(text: &str) -> u32 {
    let mut quarters: u64 = 0;
    for ch in text.chars() {
        quarters += if ch.is_ascii() { 1 } else { 2 };
    }
    u32::try_from(quarters.div_ceil(4)).unwrap_or(u32::MAX)
}

/// Estimated tokens of the text of a whole request: every message, and the arguments of the tool
/// calls the model made earlier. The tool specifications are not counted (nothing sends them yet);
/// the calibration absorbs a constant overhead, not a large one that varies.
#[must_use]
pub fn estimate_request(request: &ChatRequest) -> u32 {
    let mut total = 0_u32;
    for message in &request.messages {
        let tokens = match message {
            Message::System(text) | Message::User(text) | Message::Tool { content: text, .. } => {
                estimate_tokens(text)
            }
            Message::Assistant {
                content,
                tool_calls,
            } => tool_calls
                .iter()
                .fold(estimate_tokens(content), |sum, call| {
                    call.arguments.iter().fold(
                        sum.saturating_add(estimate_tokens(&call.name)),
                        |sum, (k, v)| {
                            sum.saturating_add(estimate_tokens(k))
                                .saturating_add(estimate_tokens(&v.to_string()))
                        },
                    )
                }),
        };
        total = total.saturating_add(tokens);
    }
    total
}

/// `tokens` scaled by `milli` thousandths (1000 changes nothing), rounded up.
#[must_use]
pub fn scale_tokens(tokens: u32, milli: u32) -> u32 {
    let scaled = (u64::from(tokens) * u64::from(milli)).div_ceil(1000);
    u32::try_from(scaled).unwrap_or(u32::MAX)
}

/// Replace every run of two or more blank lines by one, and drop trailing spaces and tabs at the
/// end of each line except the last one.
///
/// The last line is left alone on purpose: in the text before a cursor it is the line being typed,
/// and its trailing space (`let x = `) is part of what the model must continue. Lines keep their
/// order.
#[must_use]
pub fn collapse_blank_runs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blank_run = 0_usize;
    let last = text.matches('\n').count();
    for (index, line) in text.split('\n').enumerate() {
        let line = if index == last {
            line
        } else {
            line.trim_end_matches([' ', '\t', '\r'])
        };
        if line.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        if index > 0 {
            out.push('\n');
        }
        out.push_str(line);
    }
    out
}

/// The largest size, in characters, of the two windows around a cursor that fits `budget_chars`.
///
/// Both windows shrink by the same factor, so the code nearest the cursor survives on each side.
/// Returns `None` when even `floor` characters per side do not fit.
#[must_use]
pub fn narrow_windows(
    before: usize,
    after: usize,
    budget_chars: usize,
    floor: usize,
) -> Option<(usize, usize)> {
    let total = before.saturating_add(after);
    if total <= budget_chars {
        return Some((before, after));
    }
    if budget_chars < floor.saturating_mul(2) {
        return None;
    }
    // Scale in integer arithmetic: share = budget * side / total.
    let scaled = |side: usize| -> usize {
        let wide = u128::try_from(budget_chars).unwrap_or(u128::MAX)
            * u128::try_from(side).unwrap_or(0)
            / u128::try_from(total).unwrap_or(1);
        usize::try_from(wide).unwrap_or(usize::MAX)
    };
    let before_share = scaled(before).max(floor).min(before);
    let after_share = scaled(after).max(floor).min(after);
    // The floors can push the sum over the budget; give the excess back to the larger side.
    let over = before_share
        .saturating_add(after_share)
        .saturating_sub(budget_chars);
    if before_share >= after_share {
        Some((
            before_share.saturating_sub(over).max(floor.min(before)),
            after_share,
        ))
    } else {
        Some((
            before_share,
            after_share.saturating_sub(over).max(floor.min(after)),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_free_and_ascii_costs_a_quarter_token() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2, "rounded up");
    }

    #[test]
    fn non_ascii_costs_more_than_ascii() {
        assert!(estimate_tokens("ñññññññ") > estimate_tokens("nnnnnnn"));
        assert_eq!(estimate_tokens("ñ"), 1);
        assert_eq!(estimate_tokens("日本語"), 2);
    }

    #[test]
    fn estimate_never_overflows() {
        let huge = "x".repeat(1_000_000);
        assert_eq!(estimate_tokens(&huge), 250_000);
    }

    #[test]
    fn blank_runs_collapse_and_trailing_space_goes() {
        assert_eq!(collapse_blank_runs("a\n\n\n\nb"), "a\n\nb");
        assert_eq!(collapse_blank_runs("a   \nb\t\r\n"), "a\nb\n");
        assert_eq!(collapse_blank_runs("a\nb"), "a\nb");
        assert_eq!(collapse_blank_runs(""), "");
    }

    #[test]
    fn a_request_is_the_sum_of_its_messages() {
        let request = ChatRequest::new(vec![
            Message::system("abcd"),
            Message::user("abcdefgh"),
            Message::assistant("abcd"),
        ])
        .unwrap();
        assert_eq!(estimate_request(&request), 1 + 2 + 1);
    }

    #[test]
    fn scaling_rounds_up_and_never_overflows() {
        assert_eq!(scale_tokens(100, 1000), 100);
        assert_eq!(scale_tokens(100, 1500), 150);
        assert_eq!(scale_tokens(3, 500), 2);
        assert_eq!(scale_tokens(0, 3000), 0);
        assert_eq!(scale_tokens(u32::MAX, 3000), u32::MAX);
    }

    #[test]
    fn the_line_being_typed_keeps_its_trailing_space() {
        assert_eq!(collapse_blank_runs("a\n\n\nlet x = "), "a\n\nlet x = ");
        assert_eq!(collapse_blank_runs("let x = "), "let x = ");
    }

    #[test]
    fn collapsing_keeps_indentation_and_single_blank_lines() {
        let code = "fn a() {\n    let x = 1;\n\n    x\n}\n";
        assert_eq!(collapse_blank_runs(code), code);
    }

    #[test]
    fn windows_that_fit_are_untouched() {
        assert_eq!(narrow_windows(100, 50, 200, 10), Some((100, 50)));
    }

    #[test]
    fn windows_shrink_together_and_fit_the_budget() {
        let (before, after) = narrow_windows(6000, 2000, 4000, 200).unwrap();
        assert!(before + after <= 4000, "{before}+{after}");
        assert!(before > after, "the larger side stays larger");
        assert!(after >= 200);
    }

    #[test]
    fn floors_are_respected_or_the_request_is_refused() {
        assert_eq!(narrow_windows(6000, 2000, 399, 200), None);
        let (before, after) = narrow_windows(6000, 10, 450, 200).unwrap();
        assert!(before + after <= 450, "{before}+{after}");
        assert_eq!(after, 10, "a side shorter than the floor keeps what it has");
    }

    #[test]
    fn zero_sized_inputs_do_not_panic() {
        assert_eq!(narrow_windows(0, 0, 0, 0), Some((0, 0)));
        assert_eq!(narrow_windows(10, 0, 4, 0), Some((4, 0)));
    }
}
