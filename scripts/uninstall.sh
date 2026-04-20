#!/usr/bin/env bash
# knotch uninstaller — see `./uninstall.sh --help` for full usage.
set -euo pipefail

BINARY_NAME="knotch"
PLUGIN_NAME="knotch"

INSTALL_DIR="${KNOTCH_INSTALL_DIR:-$HOME/.local/bin}"
KNOTCH_KEEP_PLUGIN="${KNOTCH_KEEP_PLUGIN:-0}"
KNOTCH_KEEP_BACKUP="${KNOTCH_KEEP_BACKUP:-0}"
KNOTCH_YES="${KNOTCH_YES:-0}"

INPUT_FD=""
C_RESET=""; C_DIM=""; C_RED=""; C_GREEN=""; C_YELLOW=""; C_BLUE=""; C_BOLD=""

die()      { printf '%s✗ %s%s\n' "$C_RED" "$*" "$C_RESET" >&2; exit 1; }
log_info() { printf '%s  %s%s\n' "$C_DIM" "$*" "$C_RESET"; }
log_warn() { printf '%s!  %s%s\n' "$C_YELLOW" "$*" "$C_RESET"; }
log_ok()   { printf '%s✓  %s%s\n' "$C_GREEN" "$*" "$C_RESET"; }
render_step() { printf '%s▸  %s%s\n' "$C_BLUE" "$*" "$C_RESET"; }

init_colors() {
    if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-}" != "dumb" ]; then
        C_RESET=$'\033[0m'; C_DIM=$'\033[2m'
        C_RED=$'\033[31m'; C_GREEN=$'\033[32m'
        C_YELLOW=$'\033[33m'; C_BLUE=$'\033[34m'; C_BOLD=$'\033[1m'
    fi
}

detect_tty() {
    if [ "$KNOTCH_YES" = "1" ]; then INPUT_FD=""; return 1; fi
    if [ -t 0 ]; then INPUT_FD="0"; return 0; fi
    if [ -e /dev/tty ] && [ -r /dev/tty ]; then INPUT_FD="/dev/tty"; return 0; fi
    INPUT_FD=""; return 1
}

read_line() {
    local answer
    if [ "$INPUT_FD" = "0" ]; then IFS= read -r answer || answer=""
    else IFS= read -r answer < /dev/tty || answer=""
    fi
    printf '%s' "$answer"
}

prompt_yesno() {
    local question="$1" default="$2" answer
    if [ -z "$INPUT_FD" ]; then
        [ "$default" = "Y" ] && return 0 || return 1
    fi
    local hint; [ "$default" = "Y" ] && hint="[Y/n]" || hint="[y/N]"
    printf '%s%s%s %s ' "$C_BOLD" "$question" "$C_RESET" "$hint" >&2
    answer="$(read_line)"
    answer="${answer:-$default}"
    case "$answer" in [Yy]*) return 0 ;; *) return 1 ;; esac
}

backup_path() {
    local target="$1"
    [ -e "$target" ] || return 0
    local backup="${target}.backup_$(date +%Y%m%d_%H%M%S)_$$"
    cp -r "$target" "$backup"
    log_info "Backup: $backup"
}

uninstall_binary() {
    local dest="${INSTALL_DIR}/${BINARY_NAME}"
    render_step "Removing binary"
    if [ -f "$dest" ]; then
        rm -f "$dest"
        log_ok "Removed $dest"
    else
        log_info "Binary not found at $dest"
    fi
}

uninstall_plugin() {
    local target="$HOME/.claude/plugins/${PLUGIN_NAME}"
    if [ ! -d "$target" ]; then
        log_info "No user-level plugin at $target"
        return
    fi
    if [ "$KNOTCH_KEEP_PLUGIN" = "1" ]; then
        log_info "Keeping plugin (--keep-plugin)"
        return
    fi
    # --yes means non-interactive full cleanup. Only interactive runs are
    # asked (with a conservative default of N) since plugin bundles can
    # outlive the binary when shared across projects.
    if [ "$KNOTCH_YES" != "1" ] && ! prompt_yesno "Remove plugin at $target?" "N"; then
        log_info "Plugin kept"; return
    fi
    render_step "Removing plugin"
    [ "$KNOTCH_KEEP_BACKUP" = "1" ] || backup_path "$target"
    rm -rf "$target"
    log_ok "Removed $target"

    local parent="$HOME/.claude/plugins"
    if [ -d "$parent" ] && [ -z "$(ls -A "$parent")" ]; then
        rmdir "$parent"
        log_info "Cleaned empty $parent"
    fi
}

parse_args() {
    while [ $# -gt 0 ]; do
        case "$1" in
            --install-dir)  INSTALL_DIR="$2"; shift 2 ;;
            --keep-plugin)  KNOTCH_KEEP_PLUGIN=1; shift ;;
            --keep-backup)  KNOTCH_KEEP_BACKUP=1; shift ;;
            --yes|-y)       KNOTCH_YES=1; shift ;;
            --help|-h)      print_usage; exit 0 ;;
            *)              die "Unknown flag: $1" ;;
        esac
    done
}

print_usage() {
    cat <<'USAGE'
knotch uninstaller

Usage:
  curl -fsSL https://raw.githubusercontent.com/junyeong-ai/knotch/main/scripts/uninstall.sh | bash
  ./scripts/uninstall.sh [flags]

Flags:
  --install-dir PATH   Binary directory (default: $HOME/.local/bin)
  --keep-plugin        Do not remove user-level plugin bundle
  --keep-backup        Do not back up plugin before removing
  --yes, -y            Non-interactive full cleanup (binary + plugin)
  --help, -h           Show this message

Environment variables (flags win over env, env wins over defaults):
  KNOTCH_INSTALL_DIR, KNOTCH_KEEP_PLUGIN, KNOTCH_KEEP_BACKUP,
  KNOTCH_YES, NO_COLOR
USAGE
}

main() {
    init_colors
    parse_args "$@"
    detect_tty || true

    printf '\n%sknotch uninstaller%s\n\n' "$C_BOLD" "$C_RESET"
    uninstall_binary
    uninstall_plugin
    printf '\n%s✅ Uninstall complete%s\n' "$C_GREEN$C_BOLD" "$C_RESET"
    log_info "Project-level plugins (./.claude/plugins/${PLUGIN_NAME}/) are managed by the project's git tree"
}

main "$@"
