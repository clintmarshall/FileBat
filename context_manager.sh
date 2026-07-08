#!/bin/bash

# context_manager.sh — Autonomous Context Handoff Watchdog
#
# Monitors llama.cpp server context usage. When context fills up,
# triggers a handoff: Claude saves state, server resets, fresh session continues.
#
# Writes structured state to context_state.json on every poll —
# dashboard-ready, no magic, just a file.
#
# Usage: bash context_manager.sh &
#
# Files:
#   context_state.json          — Structured state (written every poll)
#   sessions/.handoff-triggered — Watchdog → Claude: "save your state"
#   sessions/.handoff-done      — Claude → Watchdog: "state saved, proceed"

# --- Configuration ---

SERVER_HOST="http://localhost:8989"
PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
SESSIONS_DIR="$PROJECT_DIR/sessions"
STATE_FILE="$PROJECT_DIR/context_state.json"
TRIGGER_FILE="$SESSIONS_DIR/.handoff-triggered"
DONE_FILE="$SESSIONS_DIR/.handoff-done"

THRESHOLD=90
COOLDOWN=120
POLL_INTERVAL=10

# --- JSON State Management ---

# Atomic write: write to .tmp, then rename (prevents partial reads)
write_state() {
    local status="$1"
    local tokens_used="$2"
    local tokens_max="$3"
    local percent="$4"
    local server_up="$5"
    local model="$6"
    local handoff_count="$7"
    local last_handoff="$8"
    local last_result="$9"
    local last_session="${10}"
    local history="${11}"

    local tmp="${STATE_FILE}.tmp"
    local now
    now=$(date -u '+%Y-%m-%dT%H:%M:%SZ')

    cat > "$tmp" << STATE_EOF
{
  "version": 1,
  "updated": "$now",
  "config": {
    "server_host": "$SERVER_HOST",
    "threshold_percent": $THRESHOLD,
    "cooldown_seconds": $COOLDOWN,
    "poll_interval_seconds": $POLL_INTERVAL
  },
  "current": {
    "status": "$status",
    "tokens_used": $tokens_used,
    "tokens_max": $tokens_max,
    "percent": $percent,
    "server_up": $server_up,
    "last_poll": "$now",
    "model": "$model"
  },
  "handoff": {
    "total_count": $handoff_count,
    "last_handoff": "$last_handoff",
    "last_result": "$last_result",
    "last_session_file": "$last_session"
  },
  "history": [
$history
  ]
}
STATE_EOF
    mv "$tmp" "$STATE_FILE"
}

# Initialize empty state
init_state() {
    write_state "initializing" 0 0 0 "false" "" 0 "" "" "" ""
}

# Append a handoff entry to history (returns formatted JSON block)
# Usage: append_history "existing_history" "new_entry"
# Returns the full history block with proper comma separation
append_history() {
    local existing="$1"
    local new_entry="$2"

    if [ -z "$existing" ]; then
        echo "$new_entry"
    else
        # existing ends without comma, add it, then new entry
        printf '%s,\n' "$existing"
        echo "$new_entry"
    fi
}

# --- Helper Functions ---

get_context_usage() {
    # Fetch current token count and max context from the server
    # Returns: "current_tokens max_tokens percent model" or empty on failure

    local metrics
    metrics=$(curl -s --max-time 5 "$SERVER_HOST/metrics" 2>/dev/null) || return

    local props
    props=$(curl -s --max-time 5 "$SERVER_HOST/props" 2>/dev/null) || return

    # Extract current token count from prometheus metrics
    local current
    current=$(echo "$metrics" | grep -oP 'llamacpp:n_tokens_max\s+\K[\d.]+' | head -1) || return

    # Extract max context from props
    local max_ctx
    max_ctx=$(echo "$props" | grep -oP '"n_ctx"\s*:\s*\K[\d]+' | head -1) || true

    # Fallback: hardcoded from launch script if props doesn't expose it
    if [ -z "$max_ctx" ]; then
        max_ctx=200000
    fi

    # Extract model name from props
    local model
    model=$(echo "$props" | grep -oP '"model_alias"\s*:\s*"\K[^"]+' | head -1) || true
    if [ -z "$model" ]; then
        model=$(echo "$props" | grep -oP '"model_path"\s*:\s*"\K[^"]+' | head -1) || true
    fi
    if [ -z "$model" ]; then
        model="unknown"
    fi

    if [ -z "$current" ] || [ "$max_ctx" -eq 0 ]; then
        return
    fi

    local percent
    percent=$(awk "BEGIN {printf \"%.1f\", ($current / $max_ctx) * 100}")

    echo "$current $max_ctx $percent $model"
}

alert() {
    local msg="$1"
    echo "⚠️  $(date '+%H:%M:%S') — $msg"
    powershell.exe -Command "Add-Type -AssemblyName PresentationFramework; [System.Windows.MessageBox]::Show('$msg', 'Context Watchdog', 'None', 'Warning')" 2>/dev/null || true
}

wait_for_server() {
    echo "⏳ Waiting for llama-server to restart..."
    local i=0
    while [ $i -lt 30 ]; do
        if curl -s --max-time 2 "$SERVER_HOST/props" >/dev/null 2>&1; then
            echo "✅ Server is back up after $((i + 1))s"
            return 0
        fi
        sleep 1
        i=$((i + 1))
    done
    echo "❌ Server did not restart within 30s"
    return 1
}

do_handoff() {
    local tokens_at_trigger="$1"
    local percent_at_trigger="$2"
    local model="$3"
    local handoff_count="$4"
    local history="$5"

    local handoff_start
    handoff_start=$(date +%s)
    local handoff_ts
    handoff_ts=$(date -u '+%Y-%m-%dT%H:%M:%SZ')

    echo ""
    echo "========================================="
    echo "🔄 Context Handoff Initiated"
    echo "   Tokens: $tokens_at_trigger / $MAX_TOKENS ($percent_at_trigger%)"
    echo "========================================="

    # Update state: handoff in progress
    write_state "handoff_in_progress" "$tokens_at_trigger" "$MAX_TOKENS" "$percent_at_trigger" "true" "$model" "$handoff_count" "$handoff_ts" "" "" "$history"

    # Step 1: Signal Claude to save state
    echo "→ Creating handoff trigger file..."
    echo "Triggered at $(date '+%Y-%m-%d %H:%M:%S')" > "$TRIGGER_FILE"
    alert "Context at ${percent_at_trigger}%. Saving session state..."

    # Step 2: Wait for Claude to save state (timeout: 60s)
    echo "→ Waiting for Claude to save session state..."
    local claude_saved="false"
    local i=0
    while [ $i -lt 60 ]; do
        if [ -f "$DONE_FILE" ]; then
            echo "✅ Claude saved state ($(cat "$DONE_FILE"))"
            rm -f "$TRIGGER_FILE" "$DONE_FILE"
            claude_saved="true"
            break
        fi
        sleep 1
        i=$((i + 1))
    done

    if [ "$claude_saved" = "false" ]; then
        echo "⚠️  Timeout waiting for Claude to save state (60s)"
        echo "→ Proceeding with handoff anyway..."
        rm -f "$TRIGGER_FILE"
    fi

    # Step 3: Kill Claude Code (frees the terminal)
    # NOTE: Only kill claude.exe — do NOT kill node.exe (that nukes Vite, Tauri dev, etc.)
    echo "→ Stopping Claude Code..."
    taskkill //F //IM claude.exe 2>/dev/null || true
    sleep 2

    # Step 4: Kill llama-server (restart loop brings it back fresh)
    echo "→ Resetting llama-server (KV cache clear)..."
    taskkill //F //IM llama-server.exe 2>/dev/null || true

    # Step 5: Wait for server to restart
    local result="failure"
    local session_file=""
    if wait_for_server; then
        # Step 6: Reset context via API (ensure clean slate)
        echo "→ Clearing context via /reset..."
        curl -s -X POST "$SERVER_HOST/reset" >/dev/null 2>&1 || true
        sleep 2

        # Step 7: Write resume prompt to a file (avoids shell quoting issues)
        # This file carries the operating instructions — the new session has no memory
        # of how the handoff system works, so we must tell it everything.
        local prompt_file="$SESSIONS_DIR/.resume-prompt.md"
        cat > "$prompt_file" << PROMPT_EOF
# Handoff Resume — READ THIS FIRST

You are a new Claude Code session that has been launched after a context handoff.
The previous session was terminated because the context window filled up.

## What You Must Do

1. **Read these files in order:**
   - $SESSIONS_DIR/SESSION-RESUME.md — resume brief (where we left off)
   - $SESSIONS_DIR/SESSION-CURRENT.md — full session state

2. **Continue the work.** Do not ask for confirmation — pick up where we left off.

3. **Maintain the session system:**
   - Update $SESSIONS_DIR/SESSION-CURRENT.md after every completed task
   - Check if $SESSIONS_DIR/.handoff-triggered exists at natural breakpoints
   - If .handoff-triggered exists: write SESSION-RESUME.md, create .handoff-done
   - When the session ends: update SESSION-RESUME.md with current state

4. **The watchdog is running.** A background script monitors context usage.
   When it fills up, it will create .handoff-triggered and eventually relaunch you.
   Your job is to save state when you see that file.

## Key Files

| File | Purpose |
|------|---------|
| $SESSIONS_DIR/SESSION-RESUME.md | Resume brief — read first |
| $SESSIONS_DIR/SESSION-CURRENT.md | Living session state — update as you work |
| $SESSIONS_DIR/.handoff-triggered | Watchdog signal — save state when this appears |
| $SESSIONS_DIR/.handoff-done | Your signal back — state saved, proceed |
| $PROJECT_DIR/context_state.json | Watchdog state — dashboard-ready |
| $PROJECT_DIR/context_manager.sh | The watchdog script |
PROMPT_EOF

        # Step 8: Launch new Claude Code session
        echo "→ Launching fresh Claude Code session..."
        session_file="SESSION-RESUME.md"

        start bash -c "cd \"$PROJECT_DIR\" && ANTHROPIC_BASE_URL='http://localhost:8989' ANTHROPIC_AUTH_TOKEN='sk-no-key-require' ANTHROPIC_MODEL='Qwen3.6-27B-UD-Q4_K_XL.gguf' claude --permission-mode auto 'You are a new session after a context handoff. Read $prompt_file for complete instructions on what to do and how to continue the work.'; exec bash"

        result="success"
    fi

    # Calculate duration
    local handoff_end
    handoff_end=$(date +%s)
    local duration=$((handoff_end - handoff_start))

    # Build history entry
    local new_entry
    new_entry=$(cat << ENTRY_EOF
    {
      "timestamp": "$handoff_ts",
      "tokens_at_trigger": $tokens_at_trigger,
      "percent_at_trigger": $percent_at_trigger,
      "result": "$result",
      "session_file": "$session_file",
      "duration_seconds": $duration
    }
ENTRY_EOF
)

    local new_history
    new_history=$(append_history "$history" "$new_entry")

    # Update state: handoff complete
    local next_count=$((handoff_count + 1))
    write_state "monitoring" 0 "$MAX_TOKENS" 0 "true" "$model" "$next_count" "$handoff_ts" "$result" "$session_file" "$new_history"

    echo ""
    echo "✅ Handoff $result! New session launched."
    echo "   Duration: ${duration}s"
    echo "========================================="
    echo ""
}

# --- Main Loop ---

echo "============================================="
echo "🧠 Context Watchdog Active"
echo "   Server: $SERVER_HOST"
echo "   Threshold: ${THRESHOLD}%"
echo "   Poll interval: ${POLL_INTERVAL}s"
echo "   State file: $STATE_FILE"
echo "   Sessions: $SESSIONS_DIR"
echo "============================================="
echo ""

# Initialize state file
init_state

# Persistent state variables
LAST_HANDOFF=0
HANDOFF_COUNT=0
LAST_HANDOFF_TS=""
LAST_RESULT=""
LAST_SESSION=""
HISTORY=""
MAX_TOKENS=200000
MODEL=""

while true; do
    sleep "$POLL_INTERVAL"

    # Check if server is reachable
    USAGE=$(get_context_usage)
    if [ -z "$USAGE" ]; then
        # Server down — update state
        if [ -n "$MAX_TOKENS" ] && [ "$MAX_TOKENS" -gt 0 ]; then
            write_state "server_down" 0 "$MAX_TOKENS" 0 "false" "$MODEL" "$HANDOFF_COUNT" "$LAST_HANDOFF_TS" "$LAST_RESULT" "$LAST_SESSION" "$HISTORY"
        fi
        continue
    fi

    CURRENT_TOKENS=$(echo "$USAGE" | awk '{print $1}')
    MAX_TOKENS=$(echo "$USAGE" | awk '{print $2}')
    PERCENT=$(echo "$USAGE" | awk '{print $3}')
    MODEL=$(echo "$USAGE" | awk '{print $4}')

    # Update state with current readings
    write_state "monitoring" "$CURRENT_TOKENS" "$MAX_TOKENS" "$PERCENT" "true" "$MODEL" "$HANDOFF_COUNT" "$LAST_HANDOFF_TS" "$LAST_RESULT" "$LAST_SESSION" "$HISTORY"

    # Check threshold
    PERCENT_INT=$(echo "$PERCENT" | awk '{printf "%d", $1}')

    if [ "$PERCENT_INT" -ge "$THRESHOLD" ]; then
        # Cooldown check
        NOW=$(date +%s)
        ELAPSED=$((NOW - LAST_HANDOFF))

        if [ "$ELAPSED" -lt "$COOLDOWN" ]; then
            continue
        fi

        LAST_HANDOFF=$NOW
        do_handoff "$CURRENT_TOKENS" "$PERCENT" "$MODEL" "$HANDOFF_COUNT" "$HISTORY"
    fi
done
