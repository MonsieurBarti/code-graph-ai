#!/bin/bash
# codegraph-pretool-bash.sh — PreToolUse:Bash hook for Claude Code
# Auto-approves code-graph CLI calls to eliminate permission prompts.
# Returns permissionDecision='allow' when command starts with 'code-graph'.

# Guard: skip if dependencies missing
if ! command -v jq &>/dev/null; then
  exit 0
fi

# Guard: prevent recursion if hook triggers another tool call
if [ -n "$CLAUDE_HOOK_ACTIVE" ]; then
  exit 0
fi
export CLAUDE_HOOK_ACTIVE=1

INPUT=$(cat)
CMD=$(echo "$INPUT" | jq -r '.tool_input.command // empty' 2>/dev/null)

if [ -z "$CMD" ]; then
  exit 0
fi

# Guard: reject compound commands that chain after code-graph
if echo "$CMD" | grep -qE '(;|&&|\|\||`|\$\(|\$\{)'; then
  exit 0
fi

# Check if the command starts with 'code-graph' (with or without path prefix)
# Matches: "code-graph ...", "/path/to/code-graph ...", "~/.cargo/bin/code-graph ..."
case "$CMD" in
  code-graph\ *|code-graph|*/code-graph\ *|*/code-graph)
    jq -n '{
      "hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "permissionDecisionReason": "code-graph CLI auto-approved"
      }
    }'
    ;;
  *)
    # Not a code-graph command, passthrough
    exit 0
    ;;
esac
