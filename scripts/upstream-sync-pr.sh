#!/usr/bin/env bash
set -euo pipefail

# upstream-sync-pr.sh — Generate upstream sync reports for automated PR creation
#
# Reads tracking state from .upstream-tracking.json. Outputs plain markdown
# (no ANSI colors) suitable for GitHub PR bodies and sync-log entries.
#
# Usage:
#   ./scripts/upstream-sync-pr.sh --check            # Exit 0 if drift exists, 1 if none
#   ./scripts/upstream-sync-pr.sh --report           # PR body markdown to stdout
#   ./scripts/upstream-sync-pr.sh --sync-log-entry   # Sync-log skeleton to stdout
#   ./scripts/upstream-sync-pr.sh --update-tracking   # Write new HEADs to tracking file

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TRACKING_FILE="${REPO_ROOT}/.upstream-tracking.json"

DATE=$(date +%Y-%m-%d)

# --- Tracking file helpers ---

# shellcheck source=lib/tracking.sh
source "${SCRIPT_DIR}/lib/tracking.sh"

# --- Drift detection ---

has_drift_for() {
    local key="$1"
    local remote branch last_reviewed remote_head
    remote=$(read_remote "$key")
    branch=$(read_branch "$key")
    last_reviewed=$(read_tracking "$key")
    [[ -z "$last_reviewed" ]] && return 1
    remote_head=$(git rev-parse "${remote}/${branch}" 2>/dev/null) || return 1
    [[ "$last_reviewed" != "$remote_head" ]]
}

has_any_drift() {
    has_drift_for "beechat" || has_drift_for "upstream"
}

# --- Report helpers ---

commit_count() {
    local from="$1" to="$2"
    git rev-list --count "${from}..${to}" 2>/dev/null || echo 0
}

commit_table_rows() {
    local from="$1" to="$2"
    # Escape pipe characters in commit subjects to avoid breaking markdown tables
    git log --format="%h%x09%s%x09%an" "${from}..${to}" 2>/dev/null | while IFS=$'\t' read -r hash subject author; do
        subject="${subject//|/\|}"
        author="${author//|/\|}"
        echo "| \`${hash}\` | ${subject} | ${author} | | |"
    done
}

file_stats() {
    local from="$1" to="$2"
    git diff --stat "${from}..${to}" 2>/dev/null | tail -30
}

unmerged_branch_rows() {
    local remote="$1" branch="$2"
    local branches
    branches=$(git branch -r 2>/dev/null | grep "^ *${remote}/" | grep -v HEAD | grep -v "${remote}/${branch}$" || true)
    [[ -z "$branches" ]] && return
    while IFS= read -r b; do
        b=$(echo "$b" | xargs)
        local ahead
        ahead=$(git rev-list --count "${remote}/${branch}..${b}" 2>/dev/null || echo 0)
        if [[ "$ahead" -gt 0 ]]; then
            local short="${b#${remote}/}"
            echo "| \`${short}\` | +${ahead} | | |"
        fi
    done <<< "$branches"
}

# --- Report: PR body ---

generate_report_section() {
    local key="$1" display_name="$2"
    local remote branch last_reviewed remote_head
    remote=$(read_remote "$key")
    branch=$(read_branch "$key")
    last_reviewed=$(read_tracking "$key")
    remote_head=$(git rev-parse "${remote}/${branch}" 2>/dev/null || echo "")

    echo "## ${display_name}"
    echo ""

    if [[ -z "$remote_head" ]]; then
        echo "Could not resolve \`${remote}/${branch}\`. Remote may not be fetched."
        echo ""
        return
    fi

    if [[ "$last_reviewed" == "$remote_head" ]]; then
        echo "No new commits since last review."
        echo ""
        return
    fi

    local count
    count=$(commit_count "$last_reviewed" "${remote}/${branch}")

    echo "**${count} new commit(s)** since last review"
    echo ""
    echo "- Last reviewed: \`${last_reviewed:0:8}\` — $(git log --format='%s' -1 "$last_reviewed" 2>/dev/null || echo 'unknown')"
    echo "- Remote HEAD: \`${remote_head:0:8}\` — $(git log --format='%s' -1 "$remote_head" 2>/dev/null || echo 'unknown')"
    echo ""

    echo "### Commits to Triage"
    echo ""
    echo "| Commit | Description | Author | Decision | Notes |"
    echo "|--------|-------------|--------|----------|-------|"
    commit_table_rows "$last_reviewed" "${remote}/${branch}"
    echo ""

    echo "<details>"
    echo "<summary>Files changed</summary>"
    echo ""
    echo '```'
    file_stats "$last_reviewed" "${remote}/${branch}"
    echo '```'
    echo "</details>"
    echo ""

    local ub
    ub=$(unmerged_branch_rows "$remote" "$branch")
    if [[ -n "$ub" ]]; then
        echo "### Unmerged Branches"
        echo ""
        echo "| Branch | Ahead | Decision | Notes |"
        echo "|--------|-------|----------|-------|"
        echo "$ub"
        echo ""
    fi
}

generate_report() {
    echo "Automated weekly review of upstream changes pending triage."
    echo "See [UPSTREAM.md](UPSTREAM.md) for path mappings and sync strategy."
    echo ""

    generate_report_section "beechat" "Beechat / Reticulum-rs"
    generate_report_section "upstream" "FreeTAKTeam / LXMF-rs"

    echo "---"
    echo ""
    echo "## Reviewer Checklist"
    echo ""
    echo "- [ ] Triage each commit: fill in **Decision** column (\`adopt\` / \`skip\` / \`defer\`)"
    echo "- [ ] For adopted changes: create port commits on this branch (cite upstream SHA)"
    echo "- [ ] Review unmerged branches for anything worth tracking"
    echo "- [ ] Merge this PR (advances tracking markers automatically)"
    echo ""
    echo "The \`.upstream-tracking.json\` in this PR is already updated to the"
    echo "current upstream HEADs. Merging records that these commits have been reviewed."
}

# --- Report: sync-log entry ---

generate_sync_log_section() {
    local key="$1" display_name="$2"
    local remote branch last_reviewed remote_head
    remote=$(read_remote "$key")
    branch=$(read_branch "$key")
    last_reviewed=$(read_tracking "$key")
    remote_head=$(git rev-parse "${remote}/${branch}" 2>/dev/null || echo "")

    echo "### ${display_name}"
    echo ""

    if [[ -z "$remote_head" || "$last_reviewed" == "$remote_head" ]]; then
        echo "No new commits since last review."
        echo ""
        return
    fi

    echo "**Range:** \`${last_reviewed:0:8}\` → \`${remote_head:0:8}\`"
    echo ""
    echo "| Commit | Description | Author | Decision | Notes |"
    echo "|--------|-------------|--------|----------|-------|"
    commit_table_rows "$last_reviewed" "${remote}/${branch}"
    echo ""

    local ub
    ub=$(unmerged_branch_rows "$remote" "$branch")
    if [[ -n "$ub" ]]; then
        echo "**Unmerged feature branches of interest:**"
        echo ""
        echo "| Branch | Commits | Decision | Notes |"
        echo "|--------|---------|----------|-------|"
        echo "$ub"
        echo ""
    fi
}

generate_sync_log_entry() {
    echo "## ${DATE} — Automated Review"
    echo ""
    echo "**Reviewer:** (pending)"
    echo ""
    generate_sync_log_section "beechat" "Beechat / Reticulum-rs"
    generate_sync_log_section "upstream" "FreeTAKTeam / LXMF-rs"
}

# --- Update tracking ---

update_tracking() {
    for key in beechat upstream; do
        local remote branch remote_head
        remote=$(read_remote "$key")
        branch=$(read_branch "$key")
        remote_head=$(git rev-parse "${remote}/${branch}" 2>/dev/null || true)
        if [[ -n "$remote_head" ]]; then
            write_tracking "$key" "$remote_head"
        fi
    done
}

# --- Main ---

if [[ ! -f "$TRACKING_FILE" ]]; then
    echo "Error: tracking file not found: $TRACKING_FILE" >&2
    exit 1
fi

case "${1:---help}" in
    --check)
        has_any_drift
        ;;
    --report)
        generate_report
        ;;
    --sync-log-entry)
        generate_sync_log_entry
        ;;
    --update-tracking)
        update_tracking
        ;;
    -h|--help)
        echo "Usage: $0 [--check|--report|--sync-log-entry|--update-tracking]"
        echo ""
        echo "  --check            Exit 0 if upstream drift exists, 1 if none"
        echo "  --report           Generate PR body (markdown to stdout)"
        echo "  --sync-log-entry   Generate sync-log skeleton (markdown to stdout)"
        echo "  --update-tracking  Write current upstream HEADs to tracking file"
        echo ""
        echo "Tracking state: $TRACKING_FILE"
        ;;
    *)
        echo "Unknown argument: $1" >&2
        exit 1
        ;;
esac
