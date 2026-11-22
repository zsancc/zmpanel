use chrono::Utc;
use std::time::Duration;
use sysinfo::{Disks, Networks, Pid, ProcessesToUpdate, System};
use zmpanel_core::{AppError, DiskInfo, MetricPoint, ProcessItem, SystemSnapshot};

use crate::runtime_env;
use crate::state::AppState;

pub fn spawn_monitor_task(state: AppState) {
    tokio::spawn(async move {
        // Probe host virtualization / container nesting capability once at startup (zero loop overhead).
        let runtime_env = runtime_env::detect_runtime_env(false);
        // Use System::new() instead of new_all() to avoid caching the full OS process table
        // permanently in RAM (critical for 256MB NAT VPS footprint).
        let mut sys = System::new();
        let mut disks = Disks::new_with_refreshed_list();
        let mut networks = Networks::new_with_refreshed_list();
        let self_pid = Pid::from_u32(std::process::id());

        let mut last_persist_ts: i64 = 0;
        let mut prev_rx_total: u64 = total_rx(&networks);
        let mut prev_tx_total: u64 = total_tx(&networks);
        let mut last_tick = std::time::Instant::now();

        // Prime CPU readings
        sys.refresh_cpu_usage();
        tokio::time::sleep(Duration::from_millis(250)).await;

        loop {
            sys.refresh_cpu_usage();
            sys.refresh_memory();
            sys.refresh_processes(ProcessesToUpdate::Some(&[self_pid]), true);
            disks.refresh(true);
            networks.refresh(true);

            let elapsed = last_tick.elapsed().as_secs_f64().max(0.2);
            last_tick = std::time::Instant::now();

            let cur_rx = total_rx(&networks);
            let cur_tx = total_tx(&networks);
            let rx_delta = cur_rx.saturating_sub(prev_rx_total);
            let tx_delta = cur_tx.saturating_sub(prev_tx_total);
            prev_rx_total = cur_rx;
            prev_tx_total = cur_tx;

            let rx_rate = ((rx_delta as f64) / elapsed) as u64;
            let tx_rate = ((tx_delta as f64) / elapsed) as u64;

            let now = Utc::now();
            let now_ts = now.timestamp();
            let month_key = now.format("%Y-%m").to_string();

            let (rx_month, tx_month) = if rx_delta > 0 || tx_delta > 0 {
                state
                    .db
                    .add_monthly_traffic(&month_key, rx_delta, tx_delta)
                    .unwrap_or((0, 0))
            } else {
                state.db.get_monthly_traffic(&month_key).unwrap_or((0, 0))
            };

            let load = System::load_average();
            let cpu_usage = sys.global_cpu_usage();
            let mem_total = sys.total_memory();
            let mem_used = sys.used_memory();
            let mem_available = sys.available_memory();
            let swap_total = sys.total_swap();
            let swap_used = sys.used_swap();

            let panel_rss_bytes = sys
                .process(self_pid)
                .map(|p| p.memory())
                .unwrap_or(0);

            let disk_list: Vec<DiskInfo> = disks
                .list()
                .iter()
                .filter(|d| d.total_space() > 0)
                .map(|d| {
                    let total = d.total_space();
                    let avail = d.available_space();
                    DiskInfo {
                        name: d.name().to_string_lossy().to_string(),
                        mount_point: d.mount_point().to_string_lossy().to_string(),
                        fs_type: d.file_system().to_string_lossy().to_string(),
                        total_bytes: total,
                        available_bytes: avail,
                        used_bytes: total.saturating_sub(avail),
                    }
                })
                .collect();

            let cpu_brand = sys
                .cpus()
                .first()
                .map(|c| c.brand().trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Unknown CPU".to_string());

            let snapshot = SystemSnapshot {
                timestamp: now_ts,
                hostname: System::host_name().unwrap_or_else(|| "localhost".to_string()),
                os_name: System::name().unwrap_or_else(|| std::env::consts::OS.to_string()),
                os_version: System::os_version().unwrap_or_else(|| "-".to_string()),
                kernel_version: System::kernel_version().unwrap_or_else(|| "-".to_string()),
                arch: std::env::consts::ARCH.to_string(),
                uptime_secs: System::uptime(),
                load_avg: [load.one, load.five, load.fifteen],
                cpu_brand,
                cpu_cores: sys.cpus().len(),
                cpu_usage,
                mem_total,
                mem_used,
                mem_available,
                swap_total,
                swap_used,
                net_rx_rate: rx_rate,
                net_tx_rate: tx_rate,
                net_rx_month: rx_month,
                net_tx_month: tx_month,
                disks: disk_list,
                panel_rss_bytes,
                runtime_env: runtime_env.clone(),
            };

            if let Ok(mut guard) = state.latest_snapshot.write() {
                *guard = Some(snapshot);
            }

            if now_ts - last_persist_ts >= 30 {
                last_persist_ts = now_ts;
                let mem_pct = if mem_total > 0 {
                    (mem_used as f64 / mem_total as f64 * 100.0) as f32
                } else {
                    0.0
                };
                let swap_pct = if swap_total > 0 {
                    (swap_used as f64 / swap_total as f64 * 100.0) as f32
                } else {
                    0.0
                };
                let point = MetricPoint {
                    timestamp: now_ts,
                    cpu_usage,
                    mem_used_pct: mem_pct,
                    swap_used_pct: swap_pct,
                    load_1m: load.one,
                    net_rx_rate: rx_rate,
                    net_tx_rate: tx_rate,
                };
                let _ = state.db.insert_metric(&point);
            }

            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

fn total_rx(networks: &Networks) -> u64 {
    networks
        .iter()
        .filter(|(name, _)| !is_loopback_iface(name))
        .map(|(_, data)| data.total_received())
        .sum()
}

fn total_tx(networks: &Networks) -> u64 {
    networks
        .iter()
        .filter(|(name, _)| !is_loopback_iface(name))
        .map(|(_, data)| data.total_transmitted())
        .sum()
}

fn is_loopback_iface(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower == "lo" || lower.starts_with("loopback")
}

pub fn list_processes(sort_by: &str, keyword: Option<&str>, limit: usize) -> Vec<ProcessItem> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let kw = keyword.map(|k| k.to_lowercase());

    let mut items: Vec<ProcessItem> = sys
        .processes()
        .iter()
        .map(|(pid, proc_)| {
            let cmd = proc_
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(" ");
            ProcessItem {
                pid: pid.as_u32(),
                name: proc_.name().to_string_lossy().to_string(),
                cpu_usage: proc_.cpu_usage(),
                memory_bytes: proc_.memory(),
                status: format!("{:?}", proc_.status()),
                cmd,
            }
        })
        .filter(|item| {
            if let Some(ref k) = kw
                && !k.is_empty()
            {
                return item.name.to_lowercase().contains(k)
                    || item.cmd.to_lowercase().contains(k)
                    || item.pid.to_string().contains(k);
            }
            true
        })
        .collect();

    match sort_by {
        "cpu" => items.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "pid" => items.sort_by_key(|p| p.pid),
        _ => items.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes)),
    }

    items.truncate(limit.clamp(10, 500));
    items
}

pub fn kill_process(pid: u32) -> Result<(), AppError> {
    if pid == std::process::id() {
        return Err(AppError::Forbidden("禁止终止面板自身进程".to_string()));
    }
    let mut sys = System::new();
    let target = Pid::from_u32(pid);
    sys.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
    let Some(process) = sys.process(target) else {
        return Err(AppError::NotFound(format!("未找到进程 PID {}", pid)));
    };
    if process.kill() {
        Ok(())
    } else {
        Err(AppError::Forbidden(format!(
            "无法终止进程 PID {}，可能权限不足",
            pid
        )))
    }
}

#[cfg(target_os = "linux")]
pub fn configure_swap_file(size_mb: u64) -> Result<String, AppError> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;

    let swap_path = "/swapfile.zmpanel";
    // Disable existing zmpanel swapfile if active
    let _ = Command::new("swapoff").arg(swap_path).status();

    if size_mb == 0 {
        let _ = std::fs::remove_file(swap_path);
        return Ok("已关闭并清理 zmpanel Swap 文件".to_string());
    }

    if size_mb > 16384 {
        return Err(AppError::BadRequest("单次 Swap 大小上限为 16384 MB".to_string()));
    }

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(swap_path)
        .map_err(|e| AppError::Forbidden(format!("创建 Swap 文件失败（需 root 权限）: {}", e)))?;
    file.set_len(size_mb * 1024 * 1024)
        .map_err(|e| AppError::Internal(format!("分配 Swap 空间失败: {}", e)))?;
    std::fs::set_permissions(swap_path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| AppError::Internal(format!("设置 Swap 文件权限失败: {}", e)))?;

    let mk = Command::new("mkswap")
        .arg(swap_path)
        .output()
        .map_err(|e| AppError::Internal(format!("执行 mkswap 失败: {}", e)))?;
    if !mk.status.success() {
        return Err(AppError::Internal(format!(
            "mkswap 返回失败: {}",
            String::from_utf8_lossy(&mk.stderr)
        )));
    }

    let on = Command::new("swapon")
        .arg(swap_path)
        .output()
        .map_err(|e| AppError::Internal(format!("执行 swapon 失败: {}", e)))?;
    if !on.status.success() {
        return Err(AppError::Internal(format!(
            "swapon 返回失败（部分 OpenVZ/LXC 容器宿主机限制开启 Swap）: {}",
            String::from_utf8_lossy(&on.stderr)
        )));
    }

    Ok(format!("已启用 {} MB Swap 文件 ({})", size_mb, swap_path))
}

#[cfg(not(target_os = "linux"))]
pub fn configure_swap_file(_size_mb: u64) -> Result<String, AppError> {
    Err(AppError::BadRequest(
        "Swap 文件管理仅支持 Linux 服务器环境（Debian / Ubuntu / Alpine）".to_string(),
    ))
}
