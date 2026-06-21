mod modules;

use modules::{FileInfo, FileScanner, ScanResult, SoftwareManager};
use std::env;

fn main() {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("chcp")
            .args(["65001"])
            .creation_flags(0x08000000)
            .spawn();
    }

    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage();
        return;
    }

    match args[1].as_str() {
        "scan" => {
            handle_scan(&args);
        }
        "software" => {
            handle_software(&args);
        }
        "clean" => {
            handle_clean(&args);
        }
        "analyze" => {
            handle_analyze(&args);
        }
        "help" => {
            print_usage();
        }
        _ => {
            println!("未知命令: {}", args[1]);
            print_usage();
        }
    }
}

fn print_usage() {
    println!("Smart PC Manager - 智能电脑管理器");
    println!();
    println!("用法:");
    println!("  cargo run -- scan [路径]          扫描指定目录");
    println!("  cargo run -- scan --all           扫描所有驱动器");
    println!("  cargo run -- software              查看已安装软件");
    println!("  cargo run -- software <关键词>     搜索软件");
    println!("  cargo run -- analyze [路径]       分析软件关联文件");
    println!("  cargo run -- clean [路径]          删除重复文件");
    println!("  cargo run -- clean --all           删除所有驱动器的重复文件");
    println!("  cargo run -- help                 显示帮助");
    println!();
    println!("示例:");
    println!("  cargo run -- scan C:\\Users");
    println!("  cargo run -- scan D:\\");
    println!("  cargo run -- scan --all");
    println!("  cargo run -- software");
    println!("  cargo run -- software rust");
    println!("  cargo run -- analyze C:\\Program Files");
    println!("  cargo run -- clean D:\\Downloads");
    println!("  cargo run -- clean --all");
}

fn handle_scan(args: &[String]) {
    let scanner = FileScanner::new();

    println!("========================================");
    println!("Smart PC Manager - 文件扫描");
    println!("========================================");
    println!();

    if args.len() >= 3 && args[2] == "--all" {
        match FileScanner::get_drives() {
            Ok(drives) => {
                println!("发现 {} 个驱动器:", drives.len());
                for drive in &drives {
                    println!("  - {}", drive);
                }
                println!();

                let mut all_results: Vec<ScanResult> = Vec::new();

                for drive in &drives {
                    println!("正在扫描驱动器: {}", drive);
                    match scanner.scan(drive) {
                        Ok(result) => {
                            all_results.push(result);
                        }
                        Err(e) => {
                            eprintln!("警告: 扫描驱动器 {} 失败: {}", drive, e);
                        }
                    }
                }

                let total_files: usize = all_results.iter().map(|r| r.total_files).sum();
                let total_size: u64 = all_results.iter().map(|r| r.total_size).sum();
                let total_large: usize = all_results.iter().map(|r| r.large_files.len()).sum();
                let total_duplicates: usize = all_results.iter().map(|r| r.duplicate_groups.len()).sum();

                println!("\n📊 所有驱动器扫描结果统计:");
                println!("----------------------------------------");
                println!("总文件数: {}", total_files);
                println!("总大小: {} ({} GB)",
                    format_size(total_size),
                    total_size as f64 / 1024.0 / 1024.0 / 1024.0
                );
                println!("大文件数: {}", total_large);
                println!("重复文件组数: {}", total_duplicates);
                println!("\n✅ 所有驱动器扫描完成！");
            }
            Err(e) => {
                eprintln!("❌ 获取驱动器列表失败: {}", e);
            }
        }
    } else {
        let path = if args.len() >= 3 {
            args[2].clone()
        } else {
            env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        };

        match scanner.scan(&path) {
            Ok(result) => {
                println!("\n📊 扫描结果统计:");
                println!("----------------------------------------");
                println!("总文件数: {}", result.total_files);
                println!("总大小: {} ({} GB)",
                    format_size(result.total_size),
                    result.total_size as f64 / 1024.0 / 1024.0 / 1024.0
                );
                println!("大文件数: {}", result.large_files.len());
                println!("重复文件组数: {}", result.duplicate_groups.len());

                if !result.large_files.is_empty() {
                    println!("\n📁 大文件列表 (前10个):");
                    println!("----------------------------------------");
                    let mut sorted_large: Vec<_> = result.large_files.clone();
                    sorted_large.sort_by(|a, b| b.size.cmp(&a.size));

                    for (i, file) in sorted_large.iter().take(10).enumerate() {
                        println!("{}. {} - {}", i + 1, format_size(file.size), file.path);
                    }
                }

                if !result.duplicate_groups.is_empty() {
                    println!("\n🔄 重复文件列表 (前5组):");
                    println!("----------------------------------------");
                    for (i, group) in result.duplicate_groups.iter().take(5).enumerate() {
                        println!("\n组 {} ({} 个相同文件):", i + 1, group.len());
                        for file in group {
                            println!("  - {} ({})", file.path, format_size(file.size));
                        }
                    }
                }

                println!("\n✅ 扫描完成！");
            }
            Err(e) => {
                eprintln!("❌ 扫描失败: {}", e);
            }
        }
    }
}

fn handle_software(args: &[String]) {
    let manager = SoftwareManager::new();

    println!("========================================");
    println!("Smart PC Manager - 软件管理");
    println!("========================================");
    println!();

    if args.len() >= 3 {
        // 搜索软件
        let keyword = &args[2];
        println!("搜索关键词: {}", keyword);
        println!();

        match manager.search_software(keyword) {
            Ok(results) => {
                if results.is_empty() {
                    println!("未找到匹配的软件");
                } else {
                    println!("找到 {} 个匹配的软件:", results.len());
                    println!();
                    display_software_list(&results);
                }
            }
            Err(e) => {
                eprintln!("❌ 搜索失败: {}", e);
            }
        }
    } else {
        // 显示所有软件
        match manager.get_installed_software() {
            Ok(software_list) => {
                println!("已安装软件总数: {}", software_list.len());
                println!();
                display_software_list(&software_list);
            }
            Err(e) => {
                eprintln!("❌ 获取软件列表失败: {}", e);
            }
        }
    }
}

fn handle_analyze(args: &[String]) {
    let scanner = FileScanner::new();
    let software_manager = SoftwareManager::new();

    println!("========================================");
    println!("Smart PC Manager - 软件文件分析");
    println!("========================================");
    println!();

    let path = if args.len() >= 3 {
        args[2].clone()
    } else {
        env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| ".".to_string())
    };

    println!("正在扫描目录: {}", path);
    println!("正在获取软件列表...");
    println!();

    // 扫描获取所有文件
    match scanner.scan(&path) {
        Ok(result) => {
            let all_files = result.all_files;
            let total_size: u64 = all_files.iter().map(|f| f.size).sum();

            println!("扫描完成: {} 个文件, 总大小: {}", all_files.len(), format_size(total_size));
            println!();

            // 限制分析的文件数量以避免性能问题
            let files_to_analyze: Vec<FileInfo> = if all_files.len() > 10000 {
                println!("文件数量较多，仅分析前10000个文件...");
                all_files.into_iter().take(10000).collect()
            } else {
                all_files
            };

            // 分析文件与软件的关联
            let analysis = software_manager.analyze_files_with_software(&files_to_analyze);

            if analysis.is_empty() {
                println!("未找到与已安装软件关联的文件");
            } else {
                println!("========================================");
                println!("软件文件分析结果 (按大小排序)");
                println!("========================================");
                println!();

                let total_matched: usize = analysis.iter().map(|a| a.file_count).sum();
                let total_matched_size: u64 = analysis.iter().map(|a| a.total_size).sum();

                println!("总计: 找到 {} 个软件关联文件, 总大小: {}",
                    total_matched, format_size(total_matched_size));
                println!();

                for (i, item) in analysis.iter().take(20).enumerate() {
                    println!("{}. {} ({})", i + 1, item.software.name, format_size(item.total_size));
                    if let Some(ref loc) = item.software.install_location {
                        println!("   安装路径: {}", loc);
                    }
                    println!("   文件数: {}, 大小: {}", item.file_count, format_size(item.total_size));

                    // 显示部分文件
                    let display_files = item.files.iter().take(3);
                    for f in display_files {
                        let file_name = std::path::Path::new(&f.path)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| f.path.clone());
                        println!("   - {} ({})", file_name, format_size(f.size));
                    }
                    if item.files.len() > 3 {
                        println!("   ... 还有 {} 个文件", item.files.len() - 3);
                    }
                    println!();
                }

                if analysis.len() > 20 {
                    println!("... 还有 {} 个软件未显示", analysis.len() - 20);
                }
            }
        }
        Err(e) => {
            eprintln!("❌ 扫描失败: {}", e);
        }
    }
}

fn handle_clean(args: &[String]) {
    let scanner = FileScanner::new();

    println!("========================================");
    println!("Smart PC Manager - 删除重复文件");
    println!("========================================");
    println!();

    if args.len() >= 3 && args[2] == "--all" {
        match FileScanner::get_drives() {
            Ok(drives) => {
                println!("发现 {} 个驱动器:", drives.len());
                for drive in &drives {
                    println!("  - {}", drive);
                }
                println!();

                println!("\n⚠️  警告: 此操作将扫描并删除所有驱动器上的重复文件");
                println!("请确认是否继续? (Y/N)");

                let mut input = String::new();
                std::io::stdin().read_line(&mut input).unwrap_or_default();
                let input = input.trim().to_lowercase();

                if input != "y" && input != "yes" {
                    println!("\n❌ 操作已取消");
                    return;
                }

                let mut total_deleted = 0;
                let mut total_freed: u64 = 0;

                for drive in &drives {
                    println!("\n正在扫描驱动器: {}", drive);
                    match scanner.scan(drive) {
                        Ok(result) => {
                            if !result.duplicate_groups.is_empty() {
                                match scanner.delete_duplicates(&result.duplicate_groups, true) {
                                    Ok((count, freed)) => {
                                        total_deleted += count;
                                        total_freed += freed;
                                    }
                                    Err(e) => {
                                        eprintln!("警告: 删除驱动器 {} 的重复文件失败: {}", drive, e);
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("警告: 扫描驱动器 {} 失败: {}", drive, e);
                        }
                    }
                }

                println!("\n✅ 所有驱动器清理完成!");
                println!("删除文件数: {}", total_deleted);
                println!("释放空间: {}", format_size(total_freed));
            }
            Err(e) => {
                eprintln!("❌ 获取驱动器列表失败: {}", e);
            }
        }
    } else {
        let path = if args.len() >= 3 {
            args[2].clone()
        } else {
            env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| ".".to_string())
        };

        match scanner.scan(&path) {
            Ok(result) => {
                if result.duplicate_groups.is_empty() {
                    println!("✅ 未发现重复文件");
                    return;
                }

                println!("发现 {} 组重复文件", result.duplicate_groups.len());
                println!();

                for (i, group) in result.duplicate_groups.iter().take(3).enumerate() {
                    println!("组 {} ({} 个相同文件):", i + 1, group.len());
                    for file in group {
                        println!("  - {} ({})", file.path, format_size(file.size));
                    }
                }

                if result.duplicate_groups.len() > 3 {
                    println!("\n... 还有 {} 组未显示", result.duplicate_groups.len() - 3);
                }

                println!("\n⚠️  警告: 此操作将删除重复文件，保留每组中的第一个文件");
                println!("请确认是否继续? (Y/N)");

                let mut input = String::new();
                std::io::stdin().read_line(&mut input).unwrap_or_default();
                let input = input.trim().to_lowercase();

                if input == "y" || input == "yes" {
                    match scanner.delete_duplicates(&result.duplicate_groups, true) {
                        Ok((count, freed)) => {
                            println!("\n✅ 删除成功!");
                            println!("删除文件数: {}", count);
                            println!("释放空间: {}", format_size(freed));
                        }
                        Err(e) => {
                            eprintln!("❌ 删除失败: {}", e);
                        }
                    }
                } else {
                    println!("\n❌ 操作已取消");
                }
            }
            Err(e) => {
                eprintln!("❌ 扫描失败: {}", e);
            }
        }
    }
}

fn display_software_list(software_list: &[modules::SoftwareInfo]) {
    println!("{:<50} {:<15} {:<20}", "软件名称", "版本", "发布者");
    println!("{}", "-".repeat(85));

    for software in software_list.iter().take(50) {
        let name = if software.name.len() > 48 {
            format!("{}...", &software.name[..45])
        } else {
            software.name.clone()
        };

        let version = software.version.clone().unwrap_or_else(|| "-".to_string());
        let publisher = software.publisher.clone().unwrap_or_else(|| "-".to_string());

        println!("{:<50} {:<15} {:<20}", name, version, publisher);
    }

    
    if software_list.len() > 50 {
        println!("\n... 还有 {} 个软件未显示", software_list.len() - 50);
    }
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
 