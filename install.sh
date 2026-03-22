#!/usr/bin/env bash
set -euo pipefail
umask 022

BINARY_NAME="scope"
OWNER="quangdang46"
REPO="scope"
DEST="${DEST:-$HOME/.local/bin}"
VERSION="${VERSION:-}"
TAG_NAME=""
QUIET=0
EASY=0
VERIFY=0
FROM_SOURCE=0
UNINSTALL=0
MAX_RETRIES=3
DOWNLOAD_TIMEOUT=120
LOCK_DIR="/tmp/${BINARY_NAME}-install.lock.d"
TMP=""

log_info() {
    [ "$QUIET" -eq 1 ] && return
    echo "[${BINARY_NAME}] $*" >&2
}

log_warn() {
    echo "[${BINARY_NAME}] WARN: $*" >&2
}

log_success() {
    [ "$QUIET" -eq 1 ] && return
    echo "✓ $*" >&2
}

die() {
    echo "ERROR: $*" >&2
    exit 1
}

usage() {
    cat <<EOF
Install ${BINARY_NAME} from GitHub releases.

Usage: install.sh [options]
  --dest PATH         Install into PATH
  --dest=PATH         Install into PATH
  --version VERSION   Install a specific version or tag
  --version=VERSION   Install a specific version or tag
  --system            Install into /usr/local/bin
  --easy-mode         Add DEST to PATH in shell rc files
  --verify            Run ${BINARY_NAME} --version after install
  --from-source       Build from source instead of downloading a release
  --quiet, -q         Reduce log output
  --uninstall         Remove the installed binary
  -h, --help          Show this help
EOF
    exit 0
}

cleanup() {
    rm -rf "$TMP" "$LOCK_DIR" 2>/dev/null || true
}

trap cleanup EXIT

acquire_lock() {
    mkdir "$LOCK_DIR" 2>/dev/null || die "Another install is running. If stuck: rm -rf $LOCK_DIR"
    echo $$ > "$LOCK_DIR/pid"
}

normalize_version() {
    case "$1" in
        release-all-v*) printf '%s' "${1#release-all-}" ;;
        ${BINARY_NAME}-v*) printf '%s' "${1#${BINARY_NAME}-}" ;;
        v*) printf '%s' "$1" ;;
        *) printf '%s' "$1" ;;
    esac
}

release_tag_candidates() {
    local requested="$1"
    local normalized
    normalized="$(normalize_version "$requested")"
    printf '%s\n' "$requested"
    [ "$normalized" = "$requested" ] || printf '%s\n' "$normalized"
    printf '%s\n' "release-all-${normalized}"
    printf '%s\n' "${BINARY_NAME}-${normalized}"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --dest)
            [ $# -ge 2 ] || die "Missing value for --dest"
            DEST="$2"
            shift 2
            ;;
        --dest=*)
            DEST="${1#*=}"
            shift
            ;;
        --version)
            [ $# -ge 2 ] || die "Missing value for --version"
            VERSION="$2"
            shift 2
            ;;
        --version=*)
            VERSION="${1#*=}"
            shift
            ;;
        --system)
            DEST="/usr/local/bin"
            shift
            ;;
        --easy-mode)
            EASY=1
            shift
            ;;
        --verify)
            VERIFY=1
            shift
            ;;
        --from-source)
            FROM_SOURCE=1
            shift
            ;;
        --quiet|-q)
            QUIET=1
            shift
            ;;
        --uninstall)
            UNINSTALL=1
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            die "Unknown option: $1"
            ;;
    esac
done

detect_platform() {
    local os arch
    case "$(uname -s)" in
        Linux*) os="linux" ;;
        Darwin*) os="macos" ;;
        MINGW*|MSYS*|CYGWIN*) os="windows" ;;
        *) die "Unsupported OS: $(uname -s)" ;;
    esac
    case "$(uname -m)" in
        x86_64|amd64) arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *) die "Unsupported arch: $(uname -m)" ;;
    esac
    echo "${os}_${arch}"
}

asset_suffix_for_platform() {
    case "$1" in
        linux_x86_64) echo "linux-x86_64" ;;
        linux_aarch64) echo "linux-aarch64" ;;
        macos_x86_64) echo "macos-x86_64" ;;
        macos_aarch64) echo "macos-aarch64" ;;
        windows_x86_64) echo "windows-x86_64" ;;
        *) die "Unsupported platform: $1" ;;
    esac
}

binary_filename_for_platform() {
    case "$1" in
        windows_*) echo "${BINARY_NAME}.exe" ;;
        *) echo "$BINARY_NAME" ;;
    esac
}

resolve_version() {
    if [ -n "$VERSION" ]; then
        VERSION="$(normalize_version "$VERSION")"
        TAG_NAME="$VERSION"
        return 0
    fi

    TAG_NAME=$(curl -fsSL \
        --connect-timeout 10 \
        --max-time 30 \
        -H "Accept: application/vnd.github.v3+json" \
        "https://api.github.com/repos/${OWNER}/${REPO}/releases/latest" \
        2>/dev/null | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/') || true

    case "$TAG_NAME" in
        v[0-9]*|scope-v[0-9]*|release-all-v[0-9]*) ;;
        *)
            TAG_NAME=$(curl -fsSL \
                --connect-timeout 10 \
                --max-time 30 \
                -H "Accept: application/vnd.github.v3+json" \
                "https://api.github.com/repos/${OWNER}/${REPO}/releases" \
                2>/dev/null \
                | grep '"tag_name":' \
                | sed -E 's/.*"([^"]+)".*/\1/' \
                | grep -E '^(v[0-9]|scope-v[0-9]|release-all-v[0-9])' \
                | head -n 1) || true
            ;;
    esac

    if [ -z "$TAG_NAME" ]; then
        TAG_NAME=$(curl -fsSL -o /dev/null -w '%{url_effective}' \
            "https://github.com/${OWNER}/${REPO}/releases/latest" \
            2>/dev/null | sed -E 's|.*/tag/||') || true
    fi

    [ -n "$TAG_NAME" ] || die "Could not resolve version"
    VERSION="$(normalize_version "$TAG_NAME")"
    [[ "$VERSION" =~ ^v[0-9] ]] || die "Could not resolve version"
    log_info "Latest: $TAG_NAME"
}

download_file() {
    local url="$1"
    local dest="$2"
    local partial="${dest}.part"
    local attempt=0

    while [ $attempt -lt $MAX_RETRIES ]; do
        attempt=$((attempt + 1))
        if curl -fL \
            --connect-timeout 30 \
            --max-time "$DOWNLOAD_TIMEOUT" \
            --retry 2 \
            $( [ -s "$partial" ] && echo "--continue-at -" ) \
            $( [ "$QUIET" -eq 0 ] && [ -t 2 ] && echo "--progress-bar" || echo "-sS" ) \
            -o "$partial" "$url"; then
            mv -f "$partial" "$dest"
            return 0
        fi
        if [ $attempt -lt $MAX_RETRIES ]; then
            log_warn "Retrying in 3s..."
            sleep 3
        fi
    done

    return 1
}

checksum_file() {
    local file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | awk '{print $1}'
    else
        shasum -a 256 "$file" | awk '{print $1}'
    fi
}

verify_checksum() {
    local archive_path="$1"
    local checksum_path="$2"
    local expected actual
    expected=$(awk '{print $1}' "$checksum_path")
    actual=$(checksum_file "$archive_path")
    [ "$expected" = "$actual" ] || die "Checksum mismatch"
}

extract_archive() {
    local archive_path="$1"
    local dest_dir="$2"
    case "$archive_path" in
        *.tar.gz) tar -xzf "$archive_path" -C "$dest_dir" ;;
        *.zip) unzip -q "$archive_path" -d "$dest_dir" ;;
        *) die "Unsupported archive format: $archive_path" ;;
    esac
}

install_binary_atomic() {
    local src="$1"
    local dest="$2"
    local tmp="${dest}.tmp.$$"
    install -m 0755 "$src" "$tmp"
    mv -f "$tmp" "$dest" || {
        rm -f "$tmp"
        die "Failed to install binary"
    }
}

remove_path_marker() {
    local file="$1"
    local tmp="${file}.tmp.$$"
    awk -v marker="# ${BINARY_NAME} installer" 'index($0, marker) == 0 { print }' "$file" > "$tmp"
    mv -f "$tmp" "$file"
}

do_uninstall() {
    rm -f "$DEST/$BINARY_NAME" "$DEST/${BINARY_NAME}.exe"
    for rc in "$HOME/.bashrc" "$HOME/.zshrc"; do
        [ -f "$rc" ] || continue
        remove_path_marker "$rc"
    done
    log_success "Uninstalled"
    exit 0
}

maybe_add_path() {
    case ":$PATH:" in
        *":$DEST:"*) return 0 ;;
    esac

    if [ "$EASY" -eq 1 ]; then
        for rc in "$HOME/.zshrc" "$HOME/.bashrc"; do
            [ -f "$rc" ] && [ -w "$rc" ] || continue
            grep -qF "# ${BINARY_NAME} installer" "$rc" && continue
            printf '\nexport PATH="%s:$PATH"  # %s installer\n' "$DEST" "$BINARY_NAME" >> "$rc"
        done
        log_warn "PATH updated — restart shell or: export PATH=\"$DEST:\$PATH\""
    else
        log_warn "Add to PATH: export PATH=\"$DEST:\$PATH\""
    fi
}

build_from_source() {
    command -v cargo >/dev/null || die "Rust/cargo not found. Install: https://rustup.rs"
    command -v git >/dev/null || die "git not found"

    local clone_args=(clone --depth 1)
    if [ -n "$TAG_NAME" ]; then
        clone_args+=(--branch "$TAG_NAME")
    fi

    git "${clone_args[@]}" "https://github.com/${OWNER}/${REPO}.git" "$TMP/src"
    (
        cd "$TMP/src"
        CARGO_TARGET_DIR="$TMP/target" cargo build --locked --release --bin "$BINARY_NAME"
    )
    install_binary_atomic "$TMP/target/release/$BINARY_NAME" "$DEST/$BINARY_NAME"
}

print_summary() {
    local installed_bin="$1"
    echo ""
    echo "✓ ${BINARY_NAME} installed → $installed_bin"
    echo "  Version: $("$installed_bin" --version 2>/dev/null || echo 'unknown')"
    echo ""
    echo "  Quick start:"
    echo "    ${BINARY_NAME} --help"
}

main() {
    acquire_lock
    TMP=$(mktemp -d)
    mkdir -p "$DEST"

    if [ "$UNINSTALL" -eq 1 ]; then
        do_uninstall
    fi

    local platform asset_suffix archive_ext executable installed_bin archive_name extracted_bin
    platform="$(detect_platform)"
    asset_suffix="$(asset_suffix_for_platform "$platform")"
    executable="$(binary_filename_for_platform "$platform")"
    installed_bin="$DEST/$executable"
    archive_ext="tar.gz"
    case "$platform" in
        windows_*) archive_ext="zip" ;;
    esac

    log_info "Platform: $platform | Dest: $DEST"

    if [ "$FROM_SOURCE" -eq 0 ]; then
        resolve_version
        archive_name="${BINARY_NAME}-${VERSION}-${asset_suffix}.${archive_ext}"

        local downloaded=0 candidate url archive_path checksum_path
        archive_path="$TMP/$archive_name"
        checksum_path="$TMP/checksum.sha256"

        while IFS= read -r candidate; do
            [ -n "$candidate" ] || continue
            url="https://github.com/${OWNER}/${REPO}/releases/download/${candidate}/${archive_name}"
            if download_file "$url" "$archive_path"; then
                TAG_NAME="$candidate"
                downloaded=1
                if download_file "${url}.sha256" "$checksum_path"; then
                    verify_checksum "$archive_path" "$checksum_path"
                    log_info "Checksum verified"
                fi
                break
            fi
        done <<EOF
$(release_tag_candidates "${TAG_NAME:-$VERSION}")
EOF

        if [ "$downloaded" -eq 1 ]; then
            extract_archive "$archive_path" "$TMP"
            extracted_bin=$(find "$TMP" -name "$executable" -type f | head -n 1)
            [ -n "$extracted_bin" ] || die "Binary not found after extract"
            install_binary_atomic "$extracted_bin" "$installed_bin"
        else
            log_warn "Binary download failed — building from source..."
            build_from_source
            installed_bin="$DEST/$BINARY_NAME"
        fi
    else
        resolve_version
        build_from_source
        installed_bin="$DEST/$BINARY_NAME"
    fi

    maybe_add_path

    if [ "$VERIFY" -eq 1 ]; then
        "$installed_bin" --version >/dev/null
    fi

    print_summary "$installed_bin"
}

if [[ "${BASH_SOURCE[0]:-}" == "${0:-}" ]] || [[ -z "${BASH_SOURCE[0]:-}" ]]; then
    { main "$@"; }
fi
