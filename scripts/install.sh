#!/usr/bin/env bash
# ArmaraOS installer — works on Linux, macOS, WSL
# Usage: curl -sSf https://armaraos.sh | sh
#   or:  curl -sSf https://ainativelang.com/install.sh | sh
#
# Environment variables:
#   ARMARAOS_INSTALL_DIR  — custom install directory (default: ~/.armaraos/bin)
#   ARMARAOS_VERSION      — install a specific version tag (default: latest)
#
# Legacy aliases (supported for compatibility):
#   OPENFANG_INSTALL_DIR, OPENFANG_VERSION

set -euo pipefail

REPO="sbhooley/armaraos"
INSTALL_DIR="${ARMARAOS_INSTALL_DIR:-${OPENFANG_INSTALL_DIR:-$HOME/.armaraos/bin}}"

detect_platform() {
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)
    case "$ARCH" in
        x86_64|amd64) ARCH="x86_64" ;;
        aarch64|arm64) ARCH="aarch64" ;;
        *) echo "  Unsupported architecture: $ARCH"; exit 1 ;;
    esac
    case "$OS" in
        linux) PLATFORM="${ARCH}-unknown-linux-gnu" ;;
        darwin) PLATFORM="${ARCH}-apple-darwin" ;;
        mingw*|msys*|cygwin*)
            echo ""
            echo "  For Windows, use PowerShell instead:"
            echo "    irm https://armaraos.sh/install.ps1 | iex   (or use GitHub releases)"
            echo ""
            echo "  Or download the .msi installer from:"
            echo "    https://github.com/$REPO/releases/latest"
            echo ""
            echo "  Or install via cargo:"
            echo "    cargo install --git https://github.com/$REPO openfang-cli"
            exit 1
            ;;
        *) echo "  Unsupported OS: $OS"; exit 1 ;;
    esac
}

install() {
    detect_platform

    echo ""
    echo "  ArmaraOS Installer"
    echo "  =================="
    echo ""

    # Get latest version
    if [ -n "${ARMARAOS_VERSION:-}" ]; then
        VERSION="$ARMARAOS_VERSION"
        echo "  Using specified version: $VERSION"
    elif [ -n "${OPENFANG_VERSION:-}" ]; then
        VERSION="$OPENFANG_VERSION"
        echo "  Using specified version: $VERSION"
    else
        echo "  Fetching latest release..."
        VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | sed 's/.*"tag_name": *"//' | sed 's/".*//')
    fi

    if [ -z "$VERSION" ]; then
        echo "  Could not determine latest version."
        echo "  Install from source instead:"
        echo "    cargo install --git https://github.com/$REPO openfang-cli"
        exit 1
    fi

    URL="https://github.com/$REPO/releases/download/$VERSION/openfang-$PLATFORM.tar.gz"
    CHECKSUM_URL="$URL.sha256"

    echo "  Installing ArmaraOS (CLI) $VERSION for $PLATFORM..."
    mkdir -p "$INSTALL_DIR"

    # Download to temp
    TMPDIR=$(mktemp -d)
    ARCHIVE="$TMPDIR/openfang.tar.gz"
    CHECKSUM_FILE="$TMPDIR/checksum.sha256"

    cleanup() { rm -rf "$TMPDIR"; }
    trap cleanup EXIT

    if ! curl -fsSL "$URL" -o "$ARCHIVE" 2>/dev/null; then
        echo "  Download failed. The release may not exist for your platform."
        echo "  Install from source instead:"
        echo "    cargo install --git https://github.com/$REPO openfang-cli"
        exit 1
    fi

    # Verify checksum if available
    if curl -fsSL "$CHECKSUM_URL" -o "$CHECKSUM_FILE" 2>/dev/null; then
        EXPECTED=$(cut -d ' ' -f 1 < "$CHECKSUM_FILE")
        if command -v sha256sum &>/dev/null; then
            ACTUAL=$(sha256sum "$ARCHIVE" | cut -d ' ' -f 1)
        elif command -v shasum &>/dev/null; then
            ACTUAL=$(shasum -a 256 "$ARCHIVE" | cut -d ' ' -f 1)
        else
            ACTUAL=""
        fi
        if [ -n "$ACTUAL" ]; then
            if [ "$EXPECTED" != "$ACTUAL" ]; then
                echo "  Checksum verification FAILED!"
                echo "    Expected: $EXPECTED"
                echo "    Got:      $ACTUAL"
                exit 1
            fi
            echo "  Checksum verified."
        else
            echo "  No sha256sum/shasum found, skipping checksum verification."
        fi
    fi

    # Extract
    tar xzf "$ARCHIVE" -C "$INSTALL_DIR"
    chmod +x "$INSTALL_DIR/openfang"

    # Convenience alias: provide `armaraos` command while keeping `openfang` for compatibility.
    # (The upstream binary is still named `openfang` in the release archives.)
    cat > "$INSTALL_DIR/armaraos" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
exec "$DIR/openfang" "$@"
EOF
    chmod +x "$INSTALL_DIR/armaraos"

    # Ad-hoc codesign on macOS (prevents SIGKILL on Apple Silicon)
    # Must strip extended attributes (com.apple.quarantine) BEFORE signing,
    # otherwise the signature is computed over the quarantine xattr and macOS
    # rejects it as "Code Signature Invalid" → SIGKILL.
    if [ "$OS" = "darwin" ]; then
        if command -v xattr &>/dev/null; then
            xattr -cr "$INSTALL_DIR/openfang" 2>/dev/null || true
        fi
        if command -v codesign &>/dev/null; then
            if ! codesign --force --sign - "$INSTALL_DIR/openfang"; then
                echo ""
                echo "  Warning: ad-hoc code signing failed."
                echo "  On Apple Silicon, the binary may be killed (SIGKILL) by Gatekeeper."
                echo "  Try manually: xattr -cr $INSTALL_DIR/openfang && codesign --force --sign - $INSTALL_DIR/openfang"
                echo ""
            fi
        fi
    fi

    # Add to PATH — detect the user's login shell
    USER_SHELL="${SHELL:-}"
    # Fallback: check /etc/passwd if $SHELL is unset (e.g. minimal containers)
    if [ -z "$USER_SHELL" ] && command -v getent &>/dev/null; then
        USER_SHELL=$(getent passwd "$(id -un)" 2>/dev/null | cut -d: -f7)
    fi
    if [ -z "$USER_SHELL" ] && [ -f /etc/passwd ]; then
        USER_SHELL=$(grep "^$(id -un):" /etc/passwd 2>/dev/null | cut -d: -f7)
    fi

    SHELL_RC=""
    case "$USER_SHELL" in
        */zsh)  SHELL_RC="$HOME/.zshrc" ;;
        */bash) SHELL_RC="$HOME/.bashrc" ;;
        */fish) SHELL_RC="$HOME/.config/fish/config.fish" ;;
    esac
    # Also check for config files if shell detection failed.
    # Check bash/zsh first (more common defaults), fish last — avoids
    # writing to config.fish for users who merely have Fish installed.
    if [ -z "$SHELL_RC" ]; then
        if [ -f "$HOME/.bashrc" ]; then
            SHELL_RC="$HOME/.bashrc"
        elif [ -f "$HOME/.zshrc" ]; then
            SHELL_RC="$HOME/.zshrc"
        elif [ -f "$HOME/.config/fish/config.fish" ]; then
            SHELL_RC="$HOME/.config/fish/config.fish"
        fi
    fi

    if [ -n "$SHELL_RC" ] && ! grep -q "openfang" "$SHELL_RC" 2>/dev/null; then
        # Determine syntax from the TARGET FILE, not $USER_SHELL — this
        # prevents Bash syntax from ever being written to config.fish even
        # when shell detection mis-identifies the user's shell.
        case "$SHELL_RC" in
            */config.fish)
                mkdir -p "$(dirname "$SHELL_RC")"
                echo "fish_add_path \"$INSTALL_DIR\"" >> "$SHELL_RC"
                ;;
            *)
                echo "export PATH=\"$INSTALL_DIR:\$PATH\"" >> "$SHELL_RC"
                ;;
        esac
        echo "  Added $INSTALL_DIR to PATH in $SHELL_RC"
    fi

    # Verify installation
    if "$INSTALL_DIR/openfang" --version >/dev/null 2>&1; then
        INSTALLED_VERSION=$("$INSTALL_DIR/openfang" --version 2>/dev/null || echo "$VERSION")
        echo ""
        echo "  ArmaraOS installed successfully! ($INSTALLED_VERSION)"
    else
        echo ""
        echo "  ArmaraOS binary installed to $INSTALL_DIR/openfang (and wrapper $INSTALL_DIR/armaraos)"
    fi

    # Required: install AINL and register MCP server into ~/.armaraos/config.toml.
    echo ""
    echo "  Installing AINL: ainativelang[mcp]"

    PYTHON=""
    if command -v python3 >/dev/null 2>&1; then
        PYTHON="python3"
    elif command -v python >/dev/null 2>&1; then
        PYTHON="python"
    else
        echo "  Error: python is required to install AINL (ainativelang[mcp])"
        exit 1
    fi

    # Use user-site by default (avoid requiring sudo). This writes `ainl` into USER_BASE/bin.
    "$PYTHON" -m pip install --upgrade pip >/dev/null 2>&1 || true
    "$PYTHON" -m pip install --user "ainativelang[mcp]"

    USER_BASE="$("$PYTHON" -c 'import site; print(site.USER_BASE)')"
    AINL_BIN="$USER_BASE/bin/ainl"
    if [ ! -x "$AINL_BIN" ]; then
        # Some environments (Homebrew Python) may place scripts in the framework bin.
        AINL_BIN="$(command -v ainl 2>/dev/null || true)"
    fi
    if [ -z "${AINL_BIN:-}" ] || [ ! -x "${AINL_BIN:-/dev/null}" ]; then
        echo "  Error: AINL installed but could not find executable 'ainl'."
        echo "  Hint: ensure your Python user bin is on PATH:"
        echo "    export PATH=\"$USER_BASE/bin:\$PATH\""
        exit 1
    fi

    echo "  Registering AINL MCP server for ArmaraOS..."
    "$AINL_BIN" install-mcp --host armaraos

    echo ""
    echo "  Get started:"
    echo "    armaraos init   (or: openfang init)"
    echo ""
    echo "  AINL is installed. If you want `ainl` on PATH permanently, add:"
    echo "    export PATH=\"$USER_BASE/bin:\$PATH\""
    echo ""
    echo "  The setup wizard will guide you through provider selection"
    echo "  and configuration."
    echo ""
}

install
