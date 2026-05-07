use crate::{
    clips::Clip,
    job::{
        CompressCodec, CompressOptions, CompressQuality, Effect, Job, JobStatus,
    },
};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Wrap},
};

// ── Focus ──────────────────────────────────────────────────────────────────────

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Focus {
    Jobs,
    Clips,
}

// ── Modal states ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct EffectRow {
    pub label: &'static str,
    pub enabled: bool,
    pub value: EffectValue,
}

#[derive(Clone)]
pub enum EffectValue {
    Float { val: f32, min: f32, max: f32, step: f32 },
    Int   { val: u32, min: u32, max: u32, step: u32 },
}

impl EffectValue {
    pub fn inc(&mut self) {
        match self {
            Self::Float { val, max, step, .. } => *val = (*val + *step).min(*max),
            Self::Int   { val, max, step, .. } => *val = (*val + *step).min(*max),
        }
    }
    pub fn dec(&mut self) {
        match self {
            Self::Float { val, min, step, .. } => *val = (*val - *step).max(*min),
            Self::Int   { val, min, step, .. } => *val = val.saturating_sub(*step).max(*min),
        }
    }
    pub fn display(&self) -> String {
        match self {
            Self::Float { val, .. } => format!("{val:.2}"),
            Self::Int   { val, .. } => format!("{val}"),
        }
    }
}

impl EffectRow {
    pub fn to_effect(&self) -> Option<Effect> {
        if !self.enabled { return None; }
        match (self.label, &self.value) {
            ("Saturation", EffectValue::Float { val, .. }) => Some(Effect::Saturation(*val)),
            ("Sharpen",    EffectValue::Float { val, .. }) => Some(Effect::Sharpen(*val)),
            ("Interpolate",EffectValue::Int   { val, .. }) => Some(Effect::Interpolate { target_fps: *val }),
            ("MotionBlur", EffectValue::Int   { val, .. }) => Some(Effect::MotionBlur { shutter_angle: *val }),
            _ => None,
        }
    }
}

fn default_effect_rows() -> Vec<EffectRow> {
    vec![
        EffectRow {
            label: "Saturation",
            enabled: true,
            value: EffectValue::Float { val: 1.30, min: 0.5, max: 3.0, step: 0.05 },
        },
        EffectRow {
            label: "Sharpen",
            enabled: false,
            value: EffectValue::Float { val: 1.00, min: 0.0, max: 5.0, step: 0.10 },
        },
        EffectRow {
            label: "Interpolate",
            enabled: false,
            value: EffectValue::Int { val: 120, min: 60, max: 240, step: 30 },
        },
        EffectRow {
            label: "MotionBlur",
            enabled: false,
            value: EffectValue::Int { val: 180, min: 45, max: 360, step: 15 },
        },
    ]
}

pub struct PostProcessState {
    pub rows: Vec<EffectRow>,
    pub cursor: usize,
    pub clip_path: std::path::PathBuf,
}

const CODECS: &[CompressCodec] = &[
    CompressCodec::H264Nvenc,
    CompressCodec::HevcNvenc,
    CompressCodec::Hevc,
    CompressCodec::Av1,
];

const QUALITIES: &[CompressQuality] = &[
    CompressQuality::High,
    CompressQuality::Med,
    CompressQuality::Low,
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CompressField {
    Codec,
    Quality,
    TrimStart,
    TrimEnd,
}

pub struct CompressState {
    pub codec_idx: usize,
    pub quality_idx: usize,
    pub trim_start: String,
    pub trim_end: String,
    pub field: CompressField,
    pub clip_path: std::path::PathBuf,
}

impl CompressState {
    pub fn to_options(&self) -> CompressOptions {
        CompressOptions {
            codec: CODECS[self.codec_idx].clone(),
            quality: QUALITIES[self.quality_idx],
            trim_start: non_empty(&self.trim_start),
            trim_end: non_empty(&self.trim_end),
        }
    }
}

pub struct ShareState {
    pub clip: Clip,
    /// Index into `crate::share::EXPIRY_OPTIONS`
    pub expiry_idx: usize,
}

pub enum Modal {
    None,
    PostProcess(PostProcessState),
    Compress(CompressState),
    Share(ShareState),
}

// ── Hit areas (stored after each draw for mouse hit-testing) ─────────────────

#[derive(Default, Clone, Copy)]
pub struct HitRects {
    pub job_list: Rect,
    pub clip_list: Rect,
}

// ── App ───────────────────────────────────────────────────────────────────────

pub struct App {
    pub jobs: Vec<Job>,
    pub clips: Vec<Clip>,
    pub focus: Focus,
    pub job_list: ListState,
    pub clip_list: ListState,
    pub modal: Modal,
    pub hit: HitRects,
    /// Set when the daemon reports a newer version is available.
    pub update_available: Option<String>,
    /// Default expiry index for the share modal (from config).
    pub default_expiry_idx: usize,
}

impl App {
    pub fn new(default_expiry_idx: usize) -> Self {
        Self {
            jobs: Vec::new(),
            clips: Vec::new(),
            focus: Focus::Jobs,
            job_list: ListState::default(),
            clip_list: ListState::default(),
            modal: Modal::None,
            hit: HitRects::default(),
            update_available: None,
            default_expiry_idx,
        }
    }

    /// Handle a left mouse click at (col, row). Returns true if anything changed.
    pub fn on_click(&mut self, col: u16, row: u16) -> bool {
        // Clicks inside a modal close/dismiss nothing — modals handle keyboard only
        if !matches!(self.modal, Modal::None) {
            return false;
        }

        let pt = (col, row);

        if rect_contains(self.hit.job_list, pt) {
            self.focus = Focus::Jobs;
            // Row 0 inside the rect is the border, items start at row 1
            let inner_row = row.saturating_sub(self.hit.job_list.y + 1) as usize;
            if inner_row < self.jobs.len() {
                self.job_list.select(Some(inner_row));
            }
            return true;
        }

        if rect_contains(self.hit.clip_list, pt) {
            self.focus = Focus::Clips;
            let inner_row = row.saturating_sub(self.hit.clip_list.y + 1) as usize;
            if inner_row < self.clips.len() {
                self.clip_list.select(Some(inner_row));
            }
            return true;
        }

        false
    }

    /// Handle scroll wheel. `down` = true for scroll down.
    pub fn on_scroll(&mut self, col: u16, row: u16, down: bool) {
        if rect_contains(self.hit.job_list, (col, row)) {
            self.focus = Focus::Jobs;
            if down { self.select_next() } else { self.select_prev() }
        } else if rect_contains(self.hit.clip_list, (col, row)) {
            self.focus = Focus::Clips;
            if down { self.select_next() } else { self.select_prev() }
        }
    }

    // ── State updates ─────────────────────────────────────────────────────────

    pub fn upsert_job(&mut self, job: Job) {
        if let Some(existing) = self.jobs.iter_mut().find(|j| j.id == job.id) {
            *existing = job;
        } else {
            self.jobs.push(job);
            if self.job_list.selected().is_none() {
                self.job_list.select(Some(0));
            }
        }
    }

    pub fn set_clips(&mut self, clips: Vec<Clip>) {
        self.clips = clips;
        if self.clip_list.selected().is_none() && !self.clips.is_empty() {
            self.clip_list.select(Some(0));
        }
    }

    pub fn selected_job(&self) -> Option<&Job> {
        self.job_list.selected().and_then(|i| self.jobs.get(i))
    }

    pub fn selected_clip(&self) -> Option<&Clip> {
        self.clip_list.selected().and_then(|i| self.clips.get(i))
    }

    pub fn selected_job_id(&self) -> Option<uuid::Uuid> {
        self.selected_job().map(|j| j.id)
    }

    // ── Navigation ────────────────────────────────────────────────────────────

    pub fn select_next(&mut self) {
        match self.focus {
            Focus::Jobs => {
                if self.jobs.is_empty() { return; }
                let i = self.job_list.selected().map(|i| (i + 1) % self.jobs.len()).unwrap_or(0);
                self.job_list.select(Some(i));
            }
            Focus::Clips => {
                if self.clips.is_empty() { return; }
                let i = self.clip_list.selected().map(|i| (i + 1) % self.clips.len()).unwrap_or(0);
                self.clip_list.select(Some(i));
            }
        }
    }

    pub fn select_prev(&mut self) {
        match self.focus {
            Focus::Jobs => {
                if self.jobs.is_empty() { return; }
                let i = self.job_list.selected()
                    .map(|i| if i == 0 { self.jobs.len() - 1 } else { i - 1 })
                    .unwrap_or(0);
                self.job_list.select(Some(i));
            }
            Focus::Clips => {
                if self.clips.is_empty() { return; }
                let i = self.clip_list.selected()
                    .map(|i| if i == 0 { self.clips.len() - 1 } else { i - 1 })
                    .unwrap_or(0);
                self.clip_list.select(Some(i));
            }
        }
    }

    pub fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Jobs => Focus::Clips,
            Focus::Clips => Focus::Jobs,
        };
    }

    // ── Modal openers ─────────────────────────────────────────────────────────

    pub fn open_post_process(&mut self) {
        if let Some(clip) = self.selected_clip() {
            self.modal = Modal::PostProcess(PostProcessState {
                rows: default_effect_rows(),
                cursor: 0,
                clip_path: clip.path.clone(),
            });
        }
    }

    pub fn open_compress(&mut self) {
        if let Some(clip) = self.selected_clip() {
            self.modal = Modal::Compress(CompressState {
                codec_idx: 0,
                quality_idx: 0,
                trim_start: String::new(),
                trim_end: String::new(),
                field: CompressField::Codec,
                clip_path: clip.path.clone(),
            });
        }
    }

    pub fn open_share(&mut self) {
        if let Some(clip) = self.selected_clip() {
            self.modal = Modal::Share(ShareState {
                clip: clip.clone(),
                expiry_idx: self.default_expiry_idx,
            });
        }
    }

    // ── Drawing ───────────────────────────────────────────────────────────────

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(3),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(area);

        // Main two-panel area
        let panels = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(chunks[0]);

        // Store panel rects for mouse hit-testing
        self.hit.job_list  = panels[0];
        self.hit.clip_list = panels[1];

        self.draw_jobs(frame, panels[0]);
        self.draw_clips(frame, panels[1]);
        self.draw_info_bar(frame, chunks[1]);
        self.draw_help(frame, chunks[2]);

        // Modals drawn on top
        match &self.modal {
            Modal::None => {}
            Modal::PostProcess(_) => self.draw_modal_pp(frame, area),
            Modal::Compress(_) => self.draw_modal_compress(frame, area),
            Modal::Share(_) => self.draw_modal_share(frame, area),
        }
    }

    fn draw_jobs(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Jobs;
        let border_style = if focused {
            Style::default().fg(Color::Rgb(232, 80, 26))
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let items: Vec<ListItem> = self.jobs.iter().map(|job| {
            let (status_str, status_color) = match &job.status {
                JobStatus::Queued    => ("QUEUE ", Color::DarkGray),
                JobStatus::Running   => ("RUN   ", Color::Yellow),
                JobStatus::Done      => ("DONE  ", Color::Green),
                JobStatus::Failed(_) => ("FAIL  ", Color::Red),
                JobStatus::Cancelled => ("SKIP  ", Color::Magenta),
            };
            let pct = if job.status == JobStatus::Running {
                format!(" {:3}%", job.progress)
            } else {
                String::new()
            };
            Line::from(vec![
                Span::styled(status_str, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
                Span::styled(format!("[{}]", job.kind_label()), Style::default().fg(Color::Cyan)),
                Span::raw(format!(" {}{}", job.display_name(), pct)),
            ])
        }).map(ListItem::new).collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(" Jobs "),
            )
            .highlight_style(Style::default().bg(Color::Rgb(60, 20, 5)).add_modifier(Modifier::BOLD))
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(list, area, &mut self.job_list);
    }

    fn draw_clips(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Clips;
        let border_style = if focused {
            Style::default().fg(Color::Rgb(232, 80, 26))
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let items: Vec<ListItem> = self.clips.iter().map(|clip| {
            let share_indicator = if clip.share_url.is_some() { " 🔗" } else { "" };
            Line::from(vec![
                Span::raw(&clip.filename),
                Span::styled(
                    format!("  {}{}", clip.size_label(), share_indicator),
                    Style::default().fg(Color::DarkGray),
                ),
            ])
        }).map(ListItem::new).collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(" Clips "),
            )
            .highlight_style(Style::default().bg(Color::Rgb(60, 20, 5)).add_modifier(Modifier::BOLD))
            .highlight_symbol("▶ ");

        frame.render_stateful_widget(list, area, &mut self.clip_list);
    }

    fn draw_info_bar(&self, frame: &mut Frame, area: Rect) {
        match self.focus {
            Focus::Jobs => {
                // Show progress for selected job
                match self.selected_job() {
                    None => {
                        let p = Paragraph::new("No job selected")
                            .block(Block::default().borders(Borders::ALL).title(" Progress "));
                        frame.render_widget(p, area);
                    }
                    Some(job) => {
                        let label = match &job.status {
                            JobStatus::Running => format!("{}%  {}", job.progress, job.display_name()),
                            JobStatus::Done => format!("Done  {}", job.display_name()),
                            JobStatus::Failed(e) => format!("Error: {e}"),
                            JobStatus::Cancelled => format!("Cancelled  {}", job.display_name()),
                            JobStatus::Queued => format!("Waiting…  {}", job.display_name()),
                        };
                        let color = match &job.status {
                            JobStatus::Done => Color::Green,
                            JobStatus::Failed(_) => Color::Red,
                            JobStatus::Cancelled => Color::Magenta,
                            _ => Color::Rgb(232, 80, 26),
                        };
                        let gauge = Gauge::default()
                            .block(Block::default().borders(Borders::ALL).title(" Progress "))
                            .gauge_style(Style::default().fg(color))
                            .percent(job.progress as u16)
                            .label(label);
                        frame.render_widget(gauge, area);
                    }
                }
            }
            Focus::Clips => {
                // Show clip info
                let text = match self.selected_clip() {
                    None => "No clip selected".into(),
                    Some(c) => {
                        let share_info = match &c.share_url {
                            Some(url) => format!("  |  Shared: {url}"),
                            None => String::new(),
                        };
                        format!("{}  |  {}{}", c.filename, c.size_label(), share_info)
                    }
                };
                let p = Paragraph::new(text)
                    .block(Block::default().borders(Borders::ALL).title(" Clip Info "));
                frame.render_widget(p, area);
            }
        }
    }

    fn draw_help(&self, frame: &mut Frame, area: Rect) {
        // If an update is available, show it in the help bar (overrides modal hints
        // only in normal mode — modal hints still show when a modal is open).
        if let (Some(version), Modal::None) = (&self.update_available, &self.modal) {
            let line = Line::from(vec![
                Span::styled(
                    format!("  Update available: {version} — run "),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(
                    "rscapt update",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
                Span::styled("   q: quit", Style::default().fg(Color::DarkGray)),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            return;
        }

        let help = match &self.modal {
            Modal::None => {
                if self.focus == Focus::Jobs {
                    "  Tab: clips panel   ↑/↓: navigate   c: cancel job   q: quit"
                } else {
                    "  Tab: jobs panel   ↑/↓: navigate   p: post-process   x: compress   s: share   q: quit"
                }
            }
            Modal::PostProcess(_) => "  ↑/↓: navigate   Space: toggle   ←/→: adjust value   Enter: apply   Esc: cancel",
            Modal::Compress(_)    => "  ↑/↓: fields   ←/→: change codec/quality   type: trim time   Enter: queue   Esc: cancel",
            Modal::Share(_)       => "  ←/→: expiry   Enter: upload   Esc: cancel",
        };
        frame.render_widget(
            Paragraph::new(help).style(Style::default().fg(Color::DarkGray)),
            area,
        );
    }

    // ── PostProcess modal ─────────────────────────────────────────────────────

    fn draw_modal_pp(&self, frame: &mut Frame, area: Rect) {
        let Modal::PostProcess(state) = &self.modal else { return };

        let popup = centered_rect(50, 70, area);
        frame.render_widget(Clear, popup);

        let inner = Block::default()
            .borders(Borders::ALL)
            .title(" Post-Process ")
            .style(Style::default().fg(Color::White));
        let inner_area = inner.inner(popup);
        frame.render_widget(inner, popup);

        let row_height = 1u16;
        let rows: Vec<Rect> = (0..state.rows.len())
            .map(|i| Rect {
                x: inner_area.x + 1,
                y: inner_area.y + 1 + i as u16 * (row_height + 1),
                width: inner_area.width.saturating_sub(2),
                height: row_height,
            })
            .collect();

        for (i, (row_rect, effect)) in rows.iter().zip(state.rows.iter()).enumerate() {
            if row_rect.y >= inner_area.y + inner_area.height {
                break;
            }
            let selected = i == state.cursor;
            let checkbox = if effect.enabled { "[x]" } else { "[ ]" };
            let val = effect.value.display();
            let unit = match effect.label {
                "Interpolate" => " fps",
                "MotionBlur" => "°",
                _ => "",
            };
            let style = if selected {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let label_color = if effect.enabled { Color::Green } else { Color::DarkGray };
            let p = Paragraph::new(Line::from(vec![
                Span::styled(checkbox, Style::default().fg(label_color)),
                Span::raw(format!(" {:<12}  {}{:<8}  -/+", effect.label, val, unit)),
            ])).style(style);
            frame.render_widget(p, *row_rect);
        }

        // Hint at bottom of modal
        let hint_y = inner_area.y + inner_area.height.saturating_sub(2);
        if hint_y > inner_area.y {
            let hint = Paragraph::new("Enter to apply, Esc to cancel")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray));
            let hint_rect = Rect { x: inner_area.x, y: hint_y, width: inner_area.width, height: 1 };
            frame.render_widget(hint, hint_rect);
        }
    }

    // ── Compress modal ────────────────────────────────────────────────────────

    fn draw_modal_compress(&self, frame: &mut Frame, area: Rect) {
        let Modal::Compress(state) = &self.modal else { return };

        let popup = centered_rect(52, 60, area);
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Compress ")
            .style(Style::default().fg(Color::White));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let fields = [
            (CompressField::Codec,     "Codec",      format!("◄ {} ►", CODECS[state.codec_idx].label())),
            (CompressField::Quality,   "Quality",    format!("◄ {} ►", QUALITIES[state.quality_idx].label())),
            (CompressField::TrimStart, "Trim start", if state.trim_start.is_empty() { "(start)".into() } else { state.trim_start.clone() }),
            (CompressField::TrimEnd,   "Trim end",   if state.trim_end.is_empty()   { "(end)"  .into() } else { state.trim_end.clone()   }),
        ];

        for (i, (field_id, label, val)) in fields.iter().enumerate() {
            let y = inner.y + 1 + i as u16 * 2;
            if y >= inner.y + inner.height { break; }
            let selected = state.field == *field_id;
            let style = if selected {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let cursor = if selected && matches!(field_id, CompressField::TrimStart | CompressField::TrimEnd) { "_" } else { "" };
            let p = Paragraph::new(format!("  {:<12}  {}{}", label, val, cursor)).style(style);
            let rect = Rect { x: inner.x, y, width: inner.width, height: 1 };
            frame.render_widget(p, rect);
        }

        let hint_y = inner.y + inner.height.saturating_sub(2);
        if hint_y > inner.y {
            let hint = Paragraph::new("Enter to queue, Esc to cancel")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(hint, Rect { x: inner.x, y: hint_y, width: inner.width, height: 1 });
        }
    }

    // ── Share modal ───────────────────────────────────────────────────────────

    fn draw_modal_share(&self, frame: &mut Frame, area: Rect) {
        let Modal::Share(state) = &self.modal else { return };

        let popup = centered_rect(56, 55, area);
        frame.render_widget(Clear, popup);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Share to litterbox.catbox.moe ")
            .style(Style::default().fg(Color::White));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let clip = &state.clip;
        let (expiry_key, expiry_label) = crate::share::EXPIRY_OPTIONS[state.expiry_idx];

        let mut lines: Vec<Line> = vec![
            Line::from(vec![
                Span::styled("  File:    ", Style::default().fg(Color::DarkGray)),
                Span::raw(&clip.filename),
            ]),
            Line::from(vec![
                Span::styled("  Size:    ", Style::default().fg(Color::DarkGray)),
                Span::raw(clip.size_label()),
            ]),
            Line::from(vec![
                Span::styled("  Expires: ", Style::default().fg(Color::DarkGray)),
                Span::styled("◄ ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{expiry_label} ({expiry_key})"),
                    Style::default().fg(Color::Rgb(232, 80, 26)).add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ►  ←/→ to change", Style::default().fg(Color::DarkGray)),
            ]),
        ];

        if let Some(url) = &clip.share_url {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  URL:     ", Style::default().fg(Color::DarkGray)),
                Span::styled(url.as_str(), Style::default().fg(Color::Cyan)),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  Enter: re-upload   Esc: cancel", Style::default().fg(Color::DarkGray)),
            ]));
        } else {
            lines.push(Line::from(""));
            lines.push(Line::from(
                Span::styled("  Enter: upload   Esc: cancel", Style::default().fg(Color::DarkGray)),
            ));
        }

        let p = Paragraph::new(lines).wrap(Wrap { trim: false });
        frame.render_widget(p, inner);
    }

    // ── PostProcess key handlers ──────────────────────────────────────────────

    pub fn pp_nav_down(&mut self) {
        if let Modal::PostProcess(s) = &mut self.modal {
            s.cursor = (s.cursor + 1) % s.rows.len();
        }
    }

    pub fn pp_nav_up(&mut self) {
        if let Modal::PostProcess(s) = &mut self.modal {
            if s.cursor == 0 { s.cursor = s.rows.len() - 1; } else { s.cursor -= 1; }
        }
    }

    pub fn pp_toggle(&mut self) {
        if let Modal::PostProcess(s) = &mut self.modal {
            s.rows[s.cursor].enabled = !s.rows[s.cursor].enabled;
        }
    }

    pub fn pp_inc(&mut self) {
        if let Modal::PostProcess(s) = &mut self.modal {
            s.rows[s.cursor].value.inc();
        }
    }

    pub fn pp_dec(&mut self) {
        if let Modal::PostProcess(s) = &mut self.modal {
            s.rows[s.cursor].value.dec();
        }
    }

    /// Returns (clip_path, effects) if any effects are enabled.
    pub fn pp_confirm(&self) -> Option<(std::path::PathBuf, Vec<Effect>)> {
        if let Modal::PostProcess(s) = &self.modal {
            let effects: Vec<Effect> = s.rows.iter().filter_map(|r| r.to_effect()).collect();
            if effects.is_empty() { return None; }
            return Some((s.clip_path.clone(), effects));
        }
        None
    }

    // ── Compress key handlers ─────────────────────────────────────────────────

    pub fn compress_nav_down(&mut self) {
        if let Modal::Compress(s) = &mut self.modal {
            s.field = match s.field {
                CompressField::Codec     => CompressField::Quality,
                CompressField::Quality   => CompressField::TrimStart,
                CompressField::TrimStart => CompressField::TrimEnd,
                CompressField::TrimEnd   => CompressField::Codec,
            };
        }
    }

    pub fn compress_nav_up(&mut self) {
        if let Modal::Compress(s) = &mut self.modal {
            s.field = match s.field {
                CompressField::Codec     => CompressField::TrimEnd,
                CompressField::Quality   => CompressField::Codec,
                CompressField::TrimStart => CompressField::Quality,
                CompressField::TrimEnd   => CompressField::TrimStart,
            };
        }
    }

    pub fn compress_cycle_left(&mut self) {
        if let Modal::Compress(s) = &mut self.modal {
            match s.field {
                CompressField::Codec => {
                    s.codec_idx = if s.codec_idx == 0 { CODECS.len() - 1 } else { s.codec_idx - 1 };
                }
                CompressField::Quality => {
                    s.quality_idx = if s.quality_idx == 0 { QUALITIES.len() - 1 } else { s.quality_idx - 1 };
                }
                _ => {}
            }
        }
    }

    pub fn compress_cycle_right(&mut self) {
        if let Modal::Compress(s) = &mut self.modal {
            match s.field {
                CompressField::Codec    => s.codec_idx    = (s.codec_idx    + 1) % CODECS.len(),
                CompressField::Quality  => s.quality_idx  = (s.quality_idx  + 1) % QUALITIES.len(),
                _ => {}
            }
        }
    }

    pub fn compress_type_char(&mut self, c: char) {
        if let Modal::Compress(s) = &mut self.modal {
            // Allow digits, colon, period for time strings
            if c.is_ascii_digit() || c == ':' || c == '.' {
                match s.field {
                    CompressField::TrimStart => s.trim_start.push(c),
                    CompressField::TrimEnd   => s.trim_end.push(c),
                    _ => {}
                }
            }
        }
    }

    pub fn compress_backspace(&mut self) {
        if let Modal::Compress(s) = &mut self.modal {
            match s.field {
                CompressField::TrimStart => { s.trim_start.pop(); }
                CompressField::TrimEnd   => { s.trim_end.pop(); }
                _ => {}
            }
        }
    }

    /// Returns (clip_path, options).
    pub fn compress_confirm(&self) -> Option<(std::path::PathBuf, CompressOptions)> {
        if let Modal::Compress(s) = &self.modal {
            return Some((s.clip_path.clone(), s.to_options()));
        }
        None
    }

    // ── Share key handlers ────────────────────────────────────────────────────

    /// Returns (clip_path, expiry_str) for the share modal.
    pub fn share_confirm(&self) -> Option<(std::path::PathBuf, String)> {
        if let Modal::Share(s) = &self.modal {
            let expiry = crate::share::EXPIRY_OPTIONS[s.expiry_idx].0.to_owned();
            return Some((s.clip.path.clone(), expiry));
        }
        None
    }

    pub fn share_expiry_next(&mut self) {
        if let Modal::Share(s) = &mut self.modal {
            s.expiry_idx = (s.expiry_idx + 1) % crate::share::EXPIRY_OPTIONS.len();
        }
    }

    pub fn share_expiry_prev(&mut self) {
        if let Modal::Share(s) = &mut self.modal {
            let n = crate::share::EXPIRY_OPTIONS.len();
            s.expiry_idx = if s.expiry_idx == 0 { n - 1 } else { s.expiry_idx - 1 };
        }
    }
}

// ── Layout helpers ────────────────────────────────────────────────────────────

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_owned()) }
}

fn rect_contains(r: Rect, (col, row): (u16, u16)) -> bool {
    col >= r.x && col < r.x + r.width && row >= r.y && row < r.y + r.height
}
