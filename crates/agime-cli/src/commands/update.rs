use std::process::Command;

use anyhow::Result;

// TODO: Update repository to final AGIME GitHub location
const DOWNLOAD_SCRIPT_URL: &str =
    "https://github.com/fengrui198609/agime/releases/download/stable/download_cli.sh";

pub fn update(canary: bool, reconfigure: bool) -> Result<()> {
    // Windows does not support bash-based update script
    #[cfg(windows)]
    {
        eprintln!("自动更新在 Windows 上暂不支持。");
        eprintln!("请访问 https://github.com/fengrui198609/agime/releases 手动下载更新。");
        return Ok(());
    }

    // Unix-based systems use bash script
    #[cfg(not(windows))]
    {
        // Get the download script from github
        let curl_output = Command::new("curl")
            .arg("-fsSL")
            .arg(DOWNLOAD_SCRIPT_URL)
            .output()?;

        if !curl_output.status.success() {
            anyhow::bail!(
                "Failed to download update script: {}",
                std::str::from_utf8(&curl_output.stderr)?
            );
        }

        let shell_str = std::str::from_utf8(&curl_output.stdout)?;

        let update = Command::new("bash")
            .arg("-c")
            .arg(shell_str)
            .env("CANARY", canary.to_string())
            .env("CONFIGURE", reconfigure.to_string())
            .env("AGIME_TERMINAL", "1")
            .spawn()?;

        update.wait_with_output()?;

        Ok(())
    }
}
