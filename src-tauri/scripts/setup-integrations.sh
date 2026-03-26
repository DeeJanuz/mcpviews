#!/usr/bin/env bash
set -euo pipefail

# MCP Mux — Agent Integration Setup
# Sets up MCP Mux as an MCP server in supported AI agent platforms.

MCP_MUX_URL="http://localhost:4200/mcp"
SENTINEL_DIR="$HOME/.mcp-mux"
SENTINEL_FILE="$SENTINEL_DIR/.setup-complete"

# ─── Platform definitions ────────────────────────────────────────────────────

declare -a PLATFORM_NAMES=(
  "Claude Desktop"
  "Claude Code CLI"
  "Cursor IDE"
  "Windsurf"
  "Codex CLI"
  "OpenCode"
  "Antigravity"
)

detect_os() {
  case "$(uname -s)" in
    Darwin) echo "macos" ;;
    *)      echo "linux" ;;
  esac
}

OS="$(detect_os)"

config_path_for() {
  local idx="$1"
  case "$idx" in
    0) # Claude Desktop
      if [[ "$OS" == "macos" ]]; then
        echo "$HOME/Library/Application Support/Claude/claude_desktop_config.json"
      else
        echo "$HOME/.config/Claude/claude_desktop_config.json"
      fi
      ;;
    1) # Claude Code CLI
      echo "$HOME/.claude.json"
      ;;
    2) # Cursor IDE
      echo "$HOME/.cursor/mcp.json"
      ;;
    3) # Windsurf
      echo "$HOME/.codeium/windsurf/mcp_config.json"
      ;;
    4) # Codex CLI
      echo "$HOME/.codex/config.toml"
      ;;
    5) # OpenCode
      echo "$HOME/.config/opencode/opencode.json"
      ;;
    6) # Antigravity
      echo "$HOME/.gemini/antigravity/mcp_config.json"
      ;;
  esac
}

# Returns 0 if the platform is detected on the system (installed/present)
platform_detected() {
  local idx="$1"
  case "$idx" in
    0) # Claude Desktop — config dir exists
      if [[ "$OS" == "macos" ]]; then
        [[ -d "$HOME/Library/Application Support/Claude" ]]
      else
        [[ -d "$HOME/.config/Claude" ]]
      fi
      ;;
    1) # Claude Code CLI
      command -v claude &>/dev/null || [[ -f "$HOME/.claude.json" ]]
      ;;
    2) # Cursor IDE
      [[ -d "$HOME/.cursor" ]]
      ;;
    3) # Windsurf
      [[ -d "$HOME/.codeium/windsurf" ]]
      ;;
    4) # Codex CLI
      command -v codex &>/dev/null || [[ -d "$HOME/.codex" ]]
      ;;
    5) # OpenCode
      [[ -d "$HOME/.config/opencode" ]]
      ;;
    6) # Antigravity
      [[ -d "$HOME/.gemini/antigravity" ]]
      ;;
    *) return 1 ;;
  esac
}

# Returns the JSON key used for MCP servers (or "toml" for Codex)
mcp_key_for() {
  local idx="$1"
  case "$idx" in
    4) echo "toml" ;;
    5) echo "mcp" ;;
    *) echo "mcpServers" ;;
  esac
}

# Returns 0 if mcp-mux is already configured for the given platform
already_configured() {
  local idx="$1"
  local cfg
  cfg="$(config_path_for "$idx")"

  [[ -f "$cfg" ]] || return 1

  if [[ "$idx" -eq 4 ]]; then
    # Codex TOML
    grep -q '\[mcp_servers\.mcp-mux\]' "$cfg" 2>/dev/null
  else
    # JSON platforms — look for "mcp-mux" key
    grep -q '"mcp-mux"' "$cfg" 2>/dev/null
  fi
}

# ─── JSON manipulation ────────────────────────────────────────────────────────

HAS_PYTHON=false
if command -v python3 &>/dev/null; then
  HAS_PYTHON=true
fi

# Merge mcp-mux entry into a JSON config file using python3.
# $1 = file path, $2 = top-level key (mcpServers or mcp)
merge_json_python() {
  local cfg="$1" key="$2"
  python3 -c "
import json, os, sys

cfg_path = sys.argv[1]
key = sys.argv[2]
url = sys.argv[3]

data = {}
if os.path.isfile(cfg_path) and os.path.getsize(cfg_path) > 0:
    try:
        with open(cfg_path, 'r') as f:
            data = json.load(f)
    except (json.JSONDecodeError, ValueError):
        # Corrupted file — start fresh but back up
        pass

if not isinstance(data, dict):
    data = {}

if key not in data or not isinstance(data[key], dict):
    data[key] = {}

data[key]['mcp-mux'] = {'url': url}

with open(cfg_path, 'w') as f:
    json.dump(data, f, indent=2)
    f.write('\n')
" "$cfg" "$key" "$MCP_MUX_URL"
}

# Write a fresh JSON config with just the mcp-mux entry (bash fallback for new files).
# $1 = file path, $2 = top-level key
write_fresh_json() {
  local cfg="$1" key="$2"
  cat > "$cfg" <<ENDJSON
{
  "$key": {
    "mcp-mux": {
      "url": "$MCP_MUX_URL"
    }
  }
}
ENDJSON
}

# Bash-only merge: back up, then attempt a simple insertion.
# Falls back to writing a fresh file if the existing file is unparseable.
# $1 = file path, $2 = top-level key
merge_json_bash() {
  local cfg="$1" key="$2"

  # If file doesn't exist or is empty, write fresh
  if [[ ! -s "$cfg" ]]; then
    write_fresh_json "$cfg" "$key"
    return 0
  fi

  # Back up
  cp "$cfg" "${cfg}.bak"

  # Check if the top-level key already exists
  if grep -q "\"$key\"" "$cfg"; then
    # Key exists — insert mcp-mux entry after the opening brace of the key's object.
    # Strategy: find the line with "$key" and the next '{', then insert after it.
    local tmp
    tmp="$(mktemp)"
    local inserted=false
    local found_key=false
    while IFS= read -r line || [[ -n "$line" ]]; do
      echo "$line" >> "$tmp"
      if [[ "$found_key" == false ]] && echo "$line" | grep -q "\"$key\""; then
        found_key=true
        # If the opening brace is on the same line (e.g., "mcpServers": {), insert after
        if echo "$line" | grep -q '{'; then
          # Insert the entry
          cat >> "$tmp" <<ENTRY
    "mcp-mux": {
      "url": "$MCP_MUX_URL"
    },
ENTRY
          inserted=true
        fi
        continue
      fi
      if [[ "$found_key" == true ]] && [[ "$inserted" == false ]]; then
        # Look for the opening brace on a subsequent line
        if echo "$line" | grep -q '{'; then
          cat >> "$tmp" <<ENTRY
    "mcp-mux": {
      "url": "$MCP_MUX_URL"
    },
ENTRY
          inserted=true
        fi
      fi
    done < "$cfg"

    if [[ "$inserted" == true ]]; then
      mv "$tmp" "$cfg"
    else
      rm -f "$tmp"
      # Could not insert — overwrite with fresh (preserving backup)
      write_fresh_json "$cfg" "$key"
    fi
  else
    # Key does not exist — we need to add it.
    # Simple approach: if file has a top-level object, insert before the last '}'
    local tmp
    tmp="$(mktemp)"
    local last_brace_line
    last_brace_line="$(grep -n '}' "$cfg" | tail -1 | cut -d: -f1)"

    if [[ -n "$last_brace_line" ]]; then
      local line_num=0
      while IFS= read -r line || [[ -n "$line" ]]; do
        line_num=$((line_num + 1))
        if [[ "$line_num" -eq "$last_brace_line" ]]; then
          # Need a comma on the previous content if there was any
          # Insert the new key before the closing brace
          cat >> "$tmp" <<ENTRY
  ,"$key": {
    "mcp-mux": {
      "url": "$MCP_MUX_URL"
    }
  }
ENTRY
        fi
        echo "$line" >> "$tmp"
      done < "$cfg"
      mv "$tmp" "$cfg"
    else
      rm -f "$tmp"
      write_fresh_json "$cfg" "$key"
    fi
  fi
}

# High-level: configure a JSON-based platform
configure_json() {
  local idx="$1"
  local cfg key
  cfg="$(config_path_for "$idx")"
  key="$(mcp_key_for "$idx")"

  # Ensure parent directory exists
  mkdir -p "$(dirname "$cfg")"

  # Back up existing file
  if [[ -f "$cfg" ]] && [[ -s "$cfg" ]]; then
    cp "$cfg" "${cfg}.bak"
  fi

  if [[ "$HAS_PYTHON" == true ]]; then
    merge_json_python "$cfg" "$key"
  else
    merge_json_bash "$cfg" "$key"
  fi
}

# Configure Codex CLI (TOML)
configure_codex() {
  local cfg
  cfg="$(config_path_for 4)"

  mkdir -p "$(dirname "$cfg")"

  if [[ -f "$cfg" ]] && [[ -s "$cfg" ]]; then
    cp "$cfg" "${cfg}.bak"
  fi

  # Check if already present
  if [[ -f "$cfg" ]] && grep -q '\[mcp_servers\.mcp-mux\]' "$cfg" 2>/dev/null; then
    return 0
  fi

  # Append TOML section
  {
    # Add a blank line separator if the file is non-empty
    if [[ -f "$cfg" ]] && [[ -s "$cfg" ]]; then
      echo ""
    fi
    cat <<'ENDTOML'
[mcp_servers.mcp-mux]
type = "sse"
url = "http://localhost:4200/mcp"
ENDTOML
  } >> "$cfg"
}

# ─── Configure a single platform by index ─────────────────────────────────────

configure_platform() {
  local idx="$1"
  if [[ "$idx" -eq 4 ]]; then
    configure_codex
  else
    configure_json "$idx"
  fi
}

# ─── Main UI ──────────────────────────────────────────────────────────────────

main() {
  echo ""
  echo "MCP Mux — Agent Integration Setup"
  echo "==================================="
  echo ""

  # Discover which platforms are present
  declare -a detected_indices=()
  declare -a detected_status=()

  for i in "${!PLATFORM_NAMES[@]}"; do
    if platform_detected "$i"; then
      detected_indices+=("$i")
      if already_configured "$i"; then
        detected_status+=("already configured")
      else
        detected_status+=("not configured")
      fi
    fi
  done

  if [[ ${#detected_indices[@]} -eq 0 ]]; then
    echo "No supported AI agent platforms were detected on this system."
    echo ""
    echo "Supported platforms:"
    for name in "${PLATFORM_NAMES[@]}"; do
      echo "  - $name"
    done
    echo ""
    echo "Install one of the above and re-run this script."
    echo ""
    read -rp "Press Enter to close..."
    exit 0
  fi

  echo "Detected platforms:"
  for j in "${!detected_indices[@]}"; do
    local display_num=$((j + 1))
    local idx="${detected_indices[$j]}"
    local name="${PLATFORM_NAMES[$idx]}"
    local status="${detected_status[$j]}"
    printf "  [%d] %-22s (%s)\n" "$display_num" "$name" "$status"
  done

  echo ""
  read -rp "Enter numbers to install (e.g. 1 3), 'a' for all unconfigured, 'q' to quit: " user_input

  if [[ "$user_input" == "q" ]] || [[ "$user_input" == "Q" ]]; then
    echo "Aborted."
    exit 0
  fi

  # Build list of indices to configure
  declare -a to_configure=()

  if [[ "$user_input" == "a" ]] || [[ "$user_input" == "A" ]]; then
    for j in "${!detected_indices[@]}"; do
      if [[ "${detected_status[$j]}" == "not configured" ]]; then
        to_configure+=("${detected_indices[$j]}")
      fi
    done
    if [[ ${#to_configure[@]} -eq 0 ]]; then
      echo ""
      echo "All detected platforms are already configured."
      echo ""
      read -rp "Press Enter to close..."
      exit 0
    fi
  else
    # Parse space-separated numbers
    for num in $user_input; do
      # Validate it's a number
      if ! [[ "$num" =~ ^[0-9]+$ ]]; then
        echo "Invalid input: '$num' — skipping."
        continue
      fi
      local j=$((num - 1))
      if [[ "$j" -lt 0 ]] || [[ "$j" -ge ${#detected_indices[@]} ]]; then
        echo "Invalid selection: $num — skipping."
        continue
      fi
      to_configure+=("${detected_indices[$j]}")
    done
  fi

  if [[ ${#to_configure[@]} -eq 0 ]]; then
    echo ""
    echo "Nothing to configure."
    echo ""
    read -rp "Press Enter to close..."
    exit 0
  fi

  echo ""

  # Configure each selected platform
  declare -a configured_names=()
  declare -a failed_names=()

  for idx in "${to_configure[@]}"; do
    local name="${PLATFORM_NAMES[$idx]}"
    printf "Configuring %s... " "$name"
    if configure_platform "$idx"; then
      echo "done."
      configured_names+=("$name")
    else
      echo "FAILED."
      failed_names+=("$name")
    fi
  done

  # Create sentinel
  mkdir -p "$SENTINEL_DIR"
  date -u '+%Y-%m-%dT%H:%M:%SZ' > "$SENTINEL_FILE"

  # Summary
  echo ""
  echo "─── Summary ───────────────────────────────────────"
  echo ""
  if [[ ${#configured_names[@]} -gt 0 ]]; then
    echo "Successfully configured:"
    for name in "${configured_names[@]}"; do
      echo "  + $name"
    done
  fi
  if [[ ${#failed_names[@]} -gt 0 ]]; then
    echo ""
    echo "Failed to configure:"
    for name in "${failed_names[@]}"; do
      echo "  ! $name"
    done
  fi
  echo ""
  echo "MCP Mux server runs on $MCP_MUX_URL"
  echo "Make sure the MCP Mux app is running (check your system tray)."
  echo ""
  read -rp "Press Enter to close..."
}

main
