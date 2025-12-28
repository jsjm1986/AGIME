use super::SystemAutomation;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x08000000;

#[allow(dead_code)]
pub struct WindowsAutomation;

impl SystemAutomation for WindowsAutomation {
    fn execute_system_script(&self, script: &str) -> std::io::Result<String> {
        let output = Command::new("powershell")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(script)
            .env("AGIME_TERMINAL", "1")
            .creation_flags(CREATE_NO_WINDOW)
            .output()?;

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn get_shell_command(&self) -> (&'static str, &'static str) {
        ("powershell", "-Command")
    }

    fn get_temp_path(&self) -> PathBuf {
        std::env::var("TEMP")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(r"C:\Windows\Temp"))
    }
}
