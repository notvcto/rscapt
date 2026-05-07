use anyhow::{Context, Result};
use std::path::PathBuf;

// ── Public entry points ───────────────────────────────────────────────────────

/// Copy the binary to the install location, create Start Menu shortcut,
/// register the daemon as a startup shortcut, and add to user PATH.
pub fn install() -> Result<()> {
    let install_dir = install_dir();
    let dest_exe = install_dir.join("rscapt.exe");
    let src_exe = std::env::current_exe().context("resolving current exe path")?;
    // Canonicalise so the comparison works even with symlinks / UNC paths
    let src_canon = std::fs::canonicalize(&src_exe).unwrap_or(src_exe);
    let dst_canon = if dest_exe.exists() {
        std::fs::canonicalize(&dest_exe).unwrap_or(dest_exe.clone())
    } else {
        dest_exe.clone()
    };

    std::fs::create_dir_all(&install_dir)
        .context("creating install directory")?;

    if src_canon != dst_canon {
        std::fs::copy(&src_canon, &dest_exe)
            .context("copying binary to install directory")?;
        println!("[install] Binary → {}", dest_exe.display());
    } else {
        println!("[install] Binary already at install location — skipping copy");
    }

    run_ps(&install_ps_script(&dest_exe, &install_dir))
        .context("running PowerShell install script")?;

    println!("[install] Start Menu shortcut created");
    println!("[install] Startup shortcut created (rscapt tray will start on next login)");
    println!("[install] {} added to user PATH", install_dir.display());
    println!();
    println!("Done. Log out and back in, or run `rscapt tray` now to start immediately.");

    Ok(())
}

/// Remove the Start Menu shortcut, startup entry, and PATH entry.
/// Schedules the install directory for deletion on next reboot since the
/// running binary is locked by Windows.
pub fn uninstall() -> Result<()> {
    let install_dir = install_dir();

    run_ps(&uninstall_ps_script(&install_dir))
        .context("running PowerShell uninstall script")?;

    println!("[uninstall] Start Menu shortcut removed");
    println!("[uninstall] Startup shortcut removed");
    println!("[uninstall] Removed from user PATH");
    println!();
    println!(
        "The install directory ({}) will be deleted on next login.",
        install_dir.display()
    );
    println!(
        "Config and clip data in {} is untouched. Delete it manually if you want a full clean.",
        crate::config::Config::data_dir().display()
    );

    Ok(())
}

// ── Paths ─────────────────────────────────────────────────────────────────────

fn install_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rscapt")
}

// ── PowerShell scripts ────────────────────────────────────────────────────────

fn install_ps_script(exe: &std::path::Path, install_dir: &std::path::Path) -> String {
    let exe = ps_str(exe.to_string_lossy().as_ref());
    let dir = ps_str(install_dir.to_string_lossy().as_ref());

    format!(
        r#"
$shell = New-Object -ComObject WScript.Shell
$lnk = $shell.CreateShortcut("$env:APPDATA\Microsoft\Windows\Start Menu\Programs\rscapt.lnk")
$lnk.TargetPath = '{exe}'
$lnk.Arguments = 'tui'
$lnk.Description = 'rscapt - OBS replay clip processor'
$lnk.IconLocation = '{exe},0'
$lnk.Save()
$startup = $shell.CreateShortcut("$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Startup\rscapt.lnk")
$startup.TargetPath = '{exe}'
$startup.Arguments = 'tray'
$startup.Description = 'rscapt tray'
$startup.IconLocation = '{exe},0'
$startup.Save()
$cur = [Environment]::GetEnvironmentVariable('PATH', 'User')
if ($null -eq $cur) {{ $cur = '' }}
if ($cur -notlike '*{dir}*') {{ [Environment]::SetEnvironmentVariable('PATH', ($cur + ';' + '{dir}'), 'User') }}
"#
    )
}

fn uninstall_ps_script(install_dir: &std::path::Path) -> String {
    let dir = ps_str(install_dir.to_string_lossy().as_ref());

    format!(
        r#"
$lnk = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\rscapt.lnk"
if (Test-Path $lnk) {{ Remove-Item $lnk -Force -ErrorAction SilentlyContinue }}
$startup = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Startup\rscapt.lnk"
if (Test-Path $startup) {{ Remove-Item $startup -Force -ErrorAction SilentlyContinue }}
$cur = [Environment]::GetEnvironmentVariable('PATH', 'User')
if ($null -ne $cur) {{ [Environment]::SetEnvironmentVariable('PATH', (($cur -split ';' | Where-Object {{ $_ -ne '{dir}' }}) -join ';'), 'User') }}
Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\RunOnce' -Name 'rscapt-cleanup' -Value ('cmd /c rmdir /s /q "' + '{dir}' + '"') -ErrorAction SilentlyContinue
"#
    )
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Escape a string for use inside PowerShell single-quoted strings.
fn ps_str(s: &str) -> String {
    s.replace('\'', "''")
}

fn run_ps(script: &str) -> Result<()> {
    #[allow(unused_mut)]
    let mut cmd = std::process::Command::new("powershell");
    cmd.args([
        "-WindowStyle",    "Hidden",
        "-NonInteractive",
        "-NoProfile",
        "-Command",        script,
    ]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = cmd.output().context("spawning PowerShell")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        anyhow::bail!(
            "PowerShell exited {}\nstdout: {stdout}\nstderr: {stderr}",
            output.status.code().unwrap_or(-1)
        );
    }

    Ok(())
}
