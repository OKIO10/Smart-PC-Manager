use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub path: String,
    pub size: u64,
    pub modified: String,
}

#[derive(Debug)]
pub struct ScanResult {
    pub total_files: usize,
    pub total_size: u64,
    pub all_files: Vec<FileInfo>,
    pub duplicate_groups: Vec<Vec<FileInfo>>,
    pub large_files: Vec<FileInfo>,
}

pub struct FileScanner {
    min_large_file_size: u64,
    check_duplicates: bool,
}

impl FileScanner {
    pub fn new() -> Self {
        Self {
            min_large_file_size: 100 * 1024 * 1024,
            check_duplicates: true,
        }
    }

    pub fn scan(&self, path: &str) -> Result<ScanResult, String> {
        println!("开始扫描目录: {}", path);

        // 多线程并行扫描
        let (files, total_size) = self.scan_parallel(path)?;

        let total_files = files.len();
        println!("扫描完成: {} 个文件, 总大小: {} bytes", total_files, total_size);

        // 提取大文件
        let large_files: Vec<FileInfo> = files
            .iter()
            .filter(|f| f.size >= self.min_large_file_size)
            .cloned()
            .collect();

        println!("发现 {} 个大文件 (>= {} bytes)", large_files.len(), self.min_large_file_size);

        // 检测重复文件
        let duplicate_groups = if self.check_duplicates {
            self.find_duplicates(&files)
        } else {
            Vec::new()
        };

        Ok(ScanResult {
            total_files,
            total_size,
            all_files: files,
            duplicate_groups,
            large_files,
        })
    }

    pub fn delete_duplicates(&self, duplicate_groups: &[Vec<FileInfo>], keep_first: bool) -> Result<(usize, u64), String> {
        let mut deleted_count = 0;
        let mut freed_bytes = 0;

        println!("\n开始删除重复文件...");

        for group in duplicate_groups {
            let files_to_delete = if keep_first {
                &group[1..]
            } else {
                &group[..group.len() - 1]
            };

            for file in files_to_delete {
                match fs::remove_file(&file.path) {
                    Ok(_) => {
                        println!("已删除: {}", file.path);
                        deleted_count += 1;
                        freed_bytes += file.size;
                    }
                    Err(e) => {
                        eprintln!("删除失败 {}: {}", file.path, e);
                    }
                }
            }
        }

        println!("\n删除完成! 共删除 {} 个文件, 释放 {} 字节空间", deleted_count, freed_bytes);
        Ok((deleted_count, freed_bytes))
    }

    #[cfg(windows)]
    pub fn get_drives() -> Result<Vec<String>, String> {
        let mut drives = Vec::new();

        for letter in b'A'..=b'Z' {
            let drive = format!("{}:", char::from(letter));
            let path = PathBuf::from(&drive);
            if path.exists() {
                drives.push(drive);
            }
        }

        Ok(drives)
    }

    #[cfg(not(windows))]
    pub fn get_drives() -> Result<Vec<String>, String> {
        Ok(vec!["/".to_string()])
    }

    // ===== 并行扫描实现 =====
    fn scan_parallel(&self, root: &str) -> Result<(Vec<FileInfo>, u64), String> {
        let num_threads = thread::available_parallelism().map(|p| p.get()).unwrap_or(4).min(8);

        // 收集所有子目录作为工作项
        let root_path = PathBuf::from(root);
        if !root_path.exists() {
            return Err(format!("路径不存在: {}", root));
        }

        // 先处理根目录文件 + 收集子目录，然后用多线程并行处理子目录
        let files_result = Arc::new(Mutex::new(Vec::<FileInfo>::new()));
        let total_size = Arc::new(Mutex::new(0u64));

        // 首先用主线程收集所有目录 + 处理根目录的文件
        let root_dirs = self.collect_directories_and_scan(
            &root_path,
            &files_result,
            &total_size,
        )?;

        if root_dirs.is_empty() {
            // 没有子目录，直接返回
            let files = Arc::try_unwrap(files_result).unwrap().into_inner().unwrap();
            let total = *Arc::try_unwrap(total_size).unwrap().get_mut().unwrap();
            return Ok((files, total));
        }

        // 将目录放入工作队列
        let work_queue: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(root_dirs));

        // 创建工作线程
        let mut handles = Vec::with_capacity(num_threads);
        let (error_tx, error_rx) = mpsc::channel::<String>();

        for _ in 0..num_threads {
            let work_queue = Arc::clone(&work_queue);
            let files_result = Arc::clone(&files_result);
            let total_size = Arc::clone(&total_size);
            let error_tx = error_tx.clone();

            handles.push(thread::spawn(move || {
                worker_scan_thread(work_queue, files_result, total_size, error_tx);
            }));
        }

        drop(error_tx);

        // 等待所有线程完成
        for handle in handles {
            let _ = handle.join();
        }

        // 检查是否有错误
        let mut first_error: Option<String> = None;
        for err in error_rx.iter() {
            if first_error.is_none() {
                first_error = Some(err);
            }
        }

        // 收集结果
        let files = Arc::try_unwrap(files_result)
            .map_err(|_| "无法收集扫描结果".to_string())?
            .into_inner()
            .unwrap();

        let total = *Arc::try_unwrap(total_size)
            .map_err(|_| "无法收集大小".to_string())?
            .get_mut()
            .unwrap();

        Ok((files, total))
    }

    // 递归收集目录 + 扫描当前目录的文件
    fn collect_directories_and_scan(
        &self,
        path: &Path,
        files_result: &Arc<Mutex<Vec<FileInfo>>>,
        total_size: &Arc<Mutex<u64>>,
    ) -> Result<Vec<PathBuf>, String> {
        let mut directories = Vec::new();
        let mut local_files = Vec::new();
        let mut local_total: u64 = 0;

        // 用一次 read_dir 读取所有条目，避免重复调用
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(e) => {
                let err_msg = e.to_string();
                if !err_msg.contains("拒绝访问") && !err_msg.contains("Access is denied") {
                    return Err(format!("无法读取目录 {}: {}", path.display(), e));
                }
                return Ok(Vec::new());
            }
        };

        for entry in entries.filter_map(|e| e.ok()) {
            // 从 entry 直接获取 file_type，避免路径查询
            let file_type = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };

            let file_path = entry.path();
            let path_string = file_path.to_string_lossy().to_string();

            // 检查是否受保护路径
            if is_protected_path(&path_string) {
                continue;
            }

            if file_type.is_dir() {
                // 收集子目录，递归处理（避免太深的递归用栈）
                directories.push(file_path);
            } else if file_type.is_file() {
                // 从 entry 获取 metadata，只需要一次系统调用
                if let Ok(metadata) = entry.metadata() {
                    let size = metadata.len();
                    local_total += size;

                    let modified = metadata
                        .modified()
                        .map(format_system_time_fast)
                        .unwrap_or_else(|_| "Unknown".to_string());

                    local_files.push(FileInfo {
                        path: path_string,
                        size,
                        modified,
                    });
                }
            }
        }

        // 合并到全局结果
        if !local_files.is_empty() {
            if let Ok(mut guard) = files_result.lock() {
                guard.extend(local_files);
            }
        }
        if local_total > 0 {
            if let Ok(mut guard) = total_size.lock() {
                *guard += local_total;
            }
        }

        Ok(directories)
    }

    fn find_duplicates(&self, files: &[FileInfo]) -> Vec<Vec<FileInfo>> {
        if files.len() < 2 {
            return Vec::new();
        }

        println!("正在检测重复文件...");

        // 第一步：按大小分组（只对 > 0 的文件，避免空文件匹配）
        let mut size_groups: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, file) in files.iter().enumerate() {
            if file.size > 0 {
                size_groups.entry(file.size).or_default().push(i);
            }
        }

        // 找到有重复大小的候选
        let size_candidates: Vec<(u64, Vec<usize>)> = size_groups
            .into_iter()
            .filter(|(_, indices)| indices.len() > 1)
            .collect();

        if size_candidates.is_empty() {
            println!("未发现重复文件");
            return Vec::new();
        }

        let total_candidates: usize = size_candidates.iter().map(|(_, v)| v.len()).sum();
        println!("发现 {} 个候选重复文件（{} 组大小）", total_candidates, size_candidates.len());

        // ====== 第二步：快速部分哈希（前 64KB），快速排除大部分非重复文件 ======
        println!("第一步：快速部分哈希（前64KB）...");

        // 为每个文件收集任务
        let mut partial_hash_tasks: Vec<(usize, usize, String)> = Vec::with_capacity(total_candidates);
        for (group_idx, (_, indices)) in size_candidates.iter().enumerate() {
            for &file_idx in indices {
                partial_hash_tasks.push((group_idx, file_idx, files[file_idx].path.clone()));
            }
        }

        let partial_hashes = self.parallel_compute_partial_hashes(&partial_hash_tasks);

        // 按 (group_idx, partial_hash) 分组 - 只在同一大小组内比较
        let mut partial_hash_groups: HashMap<(usize, String), Vec<usize>> = HashMap::new();
        for (task_info, hash_opt) in partial_hash_tasks.iter().zip(partial_hashes.iter()) {
            if let Some(ph) = hash_opt {
                let group_idx = task_info.0;
                let file_idx = task_info.1;
                let key = (group_idx, ph.clone());
                partial_hash_groups.entry(key).or_default().push(file_idx);
            }
        }

        // 第三步：只对部分哈希在同一组中有多个文件的，才需要计算完整 MD5
        let mut need_full_hash: Vec<(usize, String)> = Vec::new();
        for (_, group) in &partial_hash_groups {
            if group.len() > 1 {
                for &file_idx in group {
                    need_full_hash.push((file_idx, files[file_idx].path.clone()));
                }
            }
        }

        println!("部分哈希后，需要计算完整 MD5 的文件: {}", need_full_hash.len());

        if need_full_hash.is_empty() {
            println!("未发现重复文件");
            return Vec::new();
        }

        // ====== 第四步：并行计算完整 MD5 ======
        println!("第二步：正在计算 {} 个文件的完整 MD5...", need_full_hash.len());

        let full_hashes = self.parallel_compute_full_hashes(&need_full_hash);

        // 按 (size, full_hash) 分组
        let mut hash_groups: HashMap<(u64, String), Vec<FileInfo>> = HashMap::new();
        for (task_info, hash_opt) in need_full_hash.iter().zip(full_hashes.iter()) {
            if let Some(hash) = hash_opt {
                let file_idx = task_info.0;
                let file = &files[file_idx];
                let key = (file.size, hash.clone());
                hash_groups.entry(key).or_default().push(FileInfo {
                    path: file.path.clone(),
                    size: file.size,
                    modified: file.modified.clone(),
                });
            }
        }

        let duplicate_groups: Vec<Vec<FileInfo>> = hash_groups
            .into_iter()
            .filter(|(_, group)| group.len() > 1)
            .map(|(_, group)| group)
            .collect();

        println!("发现 {} 组重复文件", duplicate_groups.len());
        duplicate_groups
    }

    fn parallel_compute_partial_hashes(
        &self,
        tasks: &[(usize, usize, String)],
    ) -> Vec<Option<String>> {
        let num_threads = thread::available_parallelism().map(|p| p.get()).unwrap_or(4).min(12);
        let chunk_size = (tasks.len() / num_threads).max(1);

        let (tx, rx) = mpsc::channel::<(usize, Vec<Option<String>>)>();

        let mut handles = Vec::with_capacity(num_threads);
        for (chunk_idx, chunk) in tasks.chunks(chunk_size).enumerate() {
            let tx = tx.clone();
            let chunk_paths: Vec<String> = chunk.iter().map(|(_, _, p)| p.clone()).collect();

            handles.push(thread::spawn(move || {
                let mut results = Vec::with_capacity(chunk_paths.len());
                for (i, path) in chunk_paths.iter().enumerate() {
                    results.push(Self::compute_partial_hash_internal(path));
                    // 每 500 个打印进度
                    if (i + 1) % 500 == 0 {
                        println!("  线程{}: 已处理 {}/{}", chunk_idx, i + 1, chunk_paths.len());
                    }
                }
                let _ = tx.send((chunk_idx, results));
            }));
        }
        drop(tx);

        // 按 chunk_idx 收集，保持顺序
        let mut collected: Vec<(usize, Vec<Option<String>>)> = Vec::new();
        while let Ok(item) = rx.recv() {
            collected.push(item);
        }
        collected.sort_by_key(|(idx, _)| *idx);

        let mut results: Vec<Option<String>> = Vec::with_capacity(tasks.len());
        for (_, chunk_results) in collected {
            results.extend(chunk_results);
        }

        results
    }

    fn parallel_compute_full_hashes(
        &self,
        tasks: &[(usize, String)],
    ) -> Vec<Option<String>> {
        let num_threads = thread::available_parallelism().map(|p| p.get()).unwrap_or(4).min(12);
        let chunk_size = (tasks.len() / num_threads).max(1);

        let (tx, rx) = mpsc::channel::<(usize, Vec<Option<String>>)>();

        let mut handles = Vec::with_capacity(num_threads);
        for (chunk_idx, chunk) in tasks.chunks(chunk_size).enumerate() {
            let tx = tx.clone();
            let chunk_paths: Vec<String> = chunk.iter().map(|(_, p)| p.clone()).collect();

            handles.push(thread::spawn(move || {
                let mut results = Vec::with_capacity(chunk_paths.len());
                for (i, path) in chunk_paths.iter().enumerate() {
                    results.push(Self::compute_hash_internal(path));
                    if (i + 1) % 200 == 0 {
                        println!("  线程{}: 完整MD5 已处理 {}/{}", chunk_idx, i + 1, chunk_paths.len());
                    }
                }
                let _ = tx.send((chunk_idx, results));
            }));
        }
        drop(tx);

        let mut collected: Vec<(usize, Vec<Option<String>>)> = Vec::new();
        while let Ok(item) = rx.recv() {
            collected.push(item);
        }
        collected.sort_by_key(|(idx, _)| *idx);

        let mut results: Vec<Option<String>> = Vec::with_capacity(tasks.len());
        for (_, chunk_results) in collected {
            results.extend(chunk_results);
        }

        results
    }

    fn compute_partial_hash_internal(path: &str) -> Option<String> {
        let file = File::open(path).ok()?;
        let mut reader = BufReader::with_capacity(64 * 1024, file);
        let mut context = md5::Context::new();
        let mut buffer = [0u8; 64 * 1024];
        let mut bytes_read_total: usize = 0;
        const MAX_BYTES: usize = 64 * 1024; // 只读取前 64KB

        loop {
            let remaining = MAX_BYTES - bytes_read_total;
            if remaining == 0 {
                break;
            }
            let to_read = remaining.min(buffer.len());
            let bytes_read = reader.read(&mut buffer[..to_read]).ok()?;
            if bytes_read == 0 {
                break;
            }
            context.consume(&buffer[..bytes_read]);
            bytes_read_total += bytes_read;
        }

        Some(format!("{:x}", context.compute()))
    }

    fn compute_hash_internal(path: &str) -> Option<String> {
        let file = File::open(path).ok()?;
        let mut reader = BufReader::with_capacity(128 * 1024, file);
        let mut context = md5::Context::new();
        let mut buffer = [0u8; 128 * 1024];

        loop {
            let bytes_read = reader.read(&mut buffer).ok()?;
            if bytes_read == 0 {
                break;
            }
            context.consume(&buffer[..bytes_read]);
        }

        Some(format!("{:x}", context.compute()))
    }
}

impl Default for FileScanner {
    fn default() -> Self {
        Self::new()
    }
}

// ===== 工作线程 =====
fn worker_scan_thread(
    work_queue: Arc<Mutex<Vec<PathBuf>>>,
    files_result: Arc<Mutex<Vec<FileInfo>>>,
    total_size: Arc<Mutex<u64>>,
    _error_tx: Sender<String>,
) {
    loop {
        // 从队列取一个目录
        let dir_opt = {
            let mut queue = match work_queue.lock() {
                Ok(q) => q,
                Err(_) => return,
            };
            queue.pop()
        };

        let dir = match dir_opt {
            Some(d) => d,
            None => return, // 队列空了，退出
        };

        // 扫描这个目录
        scan_single_directory(&dir, &work_queue, &files_result, &total_size);
    }
}

fn scan_single_directory(
    path: &Path,
    work_queue: &Arc<Mutex<Vec<PathBuf>>>,
    files_result: &Arc<Mutex<Vec<FileInfo>>>,
    total_size: &Arc<Mutex<u64>>,
) {
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(e) => {
            let err_msg = e.to_string();
            if !err_msg.contains("拒绝访问") && !err_msg.contains("Access is denied") {
                // 静默处理权限错误
            }
            return;
        }
    };

    let mut local_files = Vec::with_capacity(64);
    let mut local_total: u64 = 0;
    let mut sub_dirs = Vec::with_capacity(16);

    for entry in entries.filter_map(|e| e.ok()) {
        // 从 entry 直接获取 file_type
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        let file_path = entry.path();
        let path_string = file_path.to_string_lossy().to_string();

        if is_protected_path(&path_string) {
            continue;
        }

        if file_type.is_dir() {
            sub_dirs.push(file_path);
        } else if file_type.is_file() {
            // 从 entry 获取 metadata，避免额外系统调用
            if let Ok(metadata) = entry.metadata() {
                let size = metadata.len();
                local_total += size;

                let modified = metadata
                    .modified()
                    .map(format_system_time_fast)
                    .unwrap_or_else(|_| "Unknown".to_string());

                local_files.push(FileInfo {
                    path: path_string,
                    size,
                    modified,
                });
            }
        }
    }

    // 将子目录加入工作队列
    if !sub_dirs.is_empty() {
        if let Ok(mut queue) = work_queue.lock() {
            queue.extend(sub_dirs);
        }
    }

    // 合并文件结果
    if !local_files.is_empty() {
        if let Ok(mut guard) = files_result.lock() {
            guard.extend(local_files);
        }
    }
    if local_total > 0 {
        if let Ok(mut guard) = total_size.lock() {
            *guard += local_total;
        }
    }
}

fn is_protected_path(path: &str) -> bool {
    let protected_patterns = [
        "$Recycle.Bin",
        "System Volume Information",
        "Documents and Settings",
        "PerfLogs",
        "Windows\\System32\\config",
        "Windows\\System32\\drivers",
        "Windows\\SysWOW64\\config",
        "Windows\\SysWOW64\\drivers",
        "Recovery",
        "Config.Msi",
        "ProgramData\\Microsoft\\Windows\\WER",
        "ProgramData\\Microsoft\\Windows\\Caches",
    ];

    let path_lower = path.to_lowercase();

    for pattern in &protected_patterns {
        if path_lower.contains(&pattern.to_lowercase()) {
            return true;
        }
    }

    false
}

fn format_system_time_fast(time: SystemTime) -> String {
    let duration = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    let secs = duration.as_secs();
    let days = secs / 86400;
    let year = (days / 365) + 1970;
    let remaining_days = days % 365;
    let month = (remaining_days / 30) + 1;
    let day = (remaining_days % 30) + 1;
    let hours = (secs % 86400) / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;

    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        year, month, day, hours, minutes, seconds)
}
