//! First-run installer wizard — ratatui TUI.
//!
//! Collects setup options from the user, then exits cleanly so
//! `setup::apply()` can run in normal terminal mode.

use crate::{
    obs_profile,
    setup::{ObsChoice, SetupOptions},
};
use crossterm::{
    event::{
        self, Event, KeyCode, KeyEventKind,
        MouseButton, MouseEventKind, EnableMouseCapture, DisableMouseCapture,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use std::io;

// ── Step definitions ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Welcome,
    ObsChoice,
    OutputDir,
    BufferDuration,
    CaptureSource,
    Autostart,
    Confirm,
}

impl Step {
    const ALL_VISIBLE: &'static [Step] = &[
        Step::ObsChoice,
        Step::OutputDir,
        Step::BufferDuration,
        Step::CaptureSource,
        Step::Autostart,
        Step::Confirm,
    ];

    fn label(self) -> &'static str {
        match self {
            Step::Welcome        => "Welcome",
            Step::ObsChoice      => "OBS",
            Step::OutputDir      => "Output",
            Step::BufferDuration => "Buffer",
            Step::CaptureSource  => "Capture",
            Step::Autostart      => "Autostart",
            Step::Confirm        => "Confirm",
        }
    }
}

// ── State ─────────────────────────────────────────────────────────────────────

/// Clickable areas stored after each draw.
#[derive(Default)]
struct HitRects {
    /// Radio option rows for the current step (up to 4).
    radio: [Option<Rect>; 4],
    /// "Next" / confirm area.
    next: Option<Rect>,
}

struct State {
    step: Step,
    // OBS
    obs_radio: usize,       // 0=download, 1=existing, 2=skip
    obs_detected: Option<String>,
    // Output dir
    output_dir: String,
    // Buffer
    buffer_secs: u32,
    // Capture
    capture_radio: usize,   // 0=game, 1=display
    // Autostart
    autostart: bool,
    // Text cursor position for text fields
    cursor: usize,
    // Error message shown on Confirm step
    error: Option<String>,
    // Hit rects for mouse support
    hit: HitRects,
}

impl State {
    fn new() -> Self {
        let detected = obs_profile::detect_obs()
            .map(|p| p.to_string_lossy().into_owned());

        // Default obs_radio: 1 (existing) if detected, else 0 (download)
        let obs_radio = if detected.is_some() { 1 } else { 0 };

        let output_dir = dirs::video_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("Captures")
            .to_string_lossy()
            .into_owned();

        Self {
            step: Step::Welcome,
            obs_radio,
            obs_detected: detected,
            output_dir,
            buffer_secs: 30,
            capture_radio: 0,
            autostart: true,
            cursor: 0,
            error: None,
            hit: HitRects::default(),
        }
    }

    fn next_step(&mut self) {
        self.step = match self.step {
            Step::Welcome        => Step::ObsChoice,
            Step::ObsChoice      => Step::OutputDir,
            Step::OutputDir      => Step::BufferDuration,
            Step::BufferDuration => Step::CaptureSource,
            Step::CaptureSource  => Step::Autostart,
            Step::Autostart      => Step::Confirm,
            Step::Confirm        => Step::Confirm, // handled by run()
        };
        self.cursor = 0;
        self.error = None;
    }

    fn prev_step(&mut self) {
        self.step = match self.step {
            Step::Welcome        => Step::Welcome,
            Step::ObsChoice      => Step::Welcome,
            Step::OutputDir      => Step::ObsChoice,
            Step::BufferDuration => Step::OutputDir,
            Step::CaptureSource  => Step::BufferDuration,
            Step::Autostart      => Step::CaptureSource,
            Step::Confirm        => Step::Autostart,
        };
        self.cursor = 0;
        self.error = None;
    }

    fn to_options(&self) -> SetupOptions {
        let obs = match self.obs_radio {
            1 => ObsChoice::Existing,
            2 => ObsChoice::Skip,
            _ => ObsChoice::Download,
        };
        SetupOptions {
            obs,
            output_dir: self.output_dir.clone(),
            buffer_secs: self.buffer_secs,
            capture_source: if self.capture_radio == 1 { "display" } else { "game" }.into(),
            autostart: self.autostart,
        }
    }

    fn visible_steps(&self) -> Vec<Step> {
        vec![
            Step::ObsChoice, Step::OutputDir, Step::BufferDuration,
            Step::CaptureSource, Step::Autostart, Step::Confirm,
        ]
    }

    fn step_index(&self) -> (usize, usize) {
        let steps = self.visible_steps();
        let idx = steps.iter().position(|&s| s == self.step).unwrap_or(0);
        (idx + 1, steps.len())
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Run the wizard. Returns `Some(SetupOptions)` if the user completed it,
/// `None` if they quit early.
pub fn run() -> Option<SetupOptions> {
    enable_raw_mode().ok()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture).ok()?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).ok()?;

    let mut state = State::new();
    let result = event_loop(&mut terminal, &mut state);

    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture).ok();
    terminal.show_cursor().ok();

    result
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut State,
) -> Option<SetupOptions> {
    loop {
        terminal.draw(|f| draw(f, state)).ok()?;

        match event::read() {
            Ok(Event::Key(key)) => {
                if key.kind != KeyEventKind::Press { continue; }
                match handle_key(state, key.code) {
                    KeyAction::Next => state.next_step(),
                    KeyAction::Prev => state.prev_step(),
                    KeyAction::Quit => return None,
                    KeyAction::Done => return Some(state.to_options()),
                    KeyAction::None => {}
                }
            }
            Ok(Event::Mouse(m)) => {
                match handle_mouse(state, m) {
                    KeyAction::Next => state.next_step(),
                    KeyAction::Prev => state.prev_step(),
                    KeyAction::Done => return Some(state.to_options()),
                    KeyAction::Quit | KeyAction::None => {}
                }
            }
            _ => {}
        }
    }
}

// ── Mouse handling ────────────────────────────────────────────────────────────

fn handle_mouse(state: &mut State, m: crossterm::event::MouseEvent) -> KeyAction {
    if m.kind != MouseEventKind::Down(MouseButton::Left) {
        return KeyAction::None;
    }
    let (col, row) = (m.column, m.row);

    // Check radio options
    for (i, rect) in state.hit.radio.iter().enumerate() {
        if let Some(r) = rect {
            if rect_contains(*r, (col, row)) {
                match state.step {
                    Step::ObsChoice => {
                        let was = state.obs_radio;
                        state.obs_radio = i;
                        // Double-click same option (or single click when already selected) → advance
                        if was == i { return KeyAction::Next; }
                    }
                    Step::CaptureSource => {
                        let was = state.capture_radio;
                        state.capture_radio = i;
                        if was == i { return KeyAction::Next; }
                    }
                    Step::Autostart => {
                        state.autostart = i == 0;
                    }
                    _ => {}
                }
                return KeyAction::None;
            }
        }
    }

    // Check next/confirm area
    if let Some(r) = state.hit.next {
        if rect_contains(r, (col, row)) {
            return match state.step {
                Step::Confirm => KeyAction::Done,
                Step::Welcome => KeyAction::Next,
                _ => KeyAction::Next,
            };
        }
    }

    KeyAction::None
}

fn rect_contains(r: Rect, (col, row): (u16, u16)) -> bool {
    col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
}

// ── Key handling ──────────────────────────────────────────────────────────────

enum KeyAction { Next, Prev, Quit, Done, None }

fn handle_key(state: &mut State, code: KeyCode) -> KeyAction {
    match state.step {
        Step::Welcome => match code {
            KeyCode::Enter | KeyCode::Right => KeyAction::Next,
            KeyCode::Char('q') | KeyCode::Esc => KeyAction::Quit,
            _ => KeyAction::None,
        },

        Step::ObsChoice => match code {
            KeyCode::Up | KeyCode::Char('k')   => { if state.obs_radio > 0 { state.obs_radio -= 1; } KeyAction::None }
            KeyCode::Down | KeyCode::Char('j') => { if state.obs_radio < 2 { state.obs_radio += 1; } KeyAction::None }
            KeyCode::Enter | KeyCode::Right    => KeyAction::Next,
            KeyCode::Left  => KeyAction::Prev,
            KeyCode::Char('q') | KeyCode::Esc => KeyAction::Quit,
            _ => KeyAction::None,
        },

        Step::OutputDir => match code {
            KeyCode::Char(c) => { state.output_dir.insert(state.cursor, c); state.cursor += 1; KeyAction::None }
            KeyCode::Backspace => {
                if state.cursor > 0 {
                    state.cursor -= 1;
                    state.output_dir.remove(state.cursor);
                }
                KeyAction::None
            }
            KeyCode::Left  if state.cursor > 0 => { state.cursor -= 1; KeyAction::None }
            KeyCode::Right if state.cursor < state.output_dir.len() => { state.cursor += 1; KeyAction::None }
            KeyCode::Enter => KeyAction::Next,
            KeyCode::Up    => KeyAction::Prev,
            KeyCode::Char('q') | KeyCode::Esc => KeyAction::Quit,
            _ => KeyAction::None,
        },

        Step::BufferDuration => match code {
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Right => {
                state.buffer_secs = (state.buffer_secs + 15).min(600);
                KeyAction::None
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Left => {
                state.buffer_secs = state.buffer_secs.saturating_sub(15).max(15);
                KeyAction::None
            }
            KeyCode::Enter => KeyAction::Next,
            KeyCode::Char('q') | KeyCode::Esc => KeyAction::Quit,
            _ => KeyAction::None,
        },

        Step::CaptureSource => match code {
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j') => {
                state.capture_radio = 1 - state.capture_radio;
                KeyAction::None
            }
            KeyCode::Enter | KeyCode::Right => KeyAction::Next,
            KeyCode::Left  => KeyAction::Prev,
            KeyCode::Char('q') | KeyCode::Esc => KeyAction::Quit,
            _ => KeyAction::None,
        },

        Step::Autostart => match code {
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Down | KeyCode::Char('j')
            | KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => {
                state.autostart = !state.autostart;
                KeyAction::None
            }
            KeyCode::Enter => KeyAction::Next,
            KeyCode::Char('q') | KeyCode::Esc => KeyAction::Quit,
            _ => KeyAction::None,
        },

        Step::Confirm => match code {
            KeyCode::Enter => KeyAction::Done,
            KeyCode::Left | KeyCode::Char('b') => KeyAction::Prev,
            KeyCode::Char('q') | KeyCode::Esc => KeyAction::Quit,
            _ => KeyAction::None,
        },
    }
}

// ── Drawing ───────────────────────────────────────────────────────────────────

fn draw(f: &mut Frame, state: &mut State) {
    let area = f.area();

    // Outer chrome
    let outer = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(Span::styled(
            " rscapt — setup ",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    // Layout: step bar / content / help
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(inner);

    draw_step_bar(f, state, chunks[0]);
    draw_content(f, state, chunks[1]);
    draw_help(f, state, chunks[2]);
}

fn draw_step_bar(f: &mut Frame, state: &State, area: Rect) {
    if state.step == Step::Welcome {
        let p = Paragraph::new(
            Line::from(vec![
                Span::styled("  Welcome to ", Style::default().fg(Color::DarkGray)),
                Span::styled("rscapt", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            ])
        );
        f.render_widget(p, area);
        return;
    }

    let steps = state.visible_steps();
    let (cur_idx, _) = state.step_index();

    let mut spans: Vec<Span> = vec![Span::raw("  ")];
    for (i, step) in steps.iter().enumerate() {
        let n = i + 1;
        let is_current = *step == state.step;
        let is_done    = n < cur_idx;
        let style = if is_current {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else if is_done {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let marker = if is_done { "✓" } else { &n.to_string() };
        spans.push(Span::styled(format!("{marker} {}", step.label()), style));
        if i < steps.len() - 1 {
            spans.push(Span::styled("  ─  ", Style::default().fg(Color::DarkGray)));
        }
    }

    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_content(f: &mut Frame, state: &mut State, area: Rect) {
    // Vertical centering: push content to middle
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(15), Constraint::Min(1), Constraint::Percentage(15)])
        .split(area);
    let content_area = chunks[1];

    // Clear hit rects each frame
    state.hit.radio = [None; 4];
    state.hit.next  = None;

    match state.step {
        Step::Welcome        => draw_welcome(f, state, content_area),
        Step::ObsChoice      => draw_obs_choice(f, state, content_area),
        Step::OutputDir      => draw_output_dir(f, state, content_area),
        Step::BufferDuration => draw_buffer(f, state, content_area),
        Step::CaptureSource  => draw_capture(f, state, content_area),
        Step::Autostart      => draw_autostart(f, state, content_area),
        Step::Confirm        => draw_confirm(f, state, content_area),
    }
}

fn draw_welcome(f: &mut Frame, state: &mut State, area: Rect) {
    let lines = vec![
        Line::from(Span::styled(
            "  rscapt",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("  OBS replay buffer processor — upscale, post-process,"),
        Line::from("  compress, and share your game clips automatically."),
        Line::from(""),
        Line::from(Span::styled(
            "  This wizard will get you set up in about a minute.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  [ Begin → ]",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        )),
    ];
    // Store the "Begin" line as a next hit rect
    let begin_row = area.y + 7;
    if begin_row < area.y + area.height {
        state.hit.next = Some(Rect { x: area.x, y: begin_row, width: area.width, height: 1 });
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_obs_choice(f: &mut Frame, state: &mut State, area: Rect) {
    let detected_line = match &state.obs_detected {
        Some(p) => format!("  OBS detected: {p}"),
        None    => "  OBS not found in standard locations.".into(),
    };

    let radio = |i: usize, label: &str, sub: &str| {
        let selected = state.obs_radio == i;
        let dot = if selected { "●" } else { "○" };
        let style = if selected { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) }
                    else        { Style::default().fg(Color::White) };
        vec![
            Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{dot}  {label}"), style),
            ]),
            Line::from(Span::styled(format!("      {sub}"), Style::default().fg(Color::DarkGray))),
            Line::from(""),
        ]
    };

    // Radio options start at row offset 4 (header + blank + question + blank)
    let radio_start = area.y + 4;
    for i in 0..3usize {
        let y = radio_start + (i as u16) * 3;
        if y < area.y + area.height {
            state.hit.radio[i] = Some(Rect { x: area.x, y, width: area.width, height: 2 });
        }
    }

    let existing_sub = match &state.obs_detected {
        Some(_) => "WebSocket port and password will be read from your OBS config automatically",
        None    => "OBS not found — will use default port 4455 with no auth",
    };

    let mut lines = vec![
        Line::from(Span::styled(&detected_line, Style::default().fg(Color::DarkGray))),
        Line::from(""),
        Line::from("  How should rscapt use OBS?"),
        Line::from(""),
    ];
    lines.extend(radio(0, "Download OBS automatically", "~250 MB, placed in %LOCALAPPDATA%\\rscapt\\obs"));
    lines.extend(radio(1, "I already have OBS installed", existing_sub));
    lines.extend(radio(2, "Skip for now", "set obs_exe_path in config.json later"));

    f.render_widget(Paragraph::new(lines), area);
}

fn draw_output_dir(f: &mut Frame, state: &mut State, area: Rect) {
    let display = format!("{}_", &state.output_dir);
    let lines = vec![
        Line::from("  Clip output folder"),
        Line::from(""),
        Line::from(Span::styled("  Upscaled clips will be saved here.",
            Style::default().fg(Color::DarkGray))),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(display, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_buffer(f: &mut Frame, state: &mut State, area: Rect) {
    let secs = state.buffer_secs;
    let lines = vec![
        Line::from("  Replay buffer duration"),
        Line::from(""),
        Line::from(Span::styled(
            "  How many seconds to keep in the replay buffer.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  ◄  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{secs} seconds"),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ►", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!("  ↑/↓ or ◄/► to adjust in 15-second steps (15–600 s)"),
            Style::default().fg(Color::DarkGray),
        )),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_capture(f: &mut Frame, state: &mut State, area: Rect) {
    let radio = |i: usize, label: &str, sub: &str| {
        let selected = state.capture_radio == i;
        let dot = if selected { "●" } else { "○" };
        let style = if selected { Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD) }
                    else        { Style::default().fg(Color::White) };
        vec![
            Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{dot}  {label}"), style),
            ]),
            Line::from(Span::styled(format!("      {sub}"), Style::default().fg(Color::DarkGray))),
            Line::from(""),
        ]
    };

    let radio_start = area.y + 2;
    for i in 0..2usize {
        let y = radio_start + (i as u16) * 3;
        if y < area.y + area.height {
            state.hit.radio[i] = Some(Rect { x: area.x, y, width: area.width, height: 2 });
        }
    }

    let mut lines = vec![
        Line::from("  Capture source"),
        Line::from(""),
    ];
    lines.extend(radio(0, "Game Capture", "hooks into fullscreen games — recommended for most users"));
    lines.extend(radio(1, "Display Capture", "records the entire monitor — works with windowed games"));

    f.render_widget(Paragraph::new(lines), area);
}

fn draw_autostart(f: &mut Frame, state: &mut State, area: Rect) {
    let (on_style, off_style) = if state.autostart {
        (Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD), Style::default().fg(Color::DarkGray))
    } else {
        (Style::default().fg(Color::DarkGray), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
    };

    // Row 5 holds the Yes/No toggle; split area in half for hit targets
    let toggle_row = area.y + 5;
    if toggle_row < area.y + area.height {
        let half = area.width / 2;
        state.hit.radio[0] = Some(Rect { x: area.x,        y: toggle_row, width: half,            height: 1 });
        state.hit.radio[1] = Some(Rect { x: area.x + half, y: toggle_row, width: area.width - half, height: 1 });
    }

    let lines = vec![
        Line::from("  Start daemon on login"),
        Line::from(""),
        Line::from(Span::styled(
            "  The daemon watches for replay saves and processes clips.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "  Enabling autostart means you never have to think about it.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled("[ Yes — start on login ]", on_style),
            Span::raw("   "),
            Span::styled("[ No — I'll start it manually ]", off_style),
        ]),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_confirm(f: &mut Frame, state: &mut State, area: Rect) {
    let obs_label: String = match state.obs_radio {
        0 => "Download OBS automatically".into(),
        1 => match &state.obs_detected {
            Some(p) => format!("Use existing OBS (detected at {})", p),
            None    => "Use existing OBS (will auto-detect WebSocket config)".into(),
        },
        _ => "Skip OBS management".into(),
    };
    let source_label = if state.capture_radio == 1 { "Display Capture" } else { "Game Capture" };
    let autostart_label = if state.autostart { "Yes — register autostart" } else { "No" };

    let check = Span::styled("  ✓  ", Style::default().fg(Color::Green));
    let key_style = Style::default().fg(Color::DarkGray);

    let mut lines = vec![
        Line::from(Span::styled("  Ready to install.", Style::default().add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![check.clone(), Span::styled("OBS:        ", key_style), Span::raw(&obs_label)]),
        Line::from(vec![check.clone(), Span::styled("Output:     ", key_style), Span::raw(&state.output_dir)]),
        Line::from(vec![check.clone(), Span::styled("Buffer:     ", key_style), Span::raw(format!("{} seconds", state.buffer_secs))]),
        Line::from(vec![check.clone(), Span::styled("Capture:    ", key_style), Span::raw(source_label)]),
        Line::from(vec![check.clone(), Span::styled("Autostart:  ", key_style), Span::raw(autostart_label)]),
        Line::from(""),
    ];

    if state.obs_radio == 1 {
        lines.push(Line::from(Span::styled(
            "  ⚠  Enable the replay buffer in OBS: Settings → Output → Replay Buffer",
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "  Press Enter to install.",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )));

    // Row offset shifts by 2 if the replay buffer reminder is shown
    let install_row = area.y + 8 + if state.obs_radio == 1 { 2 } else { 0 };
    if install_row < area.y + area.height {
        state.hit.next = Some(Rect { x: area.x, y: install_row, width: area.width, height: 1 });
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_help(f: &mut Frame, state: &State, area: Rect) {
    let text = match state.step {
        Step::Welcome        => "  Enter: begin   q: quit",
        Step::ObsChoice      => "  ↑/↓: select   Enter/→: next   q: quit",
        Step::OutputDir      => "  type folder   Enter: next   ←: back",
        Step::BufferDuration => "  ↑/↓: adjust   Enter: next   ←: back",
        Step::CaptureSource  => "  ↑/↓: select   Enter/→: next   ←: back",
        Step::Autostart      => "  ↑/↓/Space: toggle   Enter: next   ←: back",
        Step::Confirm        => "  Enter: install   ←/b: back   q: cancel",
    };
    f.render_widget(
        Paragraph::new(text).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}
