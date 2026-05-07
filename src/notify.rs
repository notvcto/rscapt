/// Send a Windows toast notification.
/// On non-Windows builds this logs instead.
///
/// Unpackaged Win32 apps require a registered AUMID to send toasts via WinRT directly.
/// We avoid that registration requirement by delegating to PowerShell, which has its
/// own registered AUMID and is always present on Windows 10+.
pub fn toast(title: &str, body: &str) {
    #[cfg(windows)]
    {
        // Escape single quotes in title/body to avoid breaking the PS string literals
        let title = title.replace('\'', "''");
        let body = body.replace('\'', "''");

        // PowerShell's AUMID — always registered, no setup required
        let aumid = r"{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe";

        let script = format!(
            r#"
$app = '{aumid}'
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
$xml = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent('ToastText02')
$xml.GetElementsByTagName('text').Item(0).InnerText = '{title}'
$xml.GetElementsByTagName('text').Item(1).InnerText = '{body}'
$notifier = [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app)
$notifier.Show([Windows.UI.Notifications.ToastNotification]::new($xml))
"#
        );

        let result = std::process::Command::new("powershell")
            .args([
                "-WindowStyle", "Hidden",
                "-NonInteractive",
                "-NoProfile",
                "-Command", &script,
            ])
            .spawn();

        if let Err(e) = result {
            tracing::warn!("Failed to spawn toast notification: {e}");
        }
    }

    #[cfg(not(windows))]
    {
        tracing::info!("[notify] {title}: {body}");
    }
}
