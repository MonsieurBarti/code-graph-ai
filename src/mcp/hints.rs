/// Hint generation functions for MCP tool responses.
///
/// Each function returns a `String` that starts with `\n` to ensure the hint
/// appears on its own line after the response body. Empty string means no hint.

/// Generate hint for find_symbol results.
///
/// - `symbol`: the queried symbol name
/// - `result_count`: number of results returned
/// - `truncated`: whether results were truncated
/// - `first_result_name`: name of the first result (if any)
pub fn find_hint(
    symbol: &str,
    result_count: usize,
    truncated: bool,
    first_result_name: Option<&str>,
) -> String {
    if truncated {
        // Suggest narrowing with kind filter
        return format!("\nhint: find_symbol \"{}\" kind=function", symbol);
    }
    match result_count {
        0 => String::new(),
        1 => {
            let name = first_result_name.unwrap_or(symbol);
            format!(
                "\nhint: get_context \"{}\" | alt: find_references \"{}\"",
                name, name
            )
        }
        2..=5 => {
            let name = first_result_name.unwrap_or(symbol);
            format!(
                "\nhint: get_context \"{}\" | alt: find_symbol \"{}\" kind=function",
                name, symbol
            )
        }
        _ => {
            // > 5 results: suggest narrowing by kind
            format!("\nhint: find_symbol \"{}\" kind=function", symbol)
        }
    }
}

/// Generate hint for find_references results.
///
/// - `symbol`: the queried symbol name
pub fn refs_hint(symbol: &str) -> String {
    format!(
        "\nhint: get_context \"{}\" | alt: get_impact \"{}\"",
        symbol, symbol
    )
}

/// Generate hint for get_impact results.
///
/// - `symbol`: the queried symbol name
pub fn impact_hint(symbol: &str) -> String {
    format!(
        "\nhint: get_context \"{}\" | alt: find_references \"{}\"",
        symbol, symbol
    )
}

/// Generate hint for detect_circular results.
///
/// - `cycle_count`: number of cycles found
///
/// Returns empty string when no cycles found (no useful follow-up).
pub fn circular_hint(cycle_count: usize) -> String {
    if cycle_count == 0 {
        String::new()
    } else {
        // Cycles found but without specific file context here, return empty string.
        // Context-dependent follow-up is better left to the caller.
        String::new()
    }
}

/// Generate hint for get_context results.
///
/// - `symbol`: the queried symbol name
pub fn context_hint(symbol: &str) -> String {
    format!(
        "\nhint: find_references \"{}\" | alt: get_impact \"{}\"",
        symbol, symbol
    )
}

/// Generate hint for get_stats results.
pub fn stats_hint() -> String {
    "\nhint: find_symbol \".*\" kind=function".to_string()
}

/// Generate hint for get_structure results.
///
/// Points to get_file_summary as the next step in the navigation funnel.
/// If a specific path was queried, suggest it; otherwise use a generic placeholder.
pub fn structure_hint(path: Option<&str>) -> String {
    if let Some(p) = path {
        format!("\nhint: get_file_summary \"{}\"", p)
    } else {
        "\nhint: get_file_summary \"<path>\" | alt: find_symbol \".*\"".to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_hint_single_result() {
        let hint = find_hint("Foo", 1, false, Some("Foo"));
        assert!(
            hint.contains("hint: get_context \"Foo\""),
            "single result should hint get_context: got '{}'",
            hint
        );
        assert!(
            hint.contains("alt: find_references \"Foo\""),
            "single result should include alt find_references: got '{}'",
            hint
        );
    }

    #[test]
    fn test_find_hint_multiple_results() {
        let hint = find_hint("Foo", 3, false, Some("FooBar"));
        assert!(
            hint.contains("hint: get_context \"FooBar\""),
            "multiple results should hint get_context with first name: got '{}'",
            hint
        );
    }

    #[test]
    fn test_find_hint_truncated() {
        let hint = find_hint("Foo", 25, true, Some("Foo"));
        assert!(
            hint.contains("kind="),
            "truncated results should suggest narrowing with kind=: got '{}'",
            hint
        );
    }

    #[test]
    fn test_find_hint_many_results() {
        let hint = find_hint("Foo", 10, false, Some("Foo"));
        assert!(
            hint.contains("kind="),
            "many results (>5) should suggest narrowing with kind=: got '{}'",
            hint
        );
    }

    #[test]
    fn test_refs_hint() {
        let hint = refs_hint("Bar");
        assert!(
            hint.contains("hint: get_context \"Bar\""),
            "refs hint should suggest get_context: got '{}'",
            hint
        );
        assert!(
            hint.contains("alt: get_impact \"Bar\""),
            "refs hint should include alt get_impact: got '{}'",
            hint
        );
    }

    #[test]
    fn test_impact_hint() {
        let hint = impact_hint("Baz");
        assert!(
            hint.contains("hint: get_context \"Baz\""),
            "impact hint should suggest get_context: got '{}'",
            hint
        );
    }

    #[test]
    fn test_context_hint() {
        let hint = context_hint("Qux");
        assert!(
            hint.contains("hint: find_references \"Qux\""),
            "context hint should suggest find_references: got '{}'",
            hint
        );
    }

    #[test]
    fn test_stats_hint() {
        let hint = stats_hint();
        assert!(
            hint.contains("hint: find_symbol"),
            "stats hint should suggest find_symbol: got '{}'",
            hint
        );
    }

    #[test]
    fn test_circular_hint_no_cycles() {
        let hint = circular_hint(0);
        assert_eq!(hint, "", "no cycles -> empty hint string");
    }

    #[test]
    fn test_hints_start_with_newline() {
        // All non-empty hint returns must start with \n
        let hints = vec![
            find_hint("Foo", 1, false, Some("Foo")),
            find_hint("Foo", 3, false, Some("Foo")),
            find_hint("Foo", 25, true, Some("Foo")),
            find_hint("Foo", 10, false, Some("Foo")),
            refs_hint("Bar"),
            impact_hint("Baz"),
            context_hint("Qux"),
            stats_hint(),
        ];
        for hint in hints {
            if !hint.is_empty() {
                assert!(
                    hint.starts_with('\n'),
                    "non-empty hint must start with newline: got '{}'",
                    hint
                );
            }
        }
    }

    #[test]
    fn test_structure_hint_with_path() {
        let hint = structure_hint(Some("src/main.rs"));
        assert!(
            hint.contains("hint: get_file_summary \"src/main.rs\""),
            "structure hint with path should include get_file_summary with path: got '{}'",
            hint
        );
    }

    #[test]
    fn test_structure_hint_without_path() {
        let hint = structure_hint(None);
        assert!(
            hint.contains("hint: get_file_summary"),
            "structure hint without path should still contain get_file_summary: got '{}'",
            hint
        );
    }

    #[test]
    fn test_structure_hint_starts_with_newline() {
        let hint_with_path = structure_hint(Some("src/lib.rs"));
        let hint_without_path = structure_hint(None);
        assert!(
            hint_with_path.starts_with('\n'),
            "structure hint with path must start with newline: got '{}'",
            hint_with_path
        );
        assert!(
            hint_without_path.starts_with('\n'),
            "structure hint without path must start with newline: got '{}'",
            hint_without_path
        );
    }
}
