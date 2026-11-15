// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The text score: a character error rate per written line, independent of reading order.

/// Per-line character error rate: each truth line is matched against the best substring of
/// the whole prediction (semi-global edit distance), so reading order does not count.
///
/// Returns the sum of the edit distances over the non-empty normalised truth lines, divided by
/// max(1, the sum of their lengths in characters), so longer lines weigh more. Whitespace is
/// normalised first and Mermaid fences are removed from the prediction. A truth without any
/// text scores 0.0.
pub fn line_cer(prediction: &str, truth: &str) -> f64 {
    let pred: Vec<char> = normalise(&strip_mermaid(prediction)).chars().collect();
    let lines: Vec<Vec<char>> = truth
        .lines()
        .map(|l| normalise(l).chars().collect::<Vec<_>>())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return 0.0;
    }
    let errors: usize = lines
        .iter()
        .map(|line| best_substring_distance(line, &pred))
        .sum();
    let chars: usize = lines.iter().map(Vec::len).sum();
    errors as f64 / chars.max(1) as f64
}

/// Runs of whitespace become one space, and a space before `: ? . , !` is dropped.
fn normalise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space {
            if !out.is_empty() && !matches!(c, ':' | '?' | '.' | ',' | '!') {
                out.push(' ');
            }
            pending_space = false;
        }
        out.push(c);
    }
    out
}

/// Removes every fenced block whose opening fence is ```mermaid.
fn strip_mermaid(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_mermaid = false;
    for line in s.lines() {
        let t = line.trim_start();
        if in_mermaid {
            if t.starts_with("```") {
                in_mermaid = false;
            }
            continue;
        }
        if t.starts_with("```mermaid") {
            in_mermaid = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Smallest edit distance between `needle` and any substring of `haystack`.
fn best_substring_distance(needle: &[char], haystack: &[char]) -> usize {
    // prev[j]: distance of the needle prefix against a substring ending at haystack[j]. A
    // match may start anywhere, so the empty needle prefix costs 0 at every position.
    let mut prev = vec![0usize; haystack.len() + 1];
    for (i, &n) in needle.iter().enumerate() {
        let mut cur = vec![0usize; haystack.len() + 1];
        cur[0] = i + 1;
        for (j, &h) in haystack.iter().enumerate() {
            let sub = prev[j] + usize::from(n != h);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev.into_iter().min().unwrap_or(needle.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_cer_ignores_order() {
        assert_eq!(line_cer("c d a b", "a b\nc d"), 0.0);
    }

    #[test]
    fn line_cer_counts_errors() {
        let cer = line_cer("hallo", "hello");
        assert!((cer - 0.2).abs() < 1e-12, "{cer}");
    }

    #[test]
    fn line_cer_skips_mermaid() {
        let pred = "hello\n\n```mermaid\ngraph TD\n    zzz --> yyy\n```\n";
        assert_eq!(line_cer(pred, "hello"), 0.0);
        // Text inside the fence cannot be matched.
        assert_eq!(line_cer(pred, "zzz"), 1.0);
    }

    #[test]
    fn line_cer_weights_by_characters() {
        // One error in the 2-char line, none in the 8-char line: 1/10, not (0.5 + 0) / 2.
        let cer = line_cer("xb cdefghij", "ab\ncdefghij");
        assert!((cer - 0.1).abs() < 1e-12, "{cer}");
    }

    #[test]
    fn line_cer_normalises_whitespace() {
        assert_eq!(line_cer("What  is\nthis ?", "What is this?"), 0.0);
        assert_eq!(line_cer("a , b !", "a, b!"), 0.0);
    }

    #[test]
    fn line_cer_edge_cases() {
        assert_eq!(line_cer("anything", ""), 0.0);
        assert_eq!(line_cer("", "abcd"), 1.0);
    }
}
