#!/usr/bin/env bash
set -euo pipefail

# zeno — installer for Linux and macOS
# Repository: https://github.com/mhdio64/zeno

REPO="mhdio64/zeno"
BOLD="\033[1m"
GREEN="\033[32m"
BLUE="\033[34m"
YELLOW="\033[33m"
RED="\033[31m"
RESET="\033[0m"

echo -e "${BOLD}${BLUE}"
echo "    ███████╗███████╗███╗   ██╗ ██████╗ "
echo "    ╚══███╔╝██╔════╝████╗  ██║██╔═══██╗"
echo "      ███╔╝ █████╗  ██╔██╗ ██║██║   ██║"
echo "     ███╔╝  ██╔══╝  ██║╚██╗██║██║   ██║"
echo "    ███████╗███████╗██║ ╚████║╚██████╔╝"
echo "    ╚══════╝╚══════╝╚═╝  ╚═══╝ ╚═════╝ "
echo -e "${RESET}"
echo -e "${BOLD}Fast, non-modal terminal text editor${RESET}\n"

# 1. Detect Operating System
OS="$(uname -s)"
case "${OS}" in
    Linux*)     PLATFORM="linux" ;;
    Darwin*)    PLATFORM="macos" ;;
    *)          
        echo -e "${RED}Error:${RESET} Unsupported operating system: ${OS}"
        echo "For Windows, run the PowerShell installer instead: irm https://raw.githubusercontent.com/${REPO}/main/install.ps1 | iex"
        exit 1
        ;;
esac

# 2. Detect CPU Architecture
ARCH="$(uname -m)"
case "${ARCH}" in
    x86_64|amd64)   ARCH="x86_64" ;;
    aarch64|arm64)  ARCH="aarch64" ;;
    *)
        echo -e "${RED}Error:${RESET} Unsupported CPU architecture: ${ARCH}"
        exit 1
        ;;
esac

ASSET_NAME="zeno-${PLATFORM}-${ARCH}.tar.gz"
DOWNLOAD_URL="https://github.com/${REPO}/releases/latest/download/${ASSET_NAME}"

echo -e "Detected: ${GREEN}${PLATFORM} (${ARCH})${RESET}"
echo -e "Downloading latest release from GitHub..."

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

if command -v curl >/dev/null 2>&1; then
    curl -fsSL "${DOWNLOAD_URL}" -o "${TMP_DIR}/${ASSET_NAME}"
elif command -v wget >/dev/null 2>&1; then
    wget -q "${DOWNLOAD_URL}" -O "${TMP_DIR}/${ASSET_NAME}"
else
    echo -e "${RED}Error:${RESET} Neither curl nor wget was found on your system."
    exit 1
fi

echo -e "Extracting archive..."
tar -xzf "${TMP_DIR}/${ASSET_NAME}" -C "${TMP_DIR}"

# 3. Determine Installation Directory
INSTALL_DIR="/usr/local/bin"
USE_SUDO=false

if [ ! -w "${INSTALL_DIR}" ]; then
    if [ "$(id -u)" -eq 0 ]; then
        # Already root
        USE_SUDO=false
    elif command -v sudo >/dev/null 2>&1 && [ -t 0 ]; then
        # Sudo available in an interactive shell
        USE_SUDO=true
    else
        # Fall back to user-local directory
        INSTALL_DIR="${HOME}/.local/bin"
        mkdir -p "${INSTALL_DIR}"
        USE_SUDO=false
    fi
fi

TARGET_BIN="${INSTALL_DIR}/zeno"
echo -e "Installing to ${GREEN}${TARGET_BIN}${RESET}..."

if [ "${USE_SUDO}" = true ]; then
    sudo cp "${TMP_DIR}/zeno" "${TARGET_BIN}"
    sudo chmod +x "${TARGET_BIN}"
else
    cp "${TMP_DIR}/zeno" "${TARGET_BIN}"
    chmod +x "${TARGET_BIN}"
fi

echo -e "\n${BOLD}${GREEN}✔ zeno successfully installed!${RESET}"

# Verify PATH
if ! echo "${PATH}" | tr ':' '\n' | grep -qx "${INSTALL_DIR}"; then
    echo -e "\n${YELLOW}Note:${RESET} ${INSTALL_DIR} is not currently in your PATH."
    echo "Add the following line to your ~/.bashrc or ~/.zshrc:"
    echo -e "  ${BOLD}export PATH=\"${INSTALL_DIR}:\$PATH\"${RESET}"
fi

echo -e "\nTo start editing, run:"
echo -e "  ${BOLD}zeno <filename>${RESET}\n"
