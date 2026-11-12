//! Zed 扩展壳：注册 rest-nav 语言服务器（Java），按平台下载/定位 LSP 二进制并启动。
//! 真正的接口扫描与 workspace/symbol 由原生二进制 rest-nav-lsp 完成。

use zed_extension_api as zed;
use zed::{
    current_platform, download_file, latest_github_release, make_file_executable,
    set_language_server_installation_status, Architecture, Command, DownloadedFileType,
    GithubReleaseOptions, LanguageServerInstallationStatus, LanguageServerId, Os, Worktree,
};

/// 发布 LSP 二进制的 GitHub 仓库（<owner>/<repo>）。发布前请改成你自己的仓库。
const REPO: &str = "zhy-ty/spring-rest-lsp";

struct RestNavExtension;

impl RestNavExtension {
    /// 本地覆盖：环境变量 REST_NAV_LSP，或扩展工作目录下的 rest-nav-lsp-dev[.exe]。
    /// 便于开发调试，或 GitHub 不可达时使用自构建二进制。
    fn local_override(is_windows: bool) -> Option<String> {
        if let Ok(p) = std::env::var("REST_NAV_LSP") {
            if !p.is_empty() {
                return Some(p);
            }
        }
        if let Ok(dir) = std::env::current_dir() {
            let name = if is_windows { "rest-nav-lsp-dev.exe" } else { "rest-nav-lsp-dev" };
            let p = dir.join(name);
            if p.exists() {
                return Some(p.to_string_lossy().into_owned());
            }
        }
        None
    }
}

impl zed::Extension for RestNavExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        _worktree: &Worktree,
    ) -> zed::Result<Command> {
        let (os, arch) = current_platform();
        let is_windows = matches!(os, Os::Windows);

        // 1) 本地覆盖优先
        if let Some(cmd) = Self::local_override(is_windows) {
            return Ok(Command { command: cmd, args: vec![], env: vec![] });
        }

        // 2) 平台 -> release 资产名
        let target = match (os, arch) {
            (Os::Windows, Architecture::X8664) => "x86_64-pc-windows-msvc",
            (Os::Linux, Architecture::X8664) => "x86_64-unknown-linux-gnu",
            (Os::Linux, Architecture::Aarch64) => "aarch64-unknown-linux-gnu",
            (Os::Mac, Architecture::X8664) => "x86_64-apple-darwin",
            (Os::Mac, Architecture::Aarch64) => "aarch64-apple-darwin",
            (os, arch) => return Err(format!("unsupported platform: {os:?}/{arch:?}")),
        };
        let asset_name = format!("rest-nav-lsp-{target}.gz");

        // 3) 最新 release
        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::CheckingForUpdate,
        );
        let release = latest_github_release(
            REPO,
            GithubReleaseOptions { require_assets: true, pre_release: false },
        )
        .map_err(|e| format!("failed to fetch latest release of {REPO}: {e}"))?;

        let asset = release
            .assets
            .iter()
            .find(|a| a.name == asset_name)
            .ok_or_else(|| format!("asset {asset_name} not found in release {}", release.version))?;

        // 4) 下载到扩展工作目录并按版本缓存
        let dir = std::env::current_dir().map_err(|e| e.to_string())?;
        let suffix = if is_windows { ".exe" } else { "" };
        let bin_path = dir.join(format!("rest-nav-lsp-{}{}", release.version, suffix));

        if !bin_path.exists() {
            set_language_server_installation_status(
                language_server_id,
                &LanguageServerInstallationStatus::Downloading,
            );
            let dest = bin_path.to_string_lossy().into_owned();
            download_file(&asset.download_url, &dest, DownloadedFileType::Gzip)
                .map_err(|e| format!("failed to download {asset_name}: {e}"))?;
            make_file_executable(&dest).map_err(|e| e.to_string())?;
        }

        set_language_server_installation_status(
            language_server_id,
            &LanguageServerInstallationStatus::None,
        );

        Ok(Command {
            command: bin_path.to_string_lossy().into_owned(),
            args: vec![],
            env: vec![],
        })
    }
}

zed::register_extension!(RestNavExtension);