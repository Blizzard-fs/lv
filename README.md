# lv

Terminal log viewer for PHP applications. Parses SpolsMVC and Laravel log formats, collapses duplicate entries, and supports tailing live files.

## Features

- Auto-discovers log files from project root
- Parses SpolsMVC (`runtime-errors.log`, `database-errors.log`) and Laravel (`storage/logs/laravel.log`)
- Collapses consecutive duplicate entries with a count
- File picker sidebar when multiple log files are found
- Follow mode - polls for file changes and scrolls to new entries
- Inline search filter

## Installation

Requires Rust 1.70 or later.

```sh
git clone https://github.com/Blizzard-fs/lv
cd lv
cargo build --release
cp target/release/lv ~/.local/bin/
```

## Usage

```sh
# Auto-discover log files from current project
lv

# Auto-discover from a specific project directory
lv /path/to/project

# Open specific files directly
lv storage/logs/laravel.log
lv portal/application/Logs/runtime-errors.log portal/application/Logs/database-errors.log
```

When run without arguments, `lv` walks up from the current directory to find the project root (identified by `.git` or `composer.json`), then scans for log directories (`Logs/`, `logs/`, `log/`, `storage/logs/`). Vendor directories, node_modules, build artifacts, and cache directories are skipped.

## Key bindings

| Key | Action |
|-----|--------|
| `j` / `k` / arrows | Navigate entries |
| `J` / `K` | Scroll detail pane |
| `G` / `g` | Jump to bottom / top |
| `f` | Toggle follow mode |
| `p` | Toggle file picker sidebar |
| `Tab` | Switch focus between sidebar and log list |
| `Enter` | Open selected file (in sidebar) |
| Type | Filter entries by text |
| `Ctrl+U` | Clear filter |
| `Esc` / `q` | Quit (or close sidebar / clear filter) |

## Log format detection

Formats are detected by filename and content:

| Format | Detection |
|--------|-----------|
| SpolsMVC runtime | Filename contains `runtime`, or content has `[FILE]:` tag |
| SpolsMVC database | Filename contains `database`, or content has `[MESSAGE]:` tag |
| Laravel | Filename contains `laravel` or `app.log`, or timestamp format `[YYYY-MM-DD` |
| Generic | Fallback - displays lines as-is |

## License

MIT
