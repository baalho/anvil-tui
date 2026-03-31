#!/bin/sh
# Install Anvil — detects OS/arch, downloads binary, installs to ~/.local/bin/
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/baalho/anvil-tui/main/scripts/install.sh | sh
#
# Environment variables:
#   ANVIL_VERSION  — version to install (default: latest)
#   ANVIL_DIR      — install directory (default: ~/.local/bin)

set -e

REPO="baalho/anvil-tui"
INSTALL_DIR="${ANVIL_DIR:-$HOME/.local/bin}"

# Detect OS
OS="$(uname -s)"
case "$OS" in
    Linux)  PLATFORM="linux" ;;
    Darwin) PLATFORM="macos" ;;
    *)
        echo "error: unsupported OS: $OS" >&2
        echo "Anvil supports Linux and macOS. For Windows, use WSL." >&2
        exit 1
        ;;
esac

# Detect architecture
ARCH="$(uname -m)"
case "$ARCH" in
    x86_64|amd64) ARCH_SUFFIX="x86_64" ;;
    arm64|aarch64) ARCH_SUFFIX="arm64" ;;
    *)
        echo "error: unsupported architecture: $ARCH" >&2
        exit 1
        ;;
esac

ARTIFACT="anvil-${PLATFORM}-${ARCH_SUFFIX}"

# Determine version
if [ -z "$ANVIL_VERSION" ]; then
    ANVIL_VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
    if [ -z "$ANVIL_VERSION" ]; then
        echo "error: could not determine latest version" >&2
        echo "Set ANVIL_VERSION manually, e.g.: ANVIL_VERSION=v0.1.0 sh install.sh" >&2
        exit 1
    fi
fi

echo "Installing Anvil ${ANVIL_VERSION} (${PLATFORM}/${ARCH_SUFFIX})..."

# Download
DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${ANVIL_VERSION}/${ARTIFACT}"
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

echo "Downloading ${DOWNLOAD_URL}..."
curl -fsSL -o "${TMPDIR}/anvil" "$DOWNLOAD_URL"

# Verify checksum if available
CHECKSUM_URL="${DOWNLOAD_URL}.sha256"
if curl -fsSL -o "${TMPDIR}/anvil.sha256" "$CHECKSUM_URL" 2>/dev/null; then
    echo "Verifying checksum..."
    cd "$TMPDIR"
    if command -v sha256sum >/dev/null 2>&1; then
        echo "$(cat anvil.sha256)  anvil" | sha256sum -c -
    elif command -v shasum >/dev/null 2>&1; then
        echo "$(cat anvil.sha256)  anvil" | shasum -a 256 -c -
    else
        echo "warning: no sha256 tool found, skipping checksum verification" >&2
    fi
    cd - >/dev/null
fi

# Install
mkdir -p "$INSTALL_DIR"
chmod +x "${TMPDIR}/anvil"
mv "${TMPDIR}/anvil" "${INSTALL_DIR}/anvil"

echo "Installed to ${INSTALL_DIR}/anvil"

# Check PATH
case ":$PATH:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        echo ""
        echo "Add ${INSTALL_DIR} to your PATH:"
        echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
        echo ""
        echo "Add this to your shell profile (~/.bashrc, ~/.zshrc, etc.)"
        ;;
esac

echo "Done. Run 'anvil --help' to get started."
