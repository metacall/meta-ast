#!/usr/bin/env sh
#
# meta-ast installer
# Downloads and installs the latest (or specified) pre-built meta-ast
# binary from GitHub Releases.
#
# Usage:
#   curl -sSL https://raw.githubusercontent.com/metacall/meta-ast/main/scripts/install.sh | sh
#   curl -sSL .../install.sh | sh -s -- --version v0.5.0
#   curl -sSL .../install.sh | sh -s -- --deploy
#   curl -sSL .../install.sh | sh -s -- --install-dir /usr/local/bin
#
set -eu

REPO="metacall/meta-ast"
BINARY_NAME="meta-ast"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
VERSION="latest"
VARIANT=""  # "" = core, "-deploy" = metacall-deploy feature

err() {
	printf 'Error: %s\n' "$1" >&2
	exit 1
}

info() {
	printf '%s\n' "$1"
}

# ---- Parse arguments -------------------------------------------------

while [ "$#" -gt 0 ]; do
	case "$1" in
		--version)
			VERSION="$2"
			shift 2
			;;
		--deploy)
			VARIANT="-deploy"
			shift
			;;
		--install-dir)
			INSTALL_DIR="$2"
			shift 2
			;;
		*)
			err "Unknown argument: $1"
			;;
	esac
done

# ---- Detect platform ---------------------------------------------------

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
	Linux)
		PLATFORM="unknown-linux-gnu"
		;;
	Darwin)
		PLATFORM="apple-darwin"
		;;
	*)
		err "Unsupported OS: $OS. For Windows, use scripts/install.ps1 instead."
		;;
esac

case "$ARCH" in
	x86_64|amd64)
		ARCH="x86_64"
		;;
	arm64|aarch64)
		ARCH="aarch64"
		;;
	*)
		err "Unsupported architecture: $ARCH"
		;;
esac

# macOS only ships glibc-equivalent builds under apple-darwin (no musl variant)
if [ "$OS" = "Linux" ] && [ -f /etc/alpine-release ]; then
	PLATFORM="unknown-linux-musl"
fi

ASSET="${BINARY_NAME}-${ARCH}-${PLATFORM}${VARIANT}"

# ---- Resolve download URL ----------------------------------------------

if [ "$VERSION" = "latest" ]; then
	DOWNLOAD_URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"
else
	DOWNLOAD_URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}"
fi

info "Detected platform: ${ARCH}-${PLATFORM}${VARIANT}"
info "Downloading: ${DOWNLOAD_URL}"

# ---- Download ------------------------------------------------------------

TMP_FILE="$(mktemp)"
trap 'rm -f "$TMP_FILE"' EXIT

if command -v curl >/dev/null 2>&1; then
	curl --fail --location --show-error --silent -o "$TMP_FILE" "$DOWNLOAD_URL" \
		|| err "Download failed. Check that the version/platform combination exists."
elif command -v wget >/dev/null 2>&1; then
	wget -q -O "$TMP_FILE" "$DOWNLOAD_URL" \
		|| err "Download failed. Check that the version/platform combination exists."
else
	err "Neither curl nor wget is available. Please install one and retry."
fi

# ---- Install ---------------------------------------------------------

mkdir -p "$INSTALL_DIR"
chmod +x "$TMP_FILE"
mv "$TMP_FILE" "${INSTALL_DIR}/${BINARY_NAME}"
trap - EXIT

info "Installed ${BINARY_NAME} to ${INSTALL_DIR}/${BINARY_NAME}"

# ---- PATH check --------------------------------------------------------

case ":$PATH:" in
	*":$INSTALL_DIR:"*)
		: # already on PATH
		;;
	*)
		info ""
		info "NOTE: ${INSTALL_DIR} is not on your PATH."
		info "Add this to your shell profile (e.g. ~/.bashrc, ~/.zshrc):"
		info "  export PATH=\"${INSTALL_DIR}:\$PATH\""
		;;
esac

info ""
info "Run '${BINARY_NAME} --help' to get started."
