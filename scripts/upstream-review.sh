#!/usr/bin/env bash
set -euo pipefail

# upstream-review.sh — Review pending upstream changes for styrene-rs
#
# Reads tracking state from .upstream-tracking.json (committed to repo).
# This file is the source of truth for CI and local workflows.
#
# Usage:
#   ./scripts/upstream-review.sh              # Review both upstreams
#   ./scripts/upstream-review.sh beechat      # Review Beechat only
#   ./scripts/upstream-review.sh upstream     # Review FreeTAKTeam only
#   ./scripts/upstream-review.sh --advance    # Advance tracking to current upstream HEADs
#   ./scripts/upstream-review.sh --status     # Show current tracking state

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
TRACKING_FILE="${REPO_ROOT}/.upstream-tracking.json"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
RESET='\033[0m'

header() { echo -e "\n${BOLD}${CYAN}=== $1 ===${RESET}\n"; }
warn()   { echo -e "${YELLOW}$1${RESET}"; }
info()   { echo -e "${GREEN}$1${RESET}"; }
err()    { echo -e "${RED}$1${RESET}" >&2; }

# --- Tracking file helpers ---

# shellcheck source=lib/tracking.sh
source "${SCRIPT_DIR}/lib/tracking.sh"

ensure_tracking_file() {
    if [[ ! -f "$TRACKING_FILE" ]]; then
        err "Tracking file not found: $TRACKING_FILE"
        err "Initialize it with: scripts/upstream-review.sh --init"
        exit 1
    fi
}

# --- Remote management ---

ensure_remotes() {
    for key in beechat upstream; do
        local remote branch
        remote=$(read_remote "$key")
        branch=$(read_branch "$key")
        if [[ -z "$remote" ]]; then
            continue
        fi
        if ! git remote | grep -q "^${remote}$"; then
            case "$key" in
                beechat)
                    echo "Adding beechat remote..."
                    git remote add "$remote" https://github.com/BeechatNetworkSystemsLtd/Reticulum-rs.git
                    ;;
                upstream)
                    echo "Adding upstream remote..."
                    git remote add "$remote" https://github.com/FreeTAKTeam/LXMF-rs.git
                    ;;
            esac
            git remote set-url --push "$remote" DISABLE
        fi
    done
}

fetch_remotes() {
    local target="${1:-all}"
    for key in beechat upstream; do
        if [[ "$target" != "all" && "$target" != "$key" ]]; then
            continue
        fi
        local remote
        remote=$(read_remote "$key")
        if [[ -n "$remote" ]]; then
            echo "Fetching ${key}..."
            git fetch "$remote" 2>&1 | grep -v "^$" || true
        fi
    done
}

# --- Review ---

review_remote() {
    local key="$1"
    local remote branch last_reviewed
    remote=$(read_remote "$key")
    branch=$(read_branch "$key")
    last_reviewed=$(read_tracking "$key")

    local display_name
    case "$key" in
        beechat)  display_name="Beechat / Reticulum-rs" ;;
        upstream) display_name="FreeTAKTeam / LXMF-rs" ;;
        *)        display_name="$key" ;;
    esac

    header "$display_name (${remote}/${branch})"

    if [[ -z "$last_reviewed" ]]; then
        warn "No last_reviewed SHA in tracking file for '$key'."
        warn "Set it in $TRACKING_FILE"
        return
    fi

    local remote_head
    remote_head=$(git rev-parse "${remote}/${branch}" 2>/dev/null || true)
    if [[ -z "$remote_head" ]]; then
        err "Could not resolve ${remote}/${branch}. Did you fetch?"
        return
    fi

    if [[ "$last_reviewed" == "$remote_head" ]]; then
        info "Up to date. No new commits since last review."
        echo "  Last reviewed: $(git log --oneline -1 "$last_reviewed")"
        return
    fi

    local count
    count=$(git rev-list --count "${last_reviewed}..${remote}/${branch}")
    warn "$count new commit(s) since last review"
    echo ""

    echo -e "${BOLD}Last reviewed:${RESET} $(git log --oneline -1 "$last_reviewed")"
    echo -e "${BOLD}Remote HEAD:${RESET}   $(git log --oneline -1 "${remote}/${branch}")"
    echo ""

    echo -e "${BOLD}Commits to review:${RESET}"
    git log --oneline --format="  %h %s [%an]" "${last_reviewed}..${remote}/${branch}"
    echo ""

    echo -e "${BOLD}Files changed:${RESET}"
    git diff --stat "${last_reviewed}..${remote}/${branch}" | tail -20
    echo ""

    # Show feature branches not yet on main
    echo -e "${BOLD}Unmerged feature branches:${RESET}"
    local branches
    branches=$(git branch -r 2>/dev/null | grep "^ *${remote}/" | grep -v HEAD | grep -v "${remote}/${branch}$" || true)
    if [[ -n "$branches" ]]; then
        local found=false
        while IFS= read -r b; do
            b=$(echo "$b" | xargs)
            local ahead
            ahead=$(git rev-list --count "${remote}/${branch}..${b}" 2>/dev/null || echo 0)
            if [[ "$ahead" -gt 0 ]]; then
                echo "  $b (+${ahead} commits)"
                found=true
            fi
        done <<< "$branches"
        if ! $found; then
            echo "  (none)"
        fi
    else
        echo "  (none)"
    fi
}

# --- Advance ---

advance_tags() {
    header "Advancing tracking markers"

    local target="${1:-all}"

    for key in beechat upstream; do
        if [[ "$target" != "all" && "$target" != "$key" ]]; then
            continue
        fi
        local remote branch
        remote=$(read_remote "$key")
        branch=$(read_branch "$key")
        local remote_head
        remote_head=$(git rev-parse "${remote}/${branch}" 2>/dev/null || true)
        if [[ -z "$remote_head" ]]; then
            err "Could not resolve ${remote}/${branch}. Did you fetch?"
            continue
        fi
        write_tracking "$key" "$remote_head"
        info "Advanced $key to $(git log --oneline -1 "$remote_head")"
    done

    echo ""
    warn "Tracking file updated: $TRACKING_FILE"
    warn "Remember to commit this change."
}

# --- Status ---

show_status() {
    header "Upstream Tracking Status"

    echo -e "${BOLD}Tracking file:${RESET} $TRACKING_FILE"
    echo ""

    for key in beechat upstream; do
        local remote branch last_reviewed
        remote=$(read_remote "$key")
        branch=$(read_branch "$key")
        last_reviewed=$(read_tracking "$key")
        if [[ -n "$last_reviewed" ]]; then
            echo -e "${BOLD}$key:${RESET} ${last_reviewed:0:8} $(git log --format='%s' -1 "$last_reviewed" 2>/dev/null || echo '(commit not fetched)')"
        else
            warn "$key: not configured"
        fi
    done
    echo ""
    echo "Remotes:"
    git remote -v | grep -E "^(beechat|upstream|origin)" | sort
}

# --- Main ---

ensure_tracking_file
ensure_remotes

case "${1:-all}" in
    --advance)
        fetch_remotes "${2:-all}"
        advance_tags "${2:-all}"
        ;;
    --status)
        show_status
        ;;
    --init)
        header "Initializing tracking file"
        warn "Tracking file already exists at $TRACKING_FILE"
        warn "Edit it manually or use --advance to set to current upstream HEADs."
        ;;
    beechat)
        fetch_remotes beechat
        review_remote beechat
        ;;
    upstream)
        fetch_remotes upstream
        review_remote upstream
        ;;
    all)
        fetch_remotes all
        review_remote beechat
        review_remote upstream
        ;;
    -h|--help)
        echo "Usage: $0 [beechat|upstream|all|--advance|--status]"
        echo ""
        echo "  all (default)   Review pending changes from both upstreams"
        echo "  beechat         Review Beechat/Reticulum-rs only"
        echo "  upstream        Review FreeTAKTeam/LXMF-rs only"
        echo "  --advance       Mark current upstream HEADs as reviewed"
        echo "  --status        Show current tracking state"
        echo ""
        echo "Tracking state: $TRACKING_FILE"
        ;;
    *)
        echo "Unknown argument: $1" >&2
        echo "Run '$0 --help' for usage." >&2
        exit 1
        ;;
esac
