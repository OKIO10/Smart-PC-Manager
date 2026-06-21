use std::process::Command;

use super::file_scanner::FileInfo;

#[derive(Debug, Clone)]
pub struct SoftwareFileAnalysis {
    pub software: SoftwareInfo,
    pub files: Vec<FileInfo>,
    pub total_size: u64,
    pub file_count: usize,
}

#[derive(Debug, Clone)]
pub struct SoftwareInfo {
    pub name: String,
    pub version: Option<String>,
    pub publisher: Option<String>,
    pub install_location: Option<String>,
}

pub struct SoftwareManager;

impl SoftwareManager {
    pub fn new() -> Self {
        Self
    }

    pub fn analyze_files_with_software(&self, files: &[FileInfo]) -> Vec<SoftwareFileAnalysis> {
        let software_list = match self.get_installed_software() {
            Ok(list) => list,
            Err(_) => return Vec::new(),
        };

        let mut analysis: Vec<SoftwareFileAnalysis> = Vec::new();

        for software in &software_list {
            if let Some(ref install_loc) = software.install_location {
                if install_loc.is_empty() {
                    continue;
                }

                let install_path_lower = install_loc.to_lowercase();
                let matching_files: Vec<&FileInfo> = files
                    .iter()
                    .filter(|f| {
                        let file_path_lower = f.path.to_lowercase();
                        file_path_lower.starts_with(&install_path_lower)
                    })
                    .collect();

                if !matching_files.is_empty() {
                    let file_count = matching_files.len();
                    let total_size: u64 = matching_files.iter().map(|f| f.size).sum();
                    analysis.push(SoftwareFileAnalysis {
                        software: software.clone(),
                        files: matching_files.into_iter().cloned().collect(),
                        total_size,
                        file_count,
                    });
                }
            }
        }

        analysis.sort_by(|a, b| b.total_size.cmp(&a.total_size));
        analysis
    }

    pub fn get_installed_software(&self) -> Result<Vec<SoftwareInfo>, String> {
        println!("正在获取已安装软件列表...");

        #[cfg(windows)]
        {
            let ps_script = r#"
$paths = @(
    'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*',
    'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*',
    'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*'
)

foreach ($path in $paths) {
    Get-ItemProperty $path -ErrorAction SilentlyContinue |
    Where-Object { $_.DisplayName -and $_.DisplayName -notmatch '^KB\d+' -and $_.DisplayName -notmatch 'Security Update' } |
    ForEach-Object {
        $name = $_.DisplayName
        $version = if ($_.DisplayVersion) { $_.DisplayVersion } else { '' }
        $publisher = if ($_.Publisher) { $_.Publisher } else { '' }
        $location = if ($_.InstallLocation) { $_.InstallLocation } else { '' }
        Write-Output "$name|$version|$publisher|$location"
    }
}
"#;

            let output = Command::new("powershell")
                .args(["-NoProfile", "-Command", ps_script])
                .output()
                .map_err(|e| format!("执行 PowerShell 命令失败: {}", e))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(format!("PowerShell 执行失败: {}", stderr));
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stdout_trimmed = stdout.trim();

            if stdout_trimmed.is_empty() {
                return Ok(Vec::new());
            }

            let software_list = parse_software_text(stdout_trimmed)?;

            println!("找到 {} 个已安装软件", software_list.len());
            Ok(software_list)
        }

        #[cfg(not(windows))]
        {
            Err("此功能仅在 Windows 系统上可用".to_string())
        }
    }

    pub fn search_software(&self, keyword: &str) -> Result<Vec<SoftwareInfo>, String> {
        let all_software = self.get_installed_software()?;
        let keyword_lower = keyword.to_lowercase();

        let filtered: Vec<SoftwareInfo> = all_software
            .into_iter()
            .filter(|s| {
                s.name.to_lowercase().contains(&keyword_lower)
                    || s.publisher
                        .as_ref()
                        .map(|p| p.to_lowercase().contains(&keyword_lower))
                        .unwrap_or(false)
            })
            .collect();

        Ok(filtered)
    }
}

impl Default for SoftwareManager {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_software_text(text: &str) -> Result<Vec<SoftwareInfo>, String> {
    let mut software_list = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split('|').collect();

        if parts.len() >= 1 {
            let name = parts[0].to_string();
            if name.is_empty() || name == " " {
                continue;
            }

            let version = if parts.len() > 1 && !parts[1].is_empty() {
                Some(parts[1].to_string())
            } else {
                None
            };

            let publisher = if parts.len() > 2 && !parts[2].is_empty() {
                Some(parts[2].to_string())
            } else {
                None
            };

            let install_location = if parts.len() > 3 && !parts[3].is_empty() {
                let loc = parts[3].trim().to_string();
                if loc.is_empty() {
                    None
                } else {
                    Some(loc)
                }
            } else {
                None
            };

            software_list.push(SoftwareInfo {
                name,
                version,
                publisher,
                install_location,
            });
        }
    }

    software_list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    software_list.dedup_by(|a, b| a.name.to_lowercase() == b.name.to_lowercase());

    Ok(software_list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_installed_software() {
        let manager = SoftwareManager::new();
        let result = manager.get_installed_software();
        assert!(result.is_ok());
    }

    #[test]
    fn test_search_software() {
        let manager = SoftwareManager::new();
        let result = manager.search_software("rust");
        assert!(result.is_ok());
    }
}
