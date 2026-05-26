// lv - Log viewer for SpolsMVC and Laravel applications
// Usage: lv [project-dir|log-file ...]
//   No args: auto-discover log files from current directory
//   Dir arg: auto-discover from that directory
//   File args: open those files directly

use std::{fs, io, path::{Path, PathBuf}, time::SystemTime};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};

// ── Palette ───────────────────────────────────────────────────────────────────
const C_PURPLE: Color = Color::Rgb(147, 112, 219);
const C_ORANGE: Color = Color::Rgb(255, 160, 80);
const C_RED:    Color = Color::Rgb(220, 80, 80);
const C_BLUE:   Color = Color::Rgb(90, 140, 220);
const C_YELLOW: Color = Color::Rgb(220, 190, 60);
const C_GREEN:  Color = Color::Rgb(80, 200, 100);
const C_DIM:    Color = Color::Rgb(110, 110, 135);
const C_BORDER: Color = Color::Rgb(55, 50, 80);
const C_SEL_BG: Color = Color::Rgb(38, 33, 62);
const C_TEXT:   Color = Color::Rgb(215, 215, 230);
const C_BG_LOW: Color = Color::Rgb(18, 16, 32);

// ── Level ─────────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq)]
enum Level { Error, Warning, Info, Debug, Unknown }

impl Level {
    fn badge(&self) -> &'static str {
        match self {
            Self::Error   => "ERR",
            Self::Warning => "WRN",
            Self::Info    => "INF",
            Self::Debug   => "DBG",
            Self::Unknown => "LOG",
        }
    }

    fn color(&self) -> Color {
        match self {
            Self::Error   => C_RED,
            Self::Warning => C_ORANGE,
            Self::Info    => C_BLUE,
            Self::Debug   => C_DIM,
            Self::Unknown => C_DIM,
        }
    }

    fn from_str(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "ERROR" | "CRITICAL" | "ALERT" | "EMERGENCY" => Self::Error,
            "WARNING" | "WARN"                            => Self::Warning,
            "INFO" | "NOTICE"                             => Self::Info,
            "DEBUG"                                       => Self::Debug,
            _                                             => Self::Unknown,
        }
    }
}

// ── LogEntry ──────────────────────────────────────────────────────────────────
#[derive(Debug, Clone)]
struct LogEntry {
    timestamp: String,
    level:     Level,
    message:   String,
    detail:    String,
    short_src: Option<String>,
    count:     u32,
}

// ── Format detection ──────────────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq)]
enum LogFormat {
    SpolsRuntime,
    SpolsDatabase,
    Laravel,
    Generic,
}

fn detect_format(path: &Path) -> LogFormat {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.contains("runtime")  { return LogFormat::SpolsRuntime;  }
    if name.contains("database") { return LogFormat::SpolsDatabase; }
    if name.contains("laravel") || name == "app.log" { return LogFormat::Laravel; }

    let content = fs::read_to_string(path).unwrap_or_default();
    let first   = content.lines().next().unwrap_or("");
    if first.starts_with('[') && first.len() > 10 {
        if first.chars().nth(3) == Some('-') && first.chars().nth(6) == Some('-') {
            return if first.contains("[FILE]:")    { LogFormat::SpolsRuntime  }
                   else                            { LogFormat::SpolsDatabase };
        }
        if first.chars().nth(5) == Some('-') && first.chars().nth(8) == Some('-') {
            return LogFormat::Laravel;
        }
    }
    LogFormat::Generic
}

// ── Parsers ───────────────────────────────────────────────────────────────────
fn parse_all(content: &str, fmt: &LogFormat) -> Vec<LogEntry> {
    match fmt {
        LogFormat::SpolsRuntime  => parse_spols_runtime(content),
        LogFormat::SpolsDatabase => parse_spols_database(content),
        LogFormat::Laravel       => parse_laravel(content),
        LogFormat::Generic       => parse_generic(content),
    }
}

fn peel_ts(line: &str) -> (&str, &str) {
    if line.starts_with('[') {
        if let Some(end) = line.find(']') {
            return (&line[1..end], line[end + 1..].trim_start());
        }
    }
    ("", line)
}

fn push_or_dedup(entries: &mut Vec<LogEntry>, e: LogEntry) {
    if let Some(last) = entries.last_mut() {
        if last.message == e.message && last.level == e.level {
            last.count    += 1;
            last.timestamp = e.timestamp;
            return;
        }
    }
    entries.push(e);
}

fn short_source_from_path(path: &str) -> Option<String> {
    if path.is_empty() { return None; }
    let fname = path.split('/').last()?;
    Some(fname.replace(" (line:", ":").replace(')', ""))
}

fn classify_spols_message(msg: &str) -> Level {
    let m = msg.to_lowercase();
    if m.contains("exception") || m.contains("uncaught") || m.contains("fatal")
        || (m.contains("error") && !m.contains("deprecated"))
    {
        Level::Error
    } else if m.contains("deprecated") || m.contains("warning") {
        Level::Warning
    } else {
        Level::Unknown
    }
}

fn parse_spols_runtime(content: &str) -> Vec<LogEntry> {
    let mut entries = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        let (ts, rest) = peel_ts(line);
        let (source, message) = if let Some(fi) = rest.find("[FILE]:") {
            let after = rest[fi + 7..].trim_start();
            if let Some(mi) = after.find("[MESSAGE]:") {
                (after[..mi].trim().to_string(), after[mi + 10..].trim().to_string())
            } else {
                (String::new(), after.to_string())
            }
        } else if let Some(mi) = rest.find("[MESSAGE]:") {
            (String::new(), rest[mi + 10..].trim().to_string())
        } else {
            (String::new(), rest.to_string())
        };
        let short_src = short_source_from_path(&source);
        let level     = classify_spols_message(&message);
        push_or_dedup(&mut entries, LogEntry {
            timestamp: ts.to_string(), level, message, detail: source, short_src, count: 1,
        });
    }
    entries
}

fn parse_spols_database(content: &str) -> Vec<LogEntry> {
    let mut entries = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }
        let (ts, rest) = peel_ts(line);
        let (message, sql) = if let Some(mi) = rest.find("[MESSAGE]:") {
            let after = rest[mi + 10..].trim_start();
            if let Some(si) = after.find("[SQL]:") {
                (after[..si].trim().to_string(), after[si + 6..].trim().to_string())
            } else {
                (after.trim().to_string(), String::new())
            }
        } else {
            (rest.trim().to_string(), String::new())
        };
        let detail = if sql.is_empty() { String::new() } else { format!("SQL:\n{}", sql) };
        push_or_dedup(&mut entries, LogEntry {
            timestamp: ts.to_string(), level: Level::Error, message, detail, short_src: None, count: 1,
        });
    }
    entries
}

fn is_laravel_header(line: &str) -> bool {
    if !line.starts_with('[') { return false; }
    if let Some(end) = line.find(']') {
        let inner = &line[1..end];
        inner.len() >= 19
            && inner.chars().nth(4)  == Some('-')
            && inner.chars().nth(7)  == Some('-')
            && inner.chars().nth(10) == Some(' ')
    } else {
        false
    }
}

fn parse_laravel(content: &str) -> Vec<LogEntry> {
    let mut entries: Vec<LogEntry> = Vec::new();
    let mut pending: Option<(String, Level, String, Vec<String>)> = None;

    for line in content.lines() {
        if is_laravel_header(line) {
            if let Some((ts, lvl, msg, dl)) = pending.take() {
                push_or_dedup(&mut entries, LogEntry {
                    timestamp: ts, level: lvl, message: msg,
                    detail: dl.join("\n"), short_src: None, count: 1,
                });
            }
            let (ts, rest) = peel_ts(line);
            let (level, message) = if let Some(colon) = rest.find(':') {
                let ch  = &rest[..colon];
                let msg = rest[colon + 1..].trim_start();
                let lvl = ch.rfind('.').map(|d| Level::from_str(&ch[d + 1..])).unwrap_or_else(|| Level::from_str(ch));
                let msg = msg.find(" {\"").map(|i| msg[..i].trim()).unwrap_or_else(|| msg.trim()).to_string();
                (lvl, msg)
            } else {
                (Level::Unknown, rest.trim().to_string())
            };
            pending = Some((ts.to_string(), level, message, Vec::new()));
        } else if let Some((_, _, _, ref mut dl)) = pending {
            dl.push(line.to_string());
        }
    }
    if let Some((ts, lvl, msg, dl)) = pending {
        push_or_dedup(&mut entries, LogEntry {
            timestamp: ts, level: lvl, message: msg,
            detail: dl.join("\n"), short_src: None, count: 1,
        });
    }
    entries
}

fn parse_generic(content: &str) -> Vec<LogEntry> {
    content.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| LogEntry {
            timestamp: String::new(), level: Level::Unknown, message: l.to_string(),
            detail: String::new(), short_src: None, count: 1,
        })
        .collect()
}

// ── LogFile ───────────────────────────────────────────────────────────────────
struct LogFile {
    path:         PathBuf,
    display_name: String,   // Relative path from project root (or filename)
    format:       LogFormat,
    entries:      Vec<LogEntry>,
    mtime:        Option<SystemTime>,
}

impl LogFile {
    fn from_path(path: PathBuf, display_name: String) -> Self {
        let format  = detect_format(&path);
        let content = fs::read_to_string(&path).unwrap_or_default();
        let entries = parse_all(&content, &format);
        let mtime   = fs::metadata(&path).ok().and_then(|m| m.modified().ok());
        LogFile { path, display_name, format, entries, mtime }
    }

    fn format_label(&self) -> &'static str {
        match self.format {
            LogFormat::SpolsRuntime  => "spols/runtime",
            LogFormat::SpolsDatabase => "spols/db",
            LogFormat::Laravel       => "laravel",
            LogFormat::Generic       => "generic",
        }
    }

    fn reload_if_changed(&mut self) -> bool {
        let new_mtime = fs::metadata(&self.path).ok().and_then(|m| m.modified().ok());
        if new_mtime != self.mtime {
            let content  = fs::read_to_string(&self.path).unwrap_or_default();
            self.entries = parse_all(&content, &self.format);
            self.mtime   = new_mtime;
            true
        } else {
            false
        }
    }
}

// ── Discovery ─────────────────────────────────────────────────────────────────
const SKIP_DIRS: &[&str] = &[
    "vendor", "node_modules", "target", ".git", "cache",
    ".promptgoblin", "bower_components", "docker",
];

fn find_project_root(start: &Path) -> PathBuf {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".git").exists()
            || dir.join("composer.json").exists()
            || dir.join("Cargo.toml").exists()
        {
            return dir;
        }
        if !dir.pop() { break; }
    }
    start.to_path_buf()
}

fn is_log_dir(name: &str) -> bool {
    matches!(name, "Logs" | "logs" | "log")
}

fn collect_direct_logs(dir: &Path, root: &Path, found: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("log") {
            let rel = path.strip_prefix(root)
                .map(|r| r.display().to_string())
                .unwrap_or_else(|_| path.display().to_string());
            found.push((path, rel));
        }
    }
}

fn scan_for_logs(dir: &Path, root: &Path, found: &mut Vec<(PathBuf, String)>, depth: usize) {
    if depth > 7 { return; }

    let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if SKIP_DIRS.contains(&name) { return; }

    if is_log_dir(name) && depth > 0 {
        // Only collect direct children, not sub-subdirectories (skips rotation folders)
        collect_direct_logs(dir, root, found);
        return;
    }

    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_for_logs(&path, root, found, depth + 1);
        }
    }
}

fn discover_logs(hint: &Path) -> Vec<LogFile> {
    let root = find_project_root(hint);
    let mut found: Vec<(PathBuf, String)> = Vec::new();
    scan_for_logs(&root, &root, &mut found, 0);
    found.sort_by(|a, b| a.1.cmp(&b.1));
    found.dedup_by(|a, b| a.0 == b.0);
    found.into_iter().map(|(p, name)| LogFile::from_path(p, name)).collect()
}

// ── App ───────────────────────────────────────────────────────────────────────
#[derive(PartialEq)]
enum Focus { List, Sidebar }

struct App {
    files:         Vec<LogFile>,
    file_idx:      usize,
    filtered:      Vec<usize>,
    list_state:    ListState,
    search:        String,
    follow:        bool,
    detail_scroll: u16,
    // Sidebar (file picker)
    sidebar_open:  bool,
    sidebar_state: ListState,
    focus:         Focus,
}

impl App {
    fn new(files: Vec<LogFile>) -> Self {
        let sidebar_open = files.len() > 1;
        let mut sidebar_state = ListState::default();
        sidebar_state.select(Some(0));

        let mut app = App {
            files,
            file_idx:      0,
            filtered:      Vec::new(),
            list_state:    ListState::default(),
            search:        String::new(),
            follow:        false,
            detail_scroll: 0,
            sidebar_open,
            sidebar_state,
            focus:         Focus::List,
        };
        app.apply_filter();
        app.scroll_to_bottom();
        app
    }

    fn current_file(&self) -> &LogFile { &self.files[self.file_idx] }

    fn selected_entry(&self) -> Option<&LogEntry> {
        self.list_state.selected()
            .and_then(|i| self.filtered.get(i))
            .map(|&ei| &self.files[self.file_idx].entries[ei])
    }

    fn apply_filter(&mut self) {
        let entries = &self.files[self.file_idx].entries;
        let q = self.search.to_lowercase();
        self.filtered = (0..entries.len())
            .filter(|&i| {
                if q.is_empty() { return true; }
                let e = &entries[i];
                e.message.to_lowercase().contains(&q)
                    || e.detail.to_lowercase().contains(&q)
                    || e.timestamp.contains(&q)
            })
            .collect();
        let max = self.filtered.len().saturating_sub(1);
        match self.list_state.selected() {
            Some(i) if i > max => {
                self.list_state.select(if self.filtered.is_empty() { None } else { Some(max) });
            }
            None if !self.filtered.is_empty() => self.list_state.select(Some(0)),
            _ => {}
        }
        self.detail_scroll = 0;
    }

    fn switch_to_file(&mut self, idx: usize) {
        if idx >= self.files.len() { return; }
        self.file_idx = idx;
        self.search.clear();
        self.apply_filter();
        self.scroll_to_bottom();
        self.sidebar_state.select(Some(idx));
    }

    fn move_down(&mut self) {
        if self.filtered.is_empty() { return; }
        let next = match self.list_state.selected() {
            Some(i) => (i + 1).min(self.filtered.len() - 1),
            None    => 0,
        };
        self.list_state.select(Some(next));
        self.detail_scroll = 0;
    }

    fn move_up(&mut self) {
        if self.filtered.is_empty() { return; }
        let prev = match self.list_state.selected() {
            Some(0) | None => 0,
            Some(i)        => i - 1,
        };
        self.list_state.select(Some(prev));
        self.detail_scroll = 0;
    }

    fn scroll_to_bottom(&mut self) {
        if !self.filtered.is_empty() {
            self.list_state.select(Some(self.filtered.len() - 1));
            self.detail_scroll = 0;
        }
    }

    fn scroll_to_top(&mut self) {
        if !self.filtered.is_empty() {
            self.list_state.select(Some(0));
            self.detail_scroll = 0;
        }
    }

    fn detail_down(&mut self) { self.detail_scroll = self.detail_scroll.saturating_add(4); }
    fn detail_up(&mut self)   { self.detail_scroll = self.detail_scroll.saturating_sub(4); }

    fn sidebar_down(&mut self) {
        let n = self.files.len();
        if n == 0 { return; }
        let next = match self.sidebar_state.selected() {
            Some(i) => (i + 1).min(n - 1),
            None    => 0,
        };
        self.sidebar_state.select(Some(next));
    }

    fn sidebar_up(&mut self) {
        let n = self.files.len();
        if n == 0 { return; }
        let prev = match self.sidebar_state.selected() {
            Some(0) | None => 0,
            Some(i)        => i - 1,
        };
        self.sidebar_state.select(Some(prev));
    }

    fn sidebar_confirm(&mut self) {
        if let Some(idx) = self.sidebar_state.selected() {
            self.switch_to_file(idx);
        }
        self.focus = Focus::List;
    }

    fn toggle_sidebar(&mut self) {
        match (self.sidebar_open, &self.focus) {
            // Closed - open and focus sidebar
            (false, _) => {
                self.sidebar_open = true;
                self.sidebar_state.select(Some(self.file_idx));
                self.focus = Focus::Sidebar;
            }
            // Open, focus on list - move focus to sidebar without closing
            (true, Focus::List) => {
                self.sidebar_state.select(Some(self.file_idx));
                self.focus = Focus::Sidebar;
            }
            // Open, focus on sidebar - close it
            (true, Focus::Sidebar) => {
                self.sidebar_open = false;
                self.focus = Focus::List;
            }
        }
    }

    fn toggle_focus(&mut self) {
        if !self.sidebar_open { return; }
        self.focus = match self.focus {
            Focus::List    => Focus::Sidebar,
            Focus::Sidebar => Focus::List,
        };
        if self.focus == Focus::Sidebar {
            self.sidebar_state.select(Some(self.file_idx));
        }
    }

    fn toggle_follow(&mut self) {
        self.follow = !self.follow;
        if self.follow { self.scroll_to_bottom(); }
    }

    fn tick(&mut self) {
        if self.files[self.file_idx].reload_if_changed() {
            let prev = self.list_state.selected();
            self.apply_filter();
            if self.follow {
                self.scroll_to_bottom();
            } else if let Some(sel) = prev {
                let max = self.filtered.len().saturating_sub(1);
                self.list_state.select(Some(sel.min(max)));
            }
        }
    }
}

// ── UI ────────────────────────────────────────────────────────────────────────
const SIDEBAR_W: u16 = 28;

fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let outer = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    let body = if app.sidebar_open && app.files.len() > 1 {
        let cols = Layout::horizontal([
            Constraint::Length(SIDEBAR_W),
            Constraint::Min(0),
        ])
        .split(outer[1]);
        draw_sidebar(frame, app, cols[0]);
        cols[1]
    } else {
        outer[1]
    };

    let mid = Layout::vertical([
        Constraint::Percentage(42),
        Constraint::Percentage(58),
    ])
    .split(body);

    draw_search(frame, app, outer[0]);
    draw_list(frame, app, mid[0]);
    draw_detail(frame, app, mid[1]);
    draw_statusbar(frame, app, outer[2]);
}

fn draw_sidebar(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Sidebar;
    let border_color = if focused { C_PURPLE } else { C_BORDER };

    let items: Vec<ListItem> = app.files.iter().enumerate().map(|(i, f)| {
        let is_active = i == app.file_idx;

        // Show last 2 path components, truncated to fit
        let name = {
            let parts: Vec<&str> = f.display_name.split('/').collect();
            let label = match parts.len() {
                0 | 1 => f.display_name.clone(),
                2     => f.display_name.clone(),
                n     => format!("{}/{}", parts[n - 2], parts[n - 1]),
            };
            let max = (area.width as usize).saturating_sub(4);
            if label.len() > max && max > 2 {
                format!("{}…", &label[..max.saturating_sub(1)])
            } else {
                label
            }
        };

        let count_str = format!(" {}", app.files[i].entries.len());

        let (name_style, count_color) = if is_active {
            (Style::default().fg(C_ORANGE).add_modifier(Modifier::BOLD), C_YELLOW)
        } else {
            (Style::default().fg(C_TEXT), C_DIM)
        };

        ListItem::new(Line::from(vec![
            Span::styled(name, name_style),
            Span::styled(count_str, Style::default().fg(count_color)),
        ]))
    }).collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(border_color))
                .title(Span::styled(" files ", Style::default().fg(C_DIM))),
        )
        .highlight_style(Style::default().bg(C_SEL_BG).add_modifier(Modifier::BOLD))
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, area, &mut app.sidebar_state);
}

fn draw_search(frame: &mut Frame, app: &App, area: Rect) {
    let file         = app.current_file();
    let list_focused = app.focus == Focus::List;
    let border_color = if list_focused { C_PURPLE } else { C_BORDER };

    let mut title_spans = vec![
        Span::raw(" "),
        Span::styled(file.display_name.clone(), Style::default().fg(C_ORANGE).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(file.format_label(), Style::default().fg(C_DIM)),
    ];
    if app.follow {
        title_spans.push(Span::raw("  "));
        title_spans.push(Span::styled(
            " FOLLOW ",
            Style::default().fg(C_BG_LOW).bg(C_GREEN).add_modifier(Modifier::BOLD),
        ));
    }
    title_spans.push(Span::raw(" "));

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(Line::from(title_spans));

    let cursor = if (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_millis() / 500) % 2 == 0 { "▌" } else { " " };

    let inner = block.inner(area);
    frame.render_widget(block, area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("/ ", Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD)),
            Span::styled(app.search.clone(), Style::default().fg(C_TEXT)),
            Span::styled(cursor, Style::default().fg(C_PURPLE)),
        ])),
        inner,
    );
}

fn draw_list(frame: &mut Frame, app: &mut App, area: Rect) {
    let file   = app.current_file();
    let total  = file.entries.len();
    let shown  = app.filtered.len();
    let label  = if shown == total {
        format!(" {} entries ", total)
    } else {
        format!(" {}/{} ", shown, total)
    };

    let items: Vec<ListItem> = app.filtered.iter().map(|&ei| {
        let e = &file.entries[ei];

        let badge = Span::styled(
            format!(" {} ", e.level.badge()),
            Style::default().fg(C_BG_LOW).bg(e.level.color()).add_modifier(Modifier::BOLD),
        );
        let ts = Span::styled(
            format!(" {:>19} ", e.timestamp),
            Style::default().fg(C_DIM),
        );

        let src_w = e.short_src.as_deref().map(|s| s.len() + 2).unwrap_or(0);
        let cnt_w = if e.count > 1 { format!(" ×{}", e.count).len() } else { 0 };
        let avail = (area.width as usize).saturating_sub(5 + 21 + src_w + cnt_w + 2);

        let msg = if e.message.len() > avail && avail > 3 {
            format!("{}…", &e.message[..avail.saturating_sub(1)])
        } else {
            e.message.clone()
        };

        let mut spans = vec![badge, ts, Span::styled(msg, Style::default().fg(C_TEXT))];

        if let Some(src) = &e.short_src {
            spans.push(Span::styled(format!("  {}", src), Style::default().fg(C_DIM)));
        }
        if e.count > 1 {
            spans.push(Span::styled(
                format!(" ×{}", e.count),
                Style::default().fg(C_YELLOW).add_modifier(Modifier::BOLD),
            ));
        }

        ListItem::new(Line::from(spans))
    }).collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(C_BORDER))
                .title(Span::styled(label, Style::default().fg(C_DIM))),
        )
        .highlight_style(Style::default().bg(C_SEL_BG).add_modifier(Modifier::BOLD))
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

fn draw_detail(frame: &mut Frame, app: &App, area: Rect) {
    let Some(entry) = app.selected_entry() else {
        frame.render_widget(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(C_BORDER))
                .title(Span::styled(" no selection ", Style::default().fg(C_DIM))),
            area,
        );
        return;
    };

    let title = if entry.count > 1 {
        format!(" {}  ×{} occurrences ", entry.timestamp, entry.count)
    } else {
        format!(" {} ", entry.timestamp)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_BORDER))
        .title(Span::styled(title, Style::default().fg(C_DIM)));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        entry.message.clone(),
        Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
    )));

    if !entry.detail.is_empty() {
        lines.push(Line::from(""));
        for line in entry.detail.lines() {
            let style = if line.trim_start().starts_with('#') {
                Style::default().fg(C_DIM)
            } else if line == "[stacktrace]" || line.starts_with("[previous exception]") {
                Style::default().fg(C_DIM).add_modifier(Modifier::ITALIC)
            } else if line == "SQL:" {
                Style::default().fg(C_YELLOW).add_modifier(Modifier::BOLD)
            } else if line.contains("Exception") || line.contains("Error") {
                Style::default().fg(C_RED)
            } else {
                Style::default().fg(C_TEXT)
            };
            lines.push(Line::from(Span::styled(line, style)));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .scroll((app.detail_scroll, 0))
            .wrap(Wrap { trim: false }),
        inner,
    );
}

fn draw_statusbar(frame: &mut Frame, app: &App, area: Rect) {
    let spans = if app.focus == Focus::Sidebar {
        vec![
            Span::styled("  ↑↓/jk", Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD)),
            Span::styled(" select file  ", Style::default().fg(C_DIM)),
            Span::styled("enter", Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD)),
            Span::styled(" open  ", Style::default().fg(C_DIM)),
            Span::styled("tab", Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD)),
            Span::styled(" focus list  ", Style::default().fg(C_DIM)),
            Span::styled("p/esc", Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD)),
            Span::styled(" close picker", Style::default().fg(C_DIM)),
        ]
    } else {
        let follow_label = if app.follow { "f off  " } else { "f follow  " };
        let picker_hint  = if app.files.len() > 1 && app.sidebar_open { "p/tab sidebar  " }
                           else if app.files.len() > 1                 { "p files  " }
                           else                                         { "" };
        vec![
            Span::styled("  ↑↓/jk", Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD)),
            Span::styled(" list  ", Style::default().fg(C_DIM)),
            Span::styled("J/K", Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD)),
            Span::styled(" detail  ", Style::default().fg(C_DIM)),
            Span::styled("G/g", Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD)),
            Span::styled(" bottom/top  ", Style::default().fg(C_DIM)),
            Span::styled("f", Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {}", follow_label), Style::default().fg(C_DIM)),
            Span::styled(picker_hint, Style::default().fg(C_DIM)),
            Span::styled("q", Style::default().fg(C_PURPLE).add_modifier(Modifier::BOLD)),
            Span::styled(" quit", Style::default().fg(C_DIM)),
        ]
    };

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(C_BG_LOW)),
        area,
    );
}

// ── Main loop ─────────────────────────────────────────────────────────────────
fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        if event::poll(std::time::Duration::from_millis(500))? {
            if let Event::Key(key) = event::read()? {

                // Sidebar-focused key handling
                if app.focus == Focus::Sidebar {
                    match (key.code, key.modifiers) {
                        (KeyCode::Esc, _) | (KeyCode::Char('p'), KeyModifiers::NONE) => {
                            app.sidebar_open = false;
                            app.focus = Focus::List;
                        }
                        (KeyCode::Tab, _) => app.toggle_focus(),
                        (KeyCode::Enter, _) => app.sidebar_confirm(),
                        (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => app.sidebar_down(),
                        (KeyCode::Up,   _) | (KeyCode::Char('k'), KeyModifiers::NONE) => app.sidebar_up(),
                        _ => {}
                    }
                    continue;
                }

                // List-focused key handling
                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), KeyModifiers::NONE)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(()),

                    (KeyCode::Esc, _) => {
                        if app.sidebar_open {
                            app.sidebar_open = false;
                            app.focus = Focus::List;
                        } else if !app.search.is_empty() {
                            app.search.clear();
                            app.apply_filter();
                        } else {
                            return Ok(());
                        }
                    }

                    (KeyCode::Char('p'), KeyModifiers::NONE) => app.toggle_sidebar(),
                    (KeyCode::Tab, _) if app.sidebar_open    => app.toggle_focus(),

                    // List navigation
                    (KeyCode::Down, _) | (KeyCode::Char('j'), KeyModifiers::NONE) => app.move_down(),
                    (KeyCode::Up,   _) | (KeyCode::Char('k'), KeyModifiers::NONE) => app.move_up(),

                    // Detail scroll (Shift+J/K)
                    (KeyCode::Char('J'), _) => app.detail_down(),
                    (KeyCode::Char('K'), _) => app.detail_up(),

                    // Goto
                    (KeyCode::Char('G'), _) | (KeyCode::End,  _) => app.scroll_to_bottom(),
                    (KeyCode::Char('g'), KeyModifiers::NONE) | (KeyCode::Home, _) => app.scroll_to_top(),

                    // Follow mode
                    (KeyCode::Char('f'), KeyModifiers::NONE) => app.toggle_follow(),

                    // Search editing
                    (KeyCode::Backspace, _) => {
                        app.search.pop();
                        app.apply_filter();
                    }
                    (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                        app.search.clear();
                        app.apply_filter();
                    }

                    // Search input (catch-all)
                    (KeyCode::Char(c), KeyModifiers::NONE)
                    | (KeyCode::Char(c), KeyModifiers::SHIFT) => {
                        app.search.push(c);
                        app.apply_filter();
                    }

                    _ => {}
                }
            }
        } else {
            app.tick();
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let files: Vec<LogFile> = if args.is_empty() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let found = discover_logs(&cwd);
        if found.is_empty() {
            eprintln!("lv: no log files found (searched from {})", cwd.display());
            eprintln!("hint: run from a project root, or pass files explicitly");
            std::process::exit(1);
        }
        found
    } else if args.len() == 1 && PathBuf::from(&args[0]).is_dir() {
        let dir = PathBuf::from(&args[0]);
        let found = discover_logs(&dir);
        if found.is_empty() {
            eprintln!("lv: no log files found under {}", dir.display());
            std::process::exit(1);
        }
        found
    } else {
        args.into_iter()
            .map(PathBuf::from)
            .filter(|p| {
                if !p.exists() { eprintln!("lv: not found: {}", p.display()); false } else { true }
            })
            .map(|p| {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();
                LogFile::from_path(p, name)
            })
            .collect()
    };

    if files.is_empty() {
        eprintln!("lv: no valid files");
        std::process::exit(1);
    }

    enable_raw_mode().expect("enable raw mode");
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).expect("alternate screen");

    let backend      = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).expect("terminal");
    let mut app      = App::new(files);

    let result = run(&mut terminal, &mut app);

    disable_raw_mode().expect("disable raw mode");
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture).expect("leave");
    terminal.show_cursor().expect("show cursor");

    if let Err(e) = result {
        eprintln!("lv: {}", e);
        std::process::exit(1);
    }
}
