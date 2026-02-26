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

/// Generate hint for get_file_summary results.
///
/// Points to get_imports as next step in the navigation funnel.
pub fn file_summary_hint(file_path: &str) -> String {
    format!(
        "\nhint: get_imports \"{}\" | alt: get_context \"<symbol>\"",
        file_path
    )
}

/// Generate hint for get_imports results.
///
/// Points to get_context as next step for investigating a specific dependency.
pub fn imports_hint(file_path: &str) -> String {
    format!(
        "\nhint: get_context \"<symbol>\" | alt: get_file_summary \"{}\"",
        file_path
    )
}

/// Generate hint for find_dead_code results.
pub fn dead_code_hint(unreachable_count: usize, unreferenced_count: usize) -> String {
    if unreachable_count == 0 && unreferenced_count == 0 {
        return "\nhint: no dead code found — try a broader scope".to_string();
    }
    "\nhint: get_file_summary \"<path>\" for details on flagged files".to_string()
}

/// Generate hint for list_projects results.
pub fn list_projects_hint() -> String {
    "\nhint: register_project to add another project | find_dead_code to analyze".to_string()
}

/// Generate hint for register_project results.
pub fn register_project_hint(alias: &str) -> String {
    format!("\nhint: find_symbol \".*\" project_path=\"{}\" to explore", alias)
}

/// Generate a combined hint for batch_query responses.
///
/// Strategy: If any query was a find_symbol with exactly one result,
/// suggest get_context for it. Otherwise, suggest batch_query as a reminder.
/// If no useful hint can be derived, return empty string.
pub fn batch_hint(queries: &[(&str, bool)]) -> String {
    // queries: slice of (tool_name, had_results)
    // Simple strategy: if batch had results, generic hint
    if queries.is_empty() {
        return String::new();
    }
    "\nhint: batch_query for multiple queries in one call".to_string()
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
    fn test_file_summary_hint() {
        let hint = file_summary_hint("src/main.rs");
        assert!(
            hint.contains("hint: get_imports \"src/main.rs\""),
            "file_summary hint should include get_imports with path: got '{}'",
            hint
        );
        assert!(
            hint.contains("alt: get_context"),
            "file_summary hint should include alt get_context: got '{}'",
            hint
        );
    }

    #[test]
    fn test_imports_hint() {
        let hint = imports_hint("src/main.rs");
        assert!(
            hint.contains("hint: get_context"),
            "imports hint should include get_context: got '{}'",
            hint
        );
        assert!(
            hint.contains("alt: get_file_summary \"src/main.rs\""),
            "imports hint should include alt get_file_summary with path: got '{}'",
            hint
        );
    }

    #[test]
    fn test_file_summary_hint_starts_with_newline() {
        let hint = file_summary_hint("src/lib.rs");
        assert!(
            hint.starts_with('\n'),
            "file_summary hint must start with newline: got '{}'",
            hint
        );
    }

    #[test]
    fn test_imports_hint_starts_with_newline() {
        let hint = imports_hint("src/lib.rs");
        assert!(
            hint.starts_with('\n'),
            "imports hint must start with newline: got '{}'",
            hint
        );
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

    #[test]
    fn test_dead_code_hint_no_dead_code() {
        let hint = dead_code_hint(0, 0);
        assert!(
            hint.contains("no dead code found"),
            "hint with zero dead code should say no dead code found: got '{}'",
            hint
        );
        assert!(hint.starts_with('\n'), "non-empty hint must start with newline");
    }

    #[test]
    fn test_dead_code_hint_with_dead_code() {
        let hint = dead_code_hint(2, 5);
        assert!(
            hint.contains("get_file_summary"),
            "hint with dead code should suggest get_file_summary: got '{}'",
            hint
        );
        assert!(hint.starts_with('\n'), "non-empty hint must start with newline");
    }

    #[test]
    fn test_list_projects_hint() {
        let hint = list_projects_hint();
        assert!(
            hint.contains("register_project"),
            "list_projects hint should mention register_project: got '{}'",
            hint
        );
        assert!(
            hint.contains("find_dead_code"),
            "list_projects hint should mention find_dead_code: got '{}'",
            hint
        );
        assert!(hint.starts_with('\n'), "non-empty hint must start with newline");
    }

    #[test]
    fn test_register_project_hint() {
        let hint = register_project_hint("/path/to/project");
        assert!(
            hint.contains("find_symbol"),
            "register_project hint should mention find_symbol: got '{}'",
            hint
        );
        assert!(
            hint.contains("/path/to/project"),
            "register_project hint should include the alias: got '{}'",
            hint
        );
        assert!(hint.starts_with('\n'), "non-empty hint must start with newline");
    }

    #[test]
    fn test_batch_hint_nonempty() {
        let queries = vec![("find_symbol", true), ("get_stats", false)];
        let hint = batch_hint(&queries);
        assert!(
            hint.contains("batch_query"),
            "batch hint with non-empty queries should contain 'batch_query': got '{}'",
            hint
        );
    }

    #[test]
    fn test_batch_hint_empty() {
        let hint = batch_hint(&[]);
        assert_eq!(hint, "", "batch_hint with empty slice should return empty string");
    }

    #[test]
    fn test_batch_hint_starts_with_newline() {
        let queries = vec![("find_symbol", true)];
        let hint = batch_hint(&queries);
        assert!(
            hint.starts_with('\n'),
            "batch hint with non-empty queries must start with newline: got '{}'",
            hint
        );
    }
}
