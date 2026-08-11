use crate::types::{Replacement, SpanInfo};

/// Apply replacements to source in reverse byte-offset order (format-preserving).
pub fn apply_replacements(source: &str, replacements: &mut Vec<Replacement>) -> String {
    if replacements.is_empty() {
        return source.to_string();
    }
    // Sort by start descending — apply from end to preserve earlier offsets
    replacements.sort_by(|a, b| b.start.cmp(&a.start));
    let mut result = source.to_string();
    for r in replacements.iter() {
        let start = r.start as usize;
        let end = r.end as usize;
        if start <= result.len() && end <= result.len() && start <= end {
            result = format!("{}{}{}", &result[..start], r.new_text, &result[end..]);
        }
    }
    result
}

/// Convert byte offset to line/column info (1-based).
pub fn span_to_info(source: &str, start: u32, end: u32) -> SpanInfo {
    let mut line = 1u32;
    let mut col = 1u32;
    for (i, ch) in source.char_indices() {
        if i == start as usize {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    SpanInfo {
        start,
        end,
        line,
        col,
    }
}

/// Remove overlapping replacements. Keep the first (wider-scope) one when sorted by start asc.
pub fn dedup_overlapping(replacements: &mut Vec<Replacement>) {
    if replacements.len() < 2 {
        return;
    }
    // Sort by start ascending, then by end descending (wider first)
    replacements.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut max_end = 0u32;
    let mut kept = vec![true; replacements.len()];
    for (i, r) in replacements.iter().enumerate() {
        if i == 0 {
            max_end = r.end;
            continue;
        }
        if r.start < max_end {
            // Overlaps — skip the later (narrower) one
            kept[i] = false;
        } else {
            max_end = r.end;
        }
    }
    let mut idx = 0;
    replacements.retain(|_| {
        let k = kept[idx];
        idx += 1;
        k
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Replacement;

    fn r(start: u32, end: u32, new_text: &str) -> Replacement {
        Replacement {
            start,
            end,
            new_text: new_text.to_string(),
            message: String::new(),
        }
    }

    #[test]
    fn single_replacement_at_start() {
        let mut reps = vec![r(0, 2, "rq")];
        assert_eq!(apply_replacements("pm.test", &mut reps), "rq.test");
    }

    #[test]
    fn single_replacement_at_end() {
        let mut reps = vec![r(5, 7, "rq")];
        assert_eq!(apply_replacements("call pm", &mut reps), "call rq");
    }

    #[test]
    fn single_replacement_in_middle() {
        let mut reps = vec![r(2, 4, "rq")];
        assert_eq!(apply_replacements("a pm b", &mut reps), "a rq b");
    }

    #[test]
    fn multiple_non_overlapping() {
        let mut reps = vec![r(0, 2, "rq"), r(6, 8, "rq")];
        assert_eq!(apply_replacements("pm.a; pm.b", &mut reps), "rq.a; rq.b");
    }

    #[test]
    fn length_changing_replacement() {
        let mut reps = vec![r(0, 12, "rq.response.text()")];
        assert_eq!(
            apply_replacements("responseBody", &mut reps),
            "rq.response.text()"
        );
    }

    #[test]
    fn empty_replacement_list() {
        let mut reps = vec![];
        assert_eq!(apply_replacements("code", &mut reps), "code");
    }

    #[test]
    fn adjacent_replacements_no_overlap() {
        let mut reps = vec![r(0, 2, "ab"), r(2, 4, "cd")];
        assert_eq!(apply_replacements("xxyy", &mut reps), "abcd");
    }

    #[test]
    fn overlapping_replacements_deduplicated() {
        let mut reps = vec![r(0, 5, "wide"), r(1, 3, "narrow")];
        dedup_overlapping(&mut reps);
        assert_eq!(reps.len(), 1);
        assert_eq!(reps[0].new_text, "wide");
    }

    #[test]
    fn span_to_info_single_line() {
        let info = span_to_info("abc", 1, 2);
        assert_eq!(info.line, 1);
        assert_eq!(info.col, 2);
    }

    #[test]
    fn span_to_info_multi_line() {
        let info = span_to_info("a\nb\nc", 4, 5);
        assert_eq!(info.line, 3);
        assert_eq!(info.col, 1);
    }

    #[test]
    fn span_to_info_offset_zero() {
        let info = span_to_info("hello", 0, 1);
        assert_eq!(info.line, 1);
        assert_eq!(info.col, 1);
    }
}
