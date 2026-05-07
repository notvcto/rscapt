use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

// ── Public entry points ───────────────────────────────────────────────────────

/// Copy the binary to the install location, create Start Menu shortcut,
/// register the daemon as a silent autostart, and add to user PATH.
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

    let vbs_path = write_vbs_launcher(&install_dir)
        .context("writing VBS launcher")?;

    run_ps(&install_ps_script(&dest_exe, &vbs_path, &install_dir))
        .context("running PowerShell install script")?;

    println!("[install] Start Menu shortcut created");
    println!("[install] Autostart registered (rscapt tray will start silently on next login)");
    println!("[install] {} added to user PATH", install_dir.display());
    println!();
    println!("Done. Log out and back in, or run `rscapt daemon` now to start immediately.");

    Ok(())
}

/// Remove the Start Menu shortcut, autostart entry, and PATH entry.
/// Schedules the install directory for deletion on next reboot since the
/// running binary is locked by Windows.
pub fn uninstall() -> Result<()> {
    let install_dir = install_dir();

    run_ps(&uninstall_ps_script(&install_dir))
        .context("running PowerShell uninstall script")?;

    println!("[uninstall] Start Menu shortcut removed");
    println!("[uninstall] Autostart entry removed");
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

// ── VBS silent launcher ───────────────────────────────────────────────────────

/// Write a VBS file that launches `rscapt tray` with no visible window.
/// Returns the path to the written file.
fn write_vbs_launcher(dir: &Path) -> Result<PathBuf> {
    let vbs_path = dir.join("rscapt-start.vbs");
    let exe_path = dir.join("rscapt.exe");
    // Escape any embedded quotes in the path (extremely rare but correct)
    let exe_str = exe_path.to_string_lossy().replace('"', "\"\"");
    let content = format!(
        "CreateObject(\"WScript.Shell\").Run \"\"\"{exe_str}\"\" tray\", 0, False\n"
    );
    std::fs::write(&vbs_path, &content).context("writing VBS launcher")?;
    Ok(vbs_path)
}

// ── PowerShell scripts ────────────────────────────────────────────────────────

fn install_ps_script(exe: &Path, vbs: &Path, install_dir: &Path) -> String {
    let exe = ps_str(exe.to_string_lossy().as_ref());
    let vbs = ps_str(vbs.to_string_lossy().as_ref());
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
Set-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'rscapt' -Value ('wscript.exe "' + '{vbs}' + '"') -ErrorAction Stop
$cur = [Environment]::GetEnvironmentVariable('PATH', 'User')
if ($null -eq $cur) {{ $cur = '' }}
if ($cur -notlike '*{dir}*') {{ [Environment]::SetEnvironmentVariable('PATH', ($cur + ';' + '{dir}'), 'User') }}
"#
    )
}

fn uninstall_ps_script(install_dir: &Path) -> String {
    let dir = ps_str(install_dir.to_string_lossy().as_ref());

    format!(
        r#"
$lnk = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\rscapt.lnk"
if (Test-Path $lnk) {{ Remove-Item $lnk -Force -ErrorAction SilentlyContinue }}
Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'rscapt' -ErrorAction SilentlyContinue
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
    let output = std::process::Command::new("powershell")
        .args([
            "-WindowStyle",    "Hidden",
            "-NonInteractive",
            "-NoProfile",
            "-Command",        script,
        ])
        .output()
        .context("spawning PowerShell")?;

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
