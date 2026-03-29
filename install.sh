#!/usr/bin/env bash
set -euo pipefail
umask 022

# === Config ===
BINARY_NAME="scope"
MCP_BINARY_NAME="scope-mcp"
OWNER="quangdang46"
REPO="scope"
DEST="${DEST:-$HOME/.local/bin}"
VERSION="${VERSION:-}"
QUIET=0; EASY=0; VERIFY=0; FROM_SOURCE=0; UNINSTALL=0
MAX_RETRIES=3; DOWNLOAD_TIMEOUT=120
LOCK_DIR="/tmp/${BINARY_NAME}-install.lock.d"
TMP=""

# === Logging ===
log_info()    { [ "$QUIET" -eq 1 ] && return; echo "[${BINARY_NAME}] $*" >&2; }
log_warn()    { echo "[${BINARY_NAME}] WARN: $*" >&2; }
log_success() { [ "$QUIET" -eq 1 ] && return; echo "✓ $*" >&2; }
die()         { echo "ERROR: $*" >&2; exit 1; }

# === Cleanup & lock ===
cleanup() { rm -rf "$TMP" "$LOCK_DIR" 2>/dev/null || true; }
trap cleanup EXIT
acquire_lock() {
    mkdir "$LOCK_DIR" 2>/dev/null || die "Another install running. rm -rf $LOCK_DIR"
    echo $$ > "$LOCK_DIR/pid"
}

# === Args ===
while [ $# -gt 0 ]; do
    case "$1" in
        --dest)       DEST="$2";   shift 2;;
        --dest=*)     DEST="${1#*=}"; shift;;
        --version)    VERSION="$2"; shift 2;;
        --version=*)  VERSION="${1#*=}"; shift;;
        --system)     DEST="/usr/local/bin"; shift;;
        --easy-mode)  EASY=1;      shift;;
        --verify)     VERIFY=1;    shift;;
        --from-source) FROM_SOURCE=1; shift;;
        --quiet|-q)   QUIET=1;     shift;;
        --uninstall)  UNINSTALL=1; shift;;
        *) shift;;
    esac
done

# === Uninstall ===
if [ "$UNINSTALL" -eq 1 ]; then
    rm -f "$DEST/$BINARY_NAME"
    rm -f "$DEST/$MCP_BINARY_NAME"
    for rc in "$HOME/.bashrc" "$HOME/.zshrc"; do
        [ -f "$rc" ] && sed -i "/${BINARY_NAME} installer/d" "$rc" 2>/dev/null || true
    done
    echo "✓ ${BINARY_NAME} uninstalled"; exit 0
fi

# === Platform ===
detect_platform() {
    local os arch
    case "$(uname -s)" in
        Linux*)  os="linux";;   Darwin*) os="macos";;
        MINGW*|MSYS*|CYGWIN*) os="windows";;
        *) die "Unsupported OS";;
    esac
    case "$(uname -m)" in
        x86_64|amd64)  arch="x86_64";;
        aarch64|arm64) arch="aarch64";;
        *) die "Unsupported arch";;
    esac
    echo "${os}-${arch}"
}

# === Version ===
resolve_version() {
    [ -n "$VERSION" ] && return 0
    FULL_TAG=$(curl -fsSL --connect-timeout 10 --max-time 30 \
        "https://api.github.com/repos/${OWNER}/${REPO}/releases/latest" 2>/dev/null \
        | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/') || true
    if ! [[ "$FULL_TAG" =~ v[0-9] ]]; then
        FULL_TAG=$(curl -fsSL -o /dev/null -w '%{url_effective}' \
            "https://github.com/${OWNER}/${REPO}/releases/latest" 2>/dev/null \
            | sed -E 's|.*/tag/||') || true
    fi
    [[ "$FULL_TAG" =~ v[0-9] ]] || die "Could not resolve version"

    if [[ "$FULL_TAG" =~ (v[0-9]+\.[0-9]+\.[0-9]+.*)$ ]]; then
       VERSION="${BASH_REMATCH[1]}"
    else
       VERSION="$FULL_TAG"
    fi

    log_info "Latest: $FULL_TAG ($VERSION)"
}

# === Download ===
download_file() {
    local url="$1" dest="$2" partial="${2}.part" attempt=0
    while [ $attempt -lt $MAX_RETRIES ]; do
        attempt=$((attempt + 1))
        curl -fL --connect-timeout 30 --max-time "$DOWNLOAD_TIMEOUT" \
             -sS --retry 2 \
             $( [ -s "$partial" ] && echo "--continue-at -") \
             -o "$partial" "$url" \
          && mv -f "$partial" "$dest" && return 0
        [ $attempt -lt $MAX_RETRIES ] && { log_warn "Retry $attempt..."; sleep 3; }
    done
    return 1
}

# === Atomic install ===
install_binary_atomic() {
    local tmp="${2}.tmp.$$"
    install -m 0755 "$1" "$tmp" && mv -f "$tmp" "$2" || { rm -f "$tmp"; die "Install failed"; }
}

# === PATH ===
maybe_add_path() {
    case ":$PATH:" in *":$DEST:"*) return 0;; esac
    if [ "$EASY" -eq 1 ]; then
        for rc in "$HOME/.zshrc" "$HOME/.bashrc"; do
            [ -f "$rc" ] && [ -w "$rc" ] || continue
            grep -qF "$DEST" "$rc" && continue
            printf '\nexport PATH="%s:$PATH"  # %s installer\n' "$DEST" "$BINARY_NAME" >> "$rc"
        done
    fi
    log_warn "Restart shell or: export PATH=\"$DEST:\$PATH\""
}

# === Source build ===
build_from_source() {
    command -v cargo >/dev/null || die "cargo not found — install Rust: https://rustup.rs"
    git clone --depth 1 "https://github.com/${OWNER}/${REPO}.git" "$TMP/src"
    (cd "$TMP/src" && CARGO_TARGET_DIR="$TMP/target" cargo build --release --package scope-cli --package scope-mcp)
    install_binary_atomic "$TMP/target/release/$BINARY_NAME" "$DEST/$BINARY_NAME"
    install_binary_atomic "$TMP/target/release/$MCP_BINARY_NAME" "$DEST/$MCP_BINARY_NAME"
}

install_release_binary() {
    local bin_name="$1" platform="$2" ext="$3"
    local archive="${bin_name}-${VERSION}-${platform}.${ext}"
    local url="https://github.com/${OWNER}/${REPO}/releases/download/${FULL_TAG}/${archive}"

    download_file "$url" "$TMP/$archive" || return 1

    if download_file "${url}.sha256" "$TMP/${bin_name}.sha256" 2>/dev/null; then
        local expected actual
        expected=$(awk '{print $1}' "$TMP/${bin_name}.sha256")
        actual=$(sha256sum "$TMP/$archive" 2>/dev/null | awk '{print $1}' \
              || shasum -a 256 "$TMP/$archive" | awk '{print $1}')
        [ "$expected" = "$actual" ] || die "Checksum mismatch for ${bin_name}"
        log_info "Checksum verified for ${bin_name}"
    fi

    case "$archive" in
        *.tar.gz) tar -xzf "$TMP/$archive" -C "$TMP";;
        *.zip)    unzip -q "$TMP/$archive" -d "$TMP";;
    esac

    local bin
    if [[ "$platform" == windows* ]]; then
        bin=$(find "$TMP" -type f -name "${bin_name}.exe" 2>/dev/null | head -1)
    else
        bin=$(find "$TMP" -type f -name "${bin_name}" 2>/dev/null | head -1)
    fi
    [ -n "$bin" ] || die "Binary ${bin_name} not found after extract"
    install_binary_atomic "$bin" "$DEST/$bin_name"
}

auto_install_mcp() {
    [ -x "$DEST/$BINARY_NAME" ] || return 0
    [ -x "$DEST/$MCP_BINARY_NAME" ] || return 0

    log_info "Auto-installing MCP config for detected user tools"
    if ! "$DEST/$BINARY_NAME" install-mcp --auto-user; then
        log_warn "MCP auto-install reported a problem; you can retry with: $DEST/$BINARY_NAME install-mcp --auto-user"
    fi
}

# === Main ===
main() {
    acquire_lock
    TMP=$(mktemp -d)
    mkdir -p "$DEST"

    local platform; platform=$(detect_platform)
    log_info "Platform: $platform | Dest: $DEST"

    if [ "$FROM_SOURCE" -eq 0 ]; then
        resolve_version
        
        if [ -z "${FULL_TAG:-}" ]; then
            if [[ "$VERSION" =~ ^scope- ]]; then
                FULL_TAG="$VERSION"
                VERSION="${VERSION#scope-}"
            else
                FULL_TAG="scope-$VERSION"
            fi
        fi

        local ext="tar.gz"; [[ "$platform" == windows* ]] && ext="zip"

        if ! install_release_binary "$BINARY_NAME" "$platform" "$ext" \
            || ! install_release_binary "$MCP_BINARY_NAME" "$platform" "$ext"; then
            log_warn "Binary download failed — building from source..."
            build_from_source
        fi
    else
        build_from_source
    fi

    maybe_add_path
    auto_install_mcp

    [ "$VERIFY" -eq 1 ] && "$DEST/$BINARY_NAME" --version

    echo ""
    echo "✓ $BINARY_NAME installed → $DEST/$BINARY_NAME"
    echo "  $("$DEST/$BINARY_NAME" --version 2>/dev/null || true)"
    echo "✓ $MCP_BINARY_NAME installed → $DEST/$MCP_BINARY_NAME"
    echo ""
    echo "  Usage: $BINARY_NAME --help"
}

# curl|bash safety: buffer entire script before executing
if [[ "${BASH_SOURCE[0]:-}" == "${0:-}" ]] || [[ -z "${BASH_SOURCE[0]:-}" ]]; then
    { main "$@"; }
fi
