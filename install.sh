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

# Helper: check if a directory is in PATH
is_in_path() {
    case ":${PATH}:" in
        *":$1:"*) return 0 ;;
        *) return 1 ;;
    esac
}

# Helper: safely configure PATH in user shell rc files
configure_shell_path() {
    local target_dir="$1"
    
    if is_in_path "${target_dir}"; then
        return 0
    fi

    local export_line
    if [[ "${target_dir}" == "${HOME}"* ]]; then
        export_line="export PATH=\"\$HOME${target_dir#$HOME}:\$PATH\""
    else
        export_line="export PATH=\"${target_dir}:\$PATH\""
    fi

    local updated=()
    local rc_files=("${HOME}/.bashrc" "${HOME}/.zshrc" "${HOME}/.bash_profile" "${HOME}/.profile")
    local found_any=false

    for rc in "${rc_files[@]}"; do
        if [ -f "${rc}" ]; then
            found_any=true
            if ! grep -Fq "${target_dir}" "${rc}" 2>/dev/null && ! grep -Fq "\$HOME${target_dir#$HOME}" "${rc}" 2>/dev/null; then
                printf "\n# Added by zeno installer\n%s\n" "${export_line}" >> "${rc}"
                updated+=("${rc}")
            fi
        fi
    done

    # If no standard shell config file exists, create .bashrc or .profile
    if [ "${found_any}" = false ]; then
        local fallback="${HOME}/.profile"
        if [ -n "${BASH_VERSION:-}" ]; then
            fallback="${HOME}/.bashrc"
        fi
        printf "# Added by zeno installer\n%s\n" "${export_line}" > "${fallback}"
        updated+=("${fallback}")
    fi

    # Fish shell support
    if [ -d "${HOME}/.config/fish" ] || command -v fish >/dev/null 2>&1; then
        local fish_cfg="${HOME}/.config/fish/config.fish"
        mkdir -p "${HOME}/.config/fish"
        local fish_cmd="fish_add_path \"${target_dir}\""
        if [ -f "${fish_cfg}" ]; then
            if ! grep -Fq "${target_dir}" "${fish_cfg}" 2>/dev/null; then
                printf "\n# Added by zeno installer\n%s\n" "${fish_cmd}" >> "${fish_cfg}"
                updated+=("${fish_cfg}")
            fi
        else
            printf "# Added by zeno installer\n%s\n" "${fish_cmd}" > "${fish_cfg}"
            updated+=("${fish_cfg}")
        fi
    fi

    if [ ${#updated[@]} -gt 0 ]; then
        echo -e "${GREEN}✔ Automatically added ${target_dir} to your PATH in:${RESET}"
        for f in "${updated[@]}"; do
            echo -e "    ${BOLD}${f}${RESET}"
        done
    fi
}

# 3. Determine Installation Directory
INSTALL_DIR="/usr/local/bin"
USE_SUDO=false

if [ -n "${ZENO_INSTALL_DIR:-}" ]; then
    INSTALL_DIR="${ZENO_INSTALL_DIR}"
    if [ -w "${INSTALL_DIR}" ] || [ "$(id -u)" -eq 0 ]; then
        USE_SUDO=false
    else
        USE_SUDO=true
    fi
elif [ -w "${INSTALL_DIR}" ]; then
    # Already writable by current user
    USE_SUDO=false
elif [ "$(id -u)" -eq 0 ]; then
    # Already running as root (e.g. `curl ... | sudo bash`)
    USE_SUDO=false
elif is_in_path "${HOME}/.local/bin"; then
    # ~/.local/bin is already in PATH! Zero-config user installation
    INSTALL_DIR="${HOME}/.local/bin"
    USE_SUDO=false
elif [ "${NO_SUDO:-0}" = "1" ] || [ "${ZENO_NO_SUDO:-0}" = "1" ]; then
    # User requested no sudo
    INSTALL_DIR="${HOME}/.local/bin"
    USE_SUDO=false
elif command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
    # User has passwordless sudo or active cached session (e.g. `sudo curl ... | bash`)
    USE_SUDO=true
elif command -v sudo >/dev/null 2>&1 && [ -c /dev/tty ] && [ -r /dev/tty ]; then
    # Interactive terminal where sudo is available
    echo -e "${YELLOW}Installing system-wide to ${INSTALL_DIR} requires administrator privileges.${RESET}"
    echo -e "Enter sudo password (or press Enter to install to ~/.local/bin without sudo):"
    if sudo -v </dev/tty >/dev/tty 2>&1; then
        USE_SUDO=true
    else
        echo -e "${YELLOW}Falling back to user installation in ~/.local/bin...${RESET}"
        INSTALL_DIR="${HOME}/.local/bin"
        USE_SUDO=false
    fi
else
    # Fall back to user-local directory (e.g. non-interactive or no sudo)
    INSTALL_DIR="${HOME}/.local/bin"
    USE_SUDO=false
fi

TARGET_BIN="${INSTALL_DIR}/zeno"
echo -e "Installing to ${GREEN}${TARGET_BIN}${RESET}..."

if [ "${USE_SUDO}" = true ]; then
    sudo mkdir -p "${INSTALL_DIR}"
    sudo cp "${TMP_DIR}/zeno" "${TARGET_BIN}"
    sudo chmod +x "${TARGET_BIN}"
else
    mkdir -p "${INSTALL_DIR}"
    cp "${TMP_DIR}/zeno" "${TARGET_BIN}"
    chmod +x "${TARGET_BIN}"
fi

echo -e "\n${BOLD}${GREEN}✔ zeno successfully installed!${RESET}"

# Ensure PATH is configured
if ! is_in_path "${INSTALL_DIR}"; then
    configure_shell_path "${INSTALL_DIR}"
    echo -e "\n${YELLOW}Note:${RESET} ${INSTALL_DIR} has been added to your PATH configuration."
    echo -e "To start editing in your current terminal session, run:"
    echo -e "  ${BOLD}source ~/.bashrc${RESET}  (or reload your shell)"
    echo -e "  ${BOLD}zeno <filename>${RESET}\n"
else
    echo -e "\nTo start editing, run:"
    echo -e "  ${BOLD}zeno <filename>${RESET}\n"
fi
