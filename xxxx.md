# Instructions: Automated Roadmap Pipeline Setup

You are an autonomous engineering agent designed to execute a prioritized roadmap over long-running sessions. Because the context window is finite, you must maintain your own state in physical files so that your memory can be safely wiped (`/clear`) without losing progress.

Follow these instructions to set up and maintain this pipeline.

---

## Step 1: Create the Project Management Files

Create these three files in the root directory of the workspace immediately.

### 1. `ROADMAP.md`
This file contains the high-level backlog. Tasks must be strictly linear and listed in priority order. Copy and paste this template:

```markdown
# Feature Roadmap

## Backlog (Priority Order)
- [ ] Task 1: [Describe the first foundational task]
- [ ] Task 2: [Describe the next logical step]
- [ ] Task 3: [Describe the subsequent feature]
- [ ] Task 4: [Describe the final verification/testing task]

## Completed Tasks
<!-- Move checked-off tasks here to keep the active backlog clean -->
```

### 2. `CURRENT_TASK.md`
This file acts as your active "RAM". It tracks what you are doing *right now*. Copy and paste this template:

```markdown
# Current Active Task
[State the exact task from the roadmap you are working on]

## Execution Plan & Micro-Steps
- [ ] Step 1: [Immediate technical action]
- [ ] Step 2: [Next logical micro-action]

## Active Codebase Context
* **Files Modified:** None
* **Current Blockers/Errors:** None

## Next Immediate Action
[The exact next step you will take if you are suddenly interrupted/cleared]
```

### 3. `CLAUDE.md`
This is your global system instruction file. Claude Code reads this file automatically. Create or append these exact rules to it:

```markdown
# Pipeline Execution Rules

1. **File-Driven Memory**: You must treat `ROADMAP.md` and `CURRENT_TASK.md` as your source of truth. Your conversation history is temporary and will be cleared frequently.
2. **Strict Focus**: Work *only* on the highest priority, unchecked item in `ROADMAP.md`. Do not multi-task.
3. **Continuous State Commits**: Every time you modify a file, run a build, or encounter an error, immediately update `CURRENT_TASK.md` with the latest status and your "Next Immediate Action".
4. **Handoff Readiness**: Assume you could be wiped via `/clear` at any second. Ensure `CURRENT_TASK.md` is always accurate enough for a fresh instance of you to read it and resume seamlessly.
5. **Task Completion**: When a task is done, check it off, move it to the "Completed Tasks" section in `ROADMAP.md`, wipe `CURRENT_TASK.md` clean, and initialize it with the next task.
```

---

## Step 2: Initialize the System

Once these files are created:
1. Populate `ROADMAP.md` with the comprehensive, prioritized list of tasks required to build the feature.
2. Fill out `CURRENT_TASK.md` for the very first item.
3. Reply to the user confirming that the tracking system is active and state your first action.




## watcher script template (requires editing for local environment - requires check of the llama.cpp context)

```Bash
#!/bin/bash

# Configuration settings (Customized for llama.cpp on port 8989)
LOG_PATH="C:/path/to/your/llama_cpp_metrics.log"
FIFO_PIPE="/tmp/claude_input_pipe"

echo -e "\033[36m[Watcher] Initializing clean state pipeline files...\033[0m"
rm -f "$FIFO_PIPE"
mkfifo "$FIFO_PIPE"

# 1. Boot Claude Code as a background job fed by our tracking pipe
# 'sleep infinity' ensures the input channel stays persistently open
(cat "$FIFO_PIPE"; sleep infinity) | claude > /dev/null 2>&1 &
CLAUDE_PID=$!

echo -e "\033[32m[Watcher] Claude Code running in background (PID: $CLAUDE_PID).\033[0m"
echo -e "\033[32m[Watcher] Actively listening to context usage threshold metrics...\033[0m"

while true; do
    if [ -f "$LOG_PATH" ]; then
        # Fetch the latest single metrics entry from your llama.cpp log file
        LAST_LINE=$(tail -n 1 "$LOG_PATH")
        
        # Match pattern strings such as "context_utilization: 0.91"
        if [[ "$LAST_LINE" =~ context_utilization:[[:space:]]*([0-9.]+) ]]; then
            USAGE="${BASH_REMATCH[1]}"
            
            # 2. Check if context window has surpassed the 90% threshold ceiling
            if (( $(echo "$USAGE >= 0.90" | bc -l) )); then
                PERCENT=$(echo "$USAGE * 100" | bc | cut -d'.' -f1)
                echo -e "\033[33m[Watcher] Warning: Context at ${PERCENT}%! Triggering state save sequence...\033[0m"
                
                # 3. Prompt Claude to safely write its mental stack out to CURRENT_TASK.md
                echo "System: The context window limit is full. Please immediately finalize your current step and update CURRENT_TASK.md." > "$FIFO_PIPE"
                
                # Give Claude 15 seconds to safely invoke its local tools and modify markdown files
                sleep 15
                
                # 4. Flush Claude Code's internal short-term chat timeline memory to 0 tokens.
                # Because the message history array drops to 0, llama.cpp automatically 
                # flushes its own VRAM cache on the very next incoming API call!
                echo -e "\033[34m[Watcher] Issuing /clear context purge to Claude Code...\033[0m"
                echo "/clear" > "$FIFO_PIPE"
                sleep 2
                
                # 5. Point Claude back to your persistent repository layout files to resume
                echo "Please re-read ROADMAP.md and CURRENT_TASK.md, then execute the next task." > "$FIFO_PIPE"
                echo -e "\033[32m[Watcher] Reset sequence absolute. Execution loop resumed cleanly.\033[0m"
            fi
        fi
    fi
    # Check metric logs every 5 seconds
    sleep 5
done

```
