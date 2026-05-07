pub mod app;

use crate::{
    config::Config,
    ipc::{ClientMessage, IpcClient, ServerMessage},
};
use anyhow::Result;
use app::{App, Focus, Modal};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind, EnableMouseCapture, DisableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{io, time::Duration};
use tokio::sync::mpsc;
use tracing::info;

pub async fn run(config: &Config) -> Result<()> {
    let (client, cmd_client) = wait_for_daemon(config.ipc_port).await?;

    let (msg_tx, mut msg_rx) = mpsc::channel::<ServerMessage>(64);

    let msg_tx_clone = msg_tx.clone();
    tokio::spawn(async move {
        let mut c = client;
        loop {
            match c.next_message().await {
                Ok(Some(msg)) => {
                    if msg_tx_clone.send(msg).await.is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }
    });

    let mut cmd_client = cmd_client;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();

    let result = run_loop(&mut terminal, &mut app, &mut msg_rx, &mut cmd_client).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;

    result
}

async fn wait_for_daemon(port: u16) -> Result<(IpcClient, IpcClient)> {
    let mut attempts = 0u32;
    loop {
        match IpcClient::connect(port).await {
            Ok(reader) => {
                match IpcClient::connect(port).await {
                    Ok(writer) => {
                        if attempts > 0 {
                            eprintln!("\r[rscapt] Connected to daemon.          ");
                        }
                        let mut reader = reader;
                        // Request initial job snapshot + clip library
                        reader.send(&ClientMessage::GetJobs).await?;
                        reader.send(&ClientMessage::GetClips).await?;
                        return Ok((reader, writer));
                    }
                    Err(_) => {}
                }
            }
            Err(_) => {
                attempts += 1;
                if attempts == 1 {
                    eprint!("[rscapt] Waiting for daemon on port {port}...");
                } else {
                    eprint!(".");
                }
                info!(attempts, "TUI waiting for daemon");
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    msg_rx: &mut mpsc::Receiver<ServerMessage>,
    cmd_client: &mut IpcClient,
) -> Result<()> {
    loop {
        // Drain incoming messages
        while let Ok(msg) = msg_rx.try_recv() {
            handle_server_message(app, msg, cmd_client).await;
        }

        terminal.draw(|f| app.draw(f))?;

        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press { continue; }
                    if handle_key(app, key.code, cmd_client).await? { break; }
                }
                Event::Mouse(m) => {
                    handle_mouse(app, m, cmd_client).await?;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

async fn handle_server_message(app: &mut App, msg: ServerMessage, _cmd_client: &mut IpcClient) {
    match msg {
        ServerMessage::Snapshot { jobs } => {
            for job in jobs {
                app.upsert_job(job);
            }
        }
        ServerMessage::JobUpdate { job } => {
            // If a Share job completed, auto-copy URL to clipboard
            if let crate::job::JobKind::Share = &job.kind {
                if job.status == crate::job::JobStatus::Done {
                    if let Some(url) = &job.share_url {
                        copy_to_clipboard(url);
                    }
                }
            }
            app.upsert_job(job);
        }
        ServerMessage::ClipLibrary { clips } | ServerMessage::ClipUpdated { clips } => {
            app.set_clips(clips);
        }
        ServerMessage::Cancelled { .. } => {}
        ServerMessage::Error { message } => {
            tracing::warn!("Daemon error: {message}");
        }
    }
}

/// Returns true if the TUI should quit.
async fn handle_key(
    app: &mut App,
    code: KeyCode,
    cmd_client: &mut IpcClient,
) -> Result<bool> {
    match &app.modal {
        Modal::None => handle_normal_key(app, code, cmd_client).await,
        Modal::PostProcess(_) => handle_pp_key(app, code, cmd_client).await,
        Modal::Compress(_) => handle_compress_key(app, code, cmd_client).await,
        Modal::Share(_) => handle_share_key(app, code, cmd_client).await,
    }
}

async fn handle_mouse(
    app: &mut App,
    m: crossterm::event::MouseEvent,
    _cmd_client: &mut IpcClient,
) -> Result<()> {
    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            app.on_click(m.column, m.row);
        }
        MouseEventKind::ScrollDown => {
            app.on_scroll(m.column, m.row, true);
        }
        MouseEventKind::ScrollUp => {
            app.on_scroll(m.column, m.row, false);
        }
        _ => {}
    }
    Ok(())
}

async fn handle_normal_key(
    app: &mut App,
    code: KeyCode,
    cmd_client: &mut IpcClient,
) -> Result<bool> {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
        KeyCode::Tab => app.toggle_focus(),
        KeyCode::Down | KeyCode::Char('j') => app.select_next(),
        KeyCode::Up   | KeyCode::Char('k') => app.select_prev(),

        // Job panel actions
        KeyCode::Char('c') if app.focus == Focus::Jobs => {
            if let Some(id) = app.selected_job_id() {
                cmd_client.send(&ClientMessage::Cancel { id }).await?;
            }
        }

        // Clip panel actions
        KeyCode::Char('p') if app.focus == Focus::Clips => app.open_post_process(),
        KeyCode::Char('x') if app.focus == Focus::Clips => app.open_compress(),
        KeyCode::Char('s') if app.focus == Focus::Clips => app.open_share(),
        KeyCode::Char('d') if app.focus == Focus::Clips => {
            if let Some(clip) = app.selected_clip() {
                if clip.share_url.is_some() {
                    let path = clip.path.clone();
                    cmd_client.send(&ClientMessage::DeleteShare { clip_path: path }).await?;
                }
            }
        }

        _ => {}
    }
    Ok(false)
}

async fn handle_pp_key(
    app: &mut App,
    code: KeyCode,
    cmd_client: &mut IpcClient,
) -> Result<bool> {
    match code {
        KeyCode::Esc => { app.modal = Modal::None; }
        KeyCode::Down | KeyCode::Char('j') => app.pp_nav_down(),
        KeyCode::Up   | KeyCode::Char('k') => app.pp_nav_up(),
        KeyCode::Char(' ') => app.pp_toggle(),
        KeyCode::Right | KeyCode::Char('+') | KeyCode::Char('l') => app.pp_inc(),
        KeyCode::Left  | KeyCode::Char('-') | KeyCode::Char('h') => app.pp_dec(),
        KeyCode::Enter => {
            if let Some((clip_path, effects)) = app.pp_confirm() {
                cmd_client
                    .send(&ClientMessage::PostProcess { clip_path, effects })
                    .await?;
            }
            app.modal = Modal::None;
        }
        _ => {}
    }
    Ok(false)
}

async fn handle_compress_key(
    app: &mut App,
    code: KeyCode,
    cmd_client: &mut IpcClient,
) -> Result<bool> {
    // Check if cursor is on a text field
    let on_text_field = matches!(
        &app.modal,
        Modal::Compress(s) if matches!(s.field, app::CompressField::TrimStart | app::CompressField::TrimEnd)
    );

    match code {
        KeyCode::Esc => { app.modal = Modal::None; }
        KeyCode::Down  | KeyCode::Tab => app.compress_nav_down(),
        KeyCode::Up => app.compress_nav_up(),
        KeyCode::Left  => app.compress_cycle_left(),
        KeyCode::Right => app.compress_cycle_right(),
        KeyCode::Backspace if on_text_field => app.compress_backspace(),
        KeyCode::Char(c) if on_text_field => app.compress_type_char(c),
        KeyCode::Enter => {
            if let Some((clip_path, options)) = app.compress_confirm() {
                cmd_client
                    .send(&ClientMessage::Compress { clip_path, options })
                    .await?;
            }
            app.modal = Modal::None;
        }
        _ => {}
    }
    Ok(false)
}

async fn handle_share_key(
    app: &mut App,
    code: KeyCode,
    cmd_client: &mut IpcClient,
) -> Result<bool> {
    match code {
        KeyCode::Esc => { app.modal = Modal::None; }
        KeyCode::Enter => {
            if let Some(clip_path) = app.share_clip_path() {
                cmd_client
                    .send(&ClientMessage::Share { clip_path })
                    .await?;
            }
            app.modal = Modal::None;
        }
        KeyCode::Char('d') => {
            if let Some(clip_path) = app.share_delete_path() {
                cmd_client
                    .send(&ClientMessage::DeleteShare { clip_path })
                    .await?;
                app.modal = Modal::None;
            }
        }
        _ => {}
    }
    Ok(false)
}

/// Copy text to the Windows clipboard via PowerShell Set-Clipboard.
/// No-op on non-Windows.
fn copy_to_clipboard(text: &str) {
    #[cfg(windows)]
    {
        let safe = text.replace('\'', "''");
        let _ = std::process::Command::new("powershell")
            .args([
                "-WindowStyle", "Hidden",
                "-NonInteractive",
                "-NoProfile",
                "-Command", &format!("Set-Clipboard -Value '{safe}'"),
            ])
            .spawn();
    }
    #[cfg(not(windows))]
    {
        tracing::info!("[clipboard] {text}");
    }
}
