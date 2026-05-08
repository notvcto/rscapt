//! System tray icon — Windows only.
//!
//! Starts the daemon if it isn't running, then sits in a Win32 message loop
//! handling menu clicks and double-click-to-open-TUI.

#[cfg(windows)]
mod imp {
    use anyhow::{Context, Result};
    use std::os::windows::process::CommandExt;
    use tray_icon::{
        TrayIconBuilder, TrayIconEvent,
        menu::{Menu, MenuItem, PredefinedMenuItem, MenuEvent},
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage, WM_QUIT,
    };

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

    pub fn run(ipc_port: u16) -> Result<()> {
        maybe_start_daemon(ipc_port);

        // ── Build context menu ────────────────────────────────────────────────
        let open_item   = MenuItem::new("Open TUI",          true, None);
        let update_item = MenuItem::new("Check for Updates", true, None);
        let exit_item   = MenuItem::new("Exit rscapt",       true, None);

        let menu = Menu::new();
        menu.append_items(&[
            &open_item,
            &PredefinedMenuItem::separator(),
            &update_item,
            &PredefinedMenuItem::separator(),
            &exit_item,
        ])
        .context("building tray menu")?;

        let open_id   = open_item.id().clone();
        let update_id = update_item.id().clone();
        let exit_id   = exit_item.id().clone();

        // ── Load icon from embedded Win32 resource (IDI_ICON1 = 1) ───────────
        let icon = tray_icon::Icon::from_resource(1, Some((32, 32)))
            .context("loading icon from embedded resource")?;

        // ── Create tray icon ──────────────────────────────────────────────────
        let _tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("rscapt")
            .with_icon(icon)
            .build()
            .context("creating tray icon")?;

        // ── Event / message loop ──────────────────────────────────────────────
        loop {
            // Menu click events
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id == exit_id {
                    // Kill daemon + tray (taskkill will get us too)
                    let _ = std::process::Command::new("taskkill")
                        .args(["/F", "/IM", "rscapt.exe"])
                        .creation_flags(CREATE_NO_WINDOW)
                        .spawn();
                    std::process::exit(0);
                } else if event.id == open_id {
                    spawn_tui();
                } else if event.id == update_id {
                    spawn_update();
                }
            }

            // Tray icon events — double-click opens TUI
            while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                if matches!(event, TrayIconEvent::DoubleClick { .. }) {
                    spawn_tui();
                }
            }

            // Win32 message pump (required for tray-icon's internal hidden window)
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                    if msg.message == WM_QUIT {
                        return Ok(());
                    }
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    /// Probe port; if nothing is listening, spawn the daemon with no window.
    fn maybe_start_daemon(port: u16) {
        if std::net::TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let _ = std::process::Command::new(exe)
            .arg("daemon")
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
    }

    /// Spawn `rscapt tui` in Windows Terminal, falling back to a new console window.
    fn spawn_tui() {
        let Ok(exe) = std::env::current_exe() else { return };
        // Prefer Windows Terminal — WT_SESSION will be set automatically so
        // setup_tui_console() won't loop. Fall back to conhost if wt unavailable.
        let wt = std::process::Command::new("wt.exe")
            .arg("--window").arg("new")
            .arg(&exe).arg("tui")
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        if wt.is_err() {
            let _ = std::process::Command::new(&exe)
                .arg("tui")
                .creation_flags(CREATE_NEW_CONSOLE)
                .spawn();
        }
    }

    /// Spawn `rscapt update` with its own console window.
    fn spawn_update() {
        let Ok(exe) = std::env::current_exe() else { return };
        let _ = std::process::Command::new(exe)
            .arg("update")
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn();
    }
}

#[cfg(windows)]
pub use imp::run;

#[cfg(not(windows))]
pub fn run(_ipc_port: u16) -> anyhow::Result<()> {
    anyhow::bail!("tray is only supported on Windows")
}
