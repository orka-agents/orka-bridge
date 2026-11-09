//! Native-owned help destinations. Renderer input selects a topic, never a URL.
#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HelpTopic {
    Virtualization,
    Hypervisor,
    Workspace,
}
impl HelpTopic {
    fn url(self) -> &'static str {
        match self {
            Self::Virtualization => "https://support.microsoft.com/en-us/windows/experience/enable-virtualization-on-windows",
            Self::Hypervisor => "https://learn.microsoft.com/en-us/dotnet/maui/android/emulator/hardware-acceleration?view=net-maui-10.0",
            Self::Workspace => "https://github.com/orka-agents/orka-bridge/blob/main/CONTRIBUTING.md",
        }
    }
}
pub fn open_help(topic: HelpTopic) -> Result<(), &'static str> {
    let url = topic.url();
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        let url: Vec<_> = url.encode_utf16().chain(Some(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                std::ptr::null(),
                url.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if result as isize > 32 {
            return Ok(());
        }
    }
    #[cfg(target_os = "macos")]
    if std::process::Command::new("/usr/bin/open")
        .arg(url)
        .status()
        .is_ok_and(|status| status.success())
    {
        return Ok(());
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = url;
    Err("Could not open setup help.")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn help_accepts_only_fixed_topics_and_routes_to_owned_destinations() {
        for (topic, expected) in [
            ("virtualization", "https://support.microsoft.com/en-us/windows/experience/enable-virtualization-on-windows"),
            ("hypervisor", "https://learn.microsoft.com/en-us/dotnet/maui/android/emulator/hardware-acceleration?view=net-maui-10.0"),
            ("workspace", "https://github.com/orka-agents/orka-bridge/blob/main/CONTRIBUTING.md"),
        ] {
            let topic: HelpTopic = serde_json::from_value(serde_json::json!(topic)).unwrap();
            assert_eq!(topic.url(), expected);
        }
        for input in [
            "https://example.com",
            "file:///etc/passwd",
            "workspace;calc.exe",
            "unknown",
            "",
        ] {
            assert!(serde_json::from_value::<HelpTopic>(serde_json::json!(input)).is_err());
        }
    }
}
