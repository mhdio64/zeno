<div align="center">

# ⚡ zeno

**A blazingly fast, non-modal terminal text editor built to replace `nano`.**

Instant open · Universal keymap · Zero input lag · Native Persian/RTL · Atomic save

[![CI](https://github.com/mhdio64/zeno/actions/workflows/ci.yml/badge.svg)](https://github.com/mhdio64/zeno/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/mhdio64/zeno?color=blue)](https://github.com/mhdio64/zeno/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Platforms](https://img.shields.io/badge/Platforms-Linux%20%7C%20macOS%20%7C%20Windows-blueviolet)](https://github.com/mhdio64/zeno/releases)

</div>

---

## 💡 What is zeno?

**zeno** is designed as a *quick editor* — an editor opened for one file, edited with full power, atomically saved, and closed; not an IDE workspace you live in.

* **Instant Open:** Uses memory mapping (`memmap2`) to open files of any size without delay. First frame renders in under 50 ms.
* **Non-Modal Editing:** No normal, insert, or visual modes. Standard keys always type characters, and familiar shortcuts (`Ctrl+S`, `Ctrl+Z`, `Ctrl+F`, `Ctrl+P`) just work.
* **Zero Input Latency:** Zero-allocation hot paths deliver guaranteed 60 FPS (<16 ms keystroke latency). Background tasks (syntax highlighting, regex search) never block the keyboard loop.
* **First-Class Persian & Arabic (RTL BiDi):** Native Unicode contextual shaping (Forms-B for گ، چ، پ، ژ، ک، ی) and bidirectional visual reordering right in your terminal.
* **Hardened & Safe:** Atomic save guards against symlink races, preserves POSIX permissions, and uses RAII raw-mode guards with panic hooks so your terminal is never left broken.

---

## 🚀 Installation

### Linux & macOS (One-Line Install)
Run this single command in your terminal:
```bash
curl -fsSL https://raw.githubusercontent.com/mhdio64/zeno/main/install.sh | bash
```
> [!TIP]
> The installer automatically detects your environment, installs `zeno`, and ensures your shell `PATH` is configured (`~/.bashrc`, `~/.zshrc`, fish). For a system-wide install directly with root privileges, you can also run:
> ```bash
> curl -fsSL https://raw.githubusercontent.com/mhdio64/zeno/main/install.sh | sudo bash
> ```

### Windows (PowerShell One-Line Install)
Run this command in PowerShell:
```powershell
irm https://raw.githubusercontent.com/mhdio64/zeno/main/install.ps1 | iex
```

### Install via Cargo (Rust)
If you have the Rust toolchain installed:
```bash
cargo install --git https://github.com/mhdio64/zeno
```

### Manual Download
Download the pre-compiled binary archive for your OS and architecture from the [GitHub Releases](https://github.com/mhdio64/zeno/releases/latest) page, extract it, and place `zeno` in your `PATH`.

### 🔄 Updating zeno
To update `zeno` to the newest release, simply re-run the installer for your platform:

- **Linux & macOS:**
  ```bash
  curl -fsSL https://raw.githubusercontent.com/mhdio64/zeno/main/install.sh | bash
  ```
  *(Or with `sudo bash` if installed system-wide)*

- **Windows (PowerShell):**
  ```powershell
  irm https://raw.githubusercontent.com/mhdio64/zeno/main/install.ps1 | iex
  ```

- **Via Cargo:**
  ```bash
  cargo install --git https://github.com/mhdio64/zeno --force
  ```

The installer automatically detects the existing install directory, replaces the binary with the latest release, and leaves your configuration file (`zeno.toml`) untouched.

---

## 🎯 Quick Start

Open any file:
```bash
zeno main.rs
```

Pipe content directly from standard input:
```bash
cat file.log | zeno -
```

---

## ⌨️ Keybindings

zeno uses an intuitive three-tier input design:
1. **Plain typing:** Every printable character types itself. Navigation keys (`arrows`, `Home`, `End`, `PageUp`, `PageDown`) navigate immediately.
2. **Chords:** Familiar shortcuts matching your natural muscle memory (`Ctrl+S`, `Ctrl+Z`, etc.).
3. **Leader sequences:** Press `Ctrl+Space` (the leader key), followed by a mnemonic letter.

### Essential Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Ctrl+S` | **Save** file (atomic, permission-preserving) |
| `Ctrl+Q` / `Leader q` | **Quit** editor (prompts if unsaved changes exist) |
| `Ctrl+Z` / `Ctrl+Y` | **Undo** / **Redo** (atomic journal) |
| `Ctrl+P` / `F1` | Open **Command Palette** |
| `Ctrl+F` | **Find** (interactive regex search with live highlight) |
| `Ctrl+H` | **Replace** (single or whole-file replacement) |
| `Ctrl+G` | **Go to line** |
| `Ctrl+A` | **Select all** |
| `Ctrl+X` / `Ctrl+C` / `Ctrl+V` | **Cut** / **Copy** / **Paste** (with OSC 52 SSH clipboard support) |
| `Ctrl+Space` | **Buffer word autocompletion** |
| `Alt+Z` | Toggle **soft word wrap** |
| `F11` / `Leader Z` | Toggle **Zen Mode** (distraction-free view) |
| `Leader T` | Open **Theme Picker** |
| `Leader ?` | Open **Help List** |

### Navigation & Line Manipulation

| Shortcut | Action |
| :--- | :--- |
| `Ctrl+←` / `Ctrl+→` | Move cursor by word |
| `Ctrl+↑` / `Ctrl+↓` | Scroll viewport up / down without moving cursor |
| `Ctrl+Home` / `Ctrl+End` | Jump to start / end of file |
| `Alt+↑` / `Alt+↓` | Move current line or selection up / down |
| `Alt+Shift+↑` / `Alt+Shift+↓` | Duplicate current line up / down |
| `Ctrl+D` | Duplicate current line |
| `Ctrl+K` | Delete to end of line |
| `Ctrl+W` | Delete word before cursor |
| `Ctrl+/` | Toggle line comment |
| `[c` / `]c` | Jump to previous / next Git diff change |

---

## 🌟 Highlight Features

### 1. Surround Selection
Select any text using `Shift + arrow keys` (or `Ctrl+A`), then simply press an opening or closing character:
* Typing `(`, `[`, `{`, `"`, `'`, `` ` ``, or `<` automatically surrounds the selection with matching pairs in a single undoable step!

### 2. Matching Bracket Highlight
Whenever the cursor rests on or adjacent to a bracket `()`, `[]`, `{}` in your code, its matching counterpart anywhere in the viewport is instantly highlighted in cyan.

### 3. Native Persian & Arabic RTL Support
Unlike traditional terminal editors that garble Middle Eastern scripts or disconnect letters, zeno features built-in glyph shaping (Arabic Presentation Forms-B) and visual BiDi reordering. Persian text displays with connected glyphs in proper reading order.

### 4. Interactive Command Palette & Themes
Press `Ctrl+P` or `F1` to search any command in real time. Switch themes on the fly with `Leader T`:
* **VS Code Dark Modern** (default high-contrast deep black `#000000`)
* **Monokai**
* **Solarized Dark** & **Solarized Light**
* **GitHub Dark**

### 5. Git Diff Gutter
The gutter automatically marks modified (`~`), added (`+`), and deleted (`_`) lines relative to the Git index. Press `[c` and `]c` to jump quickly across diff hunks.

---

## ⚙️ Configuration (`zeno.toml`)

zeno automatically loads configuration from `~/.config/zeno/zeno.toml` (or `%APPDATA%\zeno\zeno.toml` on Windows).

Example configuration:
```toml
[settings]
theme = "Dark Modern"  # default: "Dark Modern" (built-in themes: Dark Modern, Monokai, base16-ocean.dark, Solarized (dark), Solarized (light), High Contrast)
tab_size = 4
use_spaces = true
wrap = true
highlight_matching_bracket = true
highlight_word_under_cursor = true
trim_trailing_whitespace = false

[keys]
# Custom chords (example: rebind save to Ctrl+W)
"ctrl+s" = "save"
```

---

## 🛠️ Building from Source

```bash
# Clone repository
git clone https://github.com/mhdio64/zeno.git
cd zeno

# Build optimized binary
cargo build --release

# Run unit & integration test suite
cargo test

# Executable binary located at
./target/release/zeno
```

---

## 📄 License

This project is licensed under the [MIT License](LICENSE).
