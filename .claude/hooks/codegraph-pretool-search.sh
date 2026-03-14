#!/bin/bash
# codegraph-pretool-search.sh — PreToolUse:Grep|Glob hook for Claude Code
# Hybrid smart routing: intercepts search calls when the pattern looks like
# a code symbol and enriches with code-graph results. Passes through for
# string literals, regex, TODOs, file paths, error messages, etc.
#
# Replaces code-graph-enrichment.sh with improved classification logic.

# Guard: skip if dependencies missing
if ! command -v jq &>/dev/null || ! command -v code-graph &>/dev/null; then
  exit 0
fi

# Guard: prevent recursion if code-graph itself triggers Grep/Glob
if [ -n "$CLAUDE_HOOK_ACTIVE" ]; then
  exit 0
fi
export CLAUDE_HOOK_ACTIVE=1

INPUT=$(cat)

PATTERN=$(echo "$INPUT" | jq -r '.tool_input.pattern // .tool_input.glob // empty' 2>/dev/null)

# Must have a pattern to work with
if [ -z "$PATTERN" ]; then
  exit 0
fi

# Guard: patterns longer than 256 chars are not real symbols
if [ ${#PATTERN} -gt 256 ]; then
  exit 0
fi

# --- Non-symbol classification (passthrough) ---
# These patterns are clearly NOT code symbols and should use native Grep/Glob.

# 1. File glob patterns (contain *, ?, **, or file extensions like .rs, .ts)
[[ "$PATTERN" =~ (\*\*|[*?]|\.[a-z]{1,5}$) ]] && exit 0

# 2. String literals (quoted)
[[ "$PATTERN" == \"* ]] || [[ "$PATTERN" == *\" ]] || [[ "$PATTERN" == \'* ]] || [[ "$PATTERN" == *\' ]] && exit 0

# 3. TODO/FIXME markers
[[ "$PATTERN" =~ ^(TODO|FIXME|HACK|NOTE|XXX|WARN|BUG) ]] && exit 0

# 4. Error messages / natural language (lowercase words with spaces)
[[ "$PATTERN" =~ ^[a-z].*[[:space:]].*[[:space:]] ]] && exit 0

# 5. File paths
[[ "$PATTERN" =~ [/\\] ]] && exit 0

# 6. Complex regex metacharacters
[[ "$PATTERN" =~ (\(\?[=!\<]|\|.*\||[+*?]\{|\\[a-zA-Z]) ]] && exit 0

# 7. Short lowercase words (likely keywords)
[[ "$PATTERN" =~ ^[a-z]{1,6}$ ]] && exit 0

# --- Symbol classification (intercept) ---
# A pattern is a symbol if it matches one of these:

IS_SYMBOL=0

# PascalCase: starts with uppercase, has at least one more uppercase letter
# e.g., UserService, CodeGraph, AstNode, HashMap
[[ "$PATTERN" =~ ^[A-Z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9]*$ ]] && IS_SYMBOL=1

# snake_case: lowercase with underscores, at least one underscore
# e.g., build_graph, find_symbol, parse_file
[[ "$PATTERN" =~ ^[a-z][a-z0-9]*(_[a-z][a-z0-9]*)+$ ]] && IS_SYMBOL=1

# SCREAMING_SNAKE_CASE: all uppercase with underscores (constants)
# e.g., MAX_DEPTH, DEFAULT_CONFIG
[[ "$PATTERN" =~ ^[A-Z][A-Z0-9]*(_[A-Z][A-Z0-9]*)+$ ]] && IS_SYMBOL=1

# camelCase: starts lowercase, has at least one uppercase letter
# e.g., buildGraph, findSymbol, parseFile
[[ "$PATTERN" =~ ^[a-z][a-zA-Z0-9]*[A-Z][a-zA-Z0-9]*$ ]] && IS_SYMBOL=1

# Single PascalCase word (e.g., Parser, Graph, Node) — at least 3 chars
[[ "$PATTERN" =~ ^[A-Z][a-z][a-zA-Z0-9]{1,}$ ]] && IS_SYMBOL=1

# Longer snake_case-like identifiers (single word, 7+ chars, lowercase)
# e.g., petgraph, graphology, treesitter
# Skip these — too ambiguous, could be package names or text

if [ "$IS_SYMBOL" -ne 1 ]; then
  exit 0
fi

# --- Run code-graph and return enrichment context ---
GRAPH_CONTEXT=$(code-graph find -- "$PATTERN" . 2>/dev/null | head -30)

if [ -z "$GRAPH_CONTEXT" ]; then
  # No results from code-graph, let native search proceed without context
  exit 0
fi

# Return additionalContext with graph results
# Use jq to properly escape the context string
jq -n \
  --arg pattern "$PATTERN" \
  --arg context "$GRAPH_CONTEXT" \
  '{
    "hookSpecificOutput": {
      "hookEventName": "PreToolUse",
      "additionalContext": ("code-graph results for \u0027" + $pattern + "\u0027:\n" + $context)
    }
  }'
