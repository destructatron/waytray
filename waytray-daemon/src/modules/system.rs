//! System module - displays CPU and memory usage

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::config::SystemModuleConfig;
use super::{Module, ModuleContext, ModuleItem};

/// CPU usage tracking state
struct CpuState {
    prev_idle: u64,
    prev_total: u64,
}

/// Per-process CPU tracking state
struct ProcessCpuState {
    /// Map of PID to (utime + stime) from previous reading
    prev_times: HashMap<u32, u64>,
    /// Total CPU time from previous reading
    prev_total: u64,
}

/// Information about a process
struct ProcessInfo {
    name: String,
    usage: f64, // percentage for CPU, MB for memory
}

/// System module that displays CPU and memory usage
pub struct SystemModule {
    config: RwLock<SystemModuleConfig>,
    cpu_state: RwLock<Option<CpuState>>,
    process_cpu_state: RwLock<Option<ProcessCpuState>>,
}

impl SystemModule {
    pub fn new(config: SystemModuleConfig) -> Self {
        Self {
            config: RwLock::new(config),
            cpu_state: RwLock::new(None),
            process_cpu_state: RwLock::new(None),
        }
    }

    /// Read CPU usage from /proc/stat
    /// Returns percentage (0-100)
    async fn get_cpu_usage(&self) -> Option<u8> {
        let content = tokio::fs::read_to_string("/proc/stat").await.ok()?;
        let first_line = content.lines().next()?;

        // Format: cpu user nice system idle iowait irq softirq steal guest guest_nice
        if !first_line.starts_with("cpu ") {
            return None;
        }

        let values: Vec<u64> = first_line
            .split_whitespace()
            .skip(1) // Skip "cpu"
            .filter_map(|s| s.parse().ok())
            .collect();

        if values.len() < 4 {
            return None;
        }

        let idle = values[3] + values.get(4).unwrap_or(&0); // idle + iowait
        let total: u64 = values.iter().sum();

        let mut state = self.cpu_state.write().await;

        let usage = if let Some(prev) = state.as_ref() {
            let idle_delta = idle.saturating_sub(prev.prev_idle);
            let total_delta = total.saturating_sub(prev.prev_total);

            if total_delta > 0 {
                let usage = 100.0 * (1.0 - (idle_delta as f64 / total_delta as f64));
                usage.round() as u8
            } else {
                0
            }
        } else {
            0 // First reading, no delta yet
        };

        *state = Some(CpuState {
            prev_idle: idle,
            prev_total: total,
        });

        Some(usage)
    }

    /// Read memory usage from /proc/meminfo
    /// Returns (used_percent, used_gb, total_gb)
    async fn get_memory_usage(&self) -> Option<(u8, f64, f64)> {
        let content = tokio::fs::read_to_string("/proc/meminfo").await.ok()?;

        let mut mem_total: Option<u64> = None;
        let mut mem_available: Option<u64> = None;

        for line in content.lines() {
            if line.starts_with("MemTotal:") {
                mem_total = line.split_whitespace().nth(1)?.parse().ok();
            } else if line.starts_with("MemAvailable:") {
                mem_available = line.split_whitespace().nth(1)?.parse().ok();
            }

            if mem_total.is_some() && mem_available.is_some() {
                break;
            }
        }

        let total = mem_total?;
        let available = mem_available?;
        let used = total.saturating_sub(available);

        let percent = ((used as f64 / total as f64) * 100.0).round() as u8;
        let used_gb = used as f64 / 1_048_576.0; // kB to GB
        let total_gb = total as f64 / 1_048_576.0;

        Some((percent, used_gb, total_gb))
    }

    /// Read CPU temperature from thermal zones
    /// Returns temperature in Celsius
    async fn get_temperature(&self) -> Option<f32> {
        // Try thermal_zone0 first (most common for CPU)
        if let Ok(content) = tokio::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp").await {
            if let Ok(millidegrees) = content.trim().parse::<i32>() {
                return Some(millidegrees as f32 / 1000.0);
            }
        }

        // Fallback: try to find a thermal zone with "cpu" or "x86_pkg" in its type
        if let Ok(mut entries) = tokio::fs::read_dir("/sys/class/thermal").await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if let Some(name) = path.file_name() {
                    if name.to_string_lossy().starts_with("thermal_zone") {
                        // Check the type
                        let type_path = path.join("type");
                        if let Ok(zone_type) = tokio::fs::read_to_string(&type_path).await {
                            let zone_type = zone_type.trim().to_lowercase();
                            if zone_type.contains("cpu") || zone_type.contains("x86_pkg") || zone_type.contains("core") {
                                let temp_path = path.join("temp");
                                if let Ok(content) = tokio::fs::read_to_string(&temp_path).await {
                                    if let Ok(millidegrees) = content.trim().parse::<i32>() {
                                        return Some(millidegrees as f32 / 1000.0);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Get the process using the most CPU
    /// Returns (name, cpu_percentage)
    async fn get_top_cpu_process(&self) -> Option<ProcessInfo> {
        // Read total CPU time first
        let stat_content = tokio::fs::read_to_string("/proc/stat").await.ok()?;
        let first_line = stat_content.lines().next()?;
        if !first_line.starts_with("cpu ") {
            return None;
        }
        let total_values: Vec<u64> = first_line
            .split_whitespace()
            .skip(1)
            .filter_map(|s| s.parse().ok())
            .collect();
        let current_total: u64 = total_values.iter().sum();

        // Read all process CPU times
        let mut proc_entries = tokio::fs::read_dir("/proc").await.ok()?;
        let mut current_times: HashMap<u32, (String, u64)> = HashMap::new();

        while let Ok(Some(entry)) = proc_entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Only process numeric directories (PIDs)
            if let Ok(pid) = name_str.parse::<u32>() {
                let stat_path = entry.path().join("stat");
                if let Ok(content) = tokio::fs::read_to_string(&stat_path).await {
                    if let Some((proc_name, cpu_time)) = Self::parse_proc_stat(&content) {
                        current_times.insert(pid, (proc_name, cpu_time));
                    }
                }
            }
        }

        let mut state = self.process_cpu_state.write().await;

        let result = if let Some(prev) = state.as_ref() {
            let total_delta = current_total.saturating_sub(prev.prev_total);
            if total_delta == 0 {
                return None;
            }

            // Find the process with the highest CPU delta
            let mut top_process: Option<ProcessInfo> = None;
            let mut max_delta: u64 = 0;

            for (pid, (name, current_time)) in &current_times {
                if let Some(&prev_time) = prev.prev_times.get(pid) {
                    let delta = current_time.saturating_sub(prev_time);
                    if delta > max_delta {
                        max_delta = delta;
                        let cpu_percent = (delta as f64 / total_delta as f64) * 100.0;
                        top_process = Some(ProcessInfo {
                            name: name.clone(),
                            usage: cpu_percent,
                        });
                    }
                }
            }

            top_process
        } else {
            None // First reading, no delta yet
        };

        // Update state for next reading
        *state = Some(ProcessCpuState {
            prev_times: current_times.into_iter().map(|(pid, (_, time))| (pid, time)).collect(),
            prev_total: current_total,
        });

        result
    }

    /// Parse /proc/[pid]/stat to get process name and CPU time
    fn parse_proc_stat(content: &str) -> Option<(String, u64)> {
        // Format: pid (comm) state ppid ... utime stime ...
        // Fields 14 and 15 (1-indexed) are utime and stime
        // comm can contain spaces and parentheses, so we need to find the last ')'
        let start = content.find('(')?;
        let end = content.rfind(')')?;

        let name = content[start + 1..end].to_string();
        let rest = &content[end + 2..]; // Skip ") "

        let fields: Vec<&str> = rest.split_whitespace().collect();
        // After comm, fields are: state(0), ppid(1), ..., utime(11), stime(12)
        if fields.len() < 13 {
            return None;
        }

        let utime: u64 = fields[11].parse().ok()?;
        let stime: u64 = fields[12].parse().ok()?;

        Some((name, utime + stime))
    }

    /// Get the process using the most memory
    /// Returns (name, memory_mb)
    async fn get_top_memory_process(&self) -> Option<ProcessInfo> {
        let mut proc_entries = tokio::fs::read_dir("/proc").await.ok()?;
        let mut top_process: Option<ProcessInfo> = None;
        let mut max_rss: u64 = 0;

        while let Ok(Some(entry)) = proc_entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Only process numeric directories (PIDs)
            if name_str.parse::<u32>().is_ok() {
                let status_path = entry.path().join("status");
                if let Ok(content) = tokio::fs::read_to_string(&status_path).await {
                    if let Some((proc_name, rss_kb)) = Self::parse_proc_status(&content) {
                        if rss_kb > max_rss {
                            max_rss = rss_kb;
                            top_process = Some(ProcessInfo {
                                name: proc_name,
                                usage: rss_kb as f64 / 1024.0, // Convert to MB
                            });
                        }
                    }
                }
            }
        }

        top_process
    }

    /// Parse /proc/[pid]/status to get process name and RSS
    fn parse_proc_status(content: &str) -> Option<(String, u64)> {
        let mut name: Option<String> = None;
        let mut rss: Option<u64> = None;

        for line in content.lines() {
            if line.starts_with("Name:") {
                name = line.split_whitespace().nth(1).map(String::from);
            } else if line.starts_with("VmRSS:") {
                // Format: VmRSS: 12345 kB
                rss = line.split_whitespace().nth(1).and_then(|s| s.parse().ok());
            }

            if name.is_some() && rss.is_some() {
                break;
            }
        }

        Some((name?, rss?))
    }

    fn create_cpu_item(&self, usage: u8, top_process: Option<ProcessInfo>) -> ModuleItem {
        // Use generic system monitor icon (cpu-specific icons often not available)
        let icon_name = "utilities-system-monitor";

        let tooltip = match top_process {
            Some(proc) => format!("CPU Usage: {}%\nTop: {} ({:.1}%)", usage, proc.name, proc.usage),
            None => format!("CPU Usage: {}%", usage),
        };

        ModuleItem {
            id: "system:cpu".to_string(),
            module: "system".to_string(),
            label: format!("CPU {}%", usage),
            icon_name: Some(icon_name.to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(tooltip),
            actions: Vec::new(),
        }
    }

    fn create_memory_item(&self, percent: u8, used_gb: f64, total_gb: f64, top_process: Option<ProcessInfo>) -> ModuleItem {
        let tooltip = match top_process {
            Some(proc) => format!(
                "Memory: {:.1} GB / {:.1} GB ({}%)\nTop: {} ({:.0} MB)",
                used_gb, total_gb, percent, proc.name, proc.usage
            ),
            None => format!(
                "Memory: {:.1} GB / {:.1} GB ({}%)",
                used_gb, total_gb, percent
            ),
        };

        ModuleItem {
            id: "system:memory".to_string(),
            module: "system".to_string(),
            label: format!("Mem {}%", percent),
            icon_name: Some("drive-harddisk".to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(tooltip),
            actions: Vec::new(),
        }
    }

    fn create_temperature_item(&self, temp: f32) -> ModuleItem {
        // Choose icon based on temperature
        let icon_name = if temp >= 80.0 {
            "dialog-warning" // Hot!
        } else {
            "sensors-temperature"
        };

        ModuleItem {
            id: "system:temperature".to_string(),
            module: "system".to_string(),
            label: format!("{:.0}°C", temp),
            icon_name: Some(icon_name.to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(format!("CPU Temperature: {:.1}°C", temp)),
            actions: Vec::new(),
        }
    }

    async fn create_items(&self) -> Vec<ModuleItem> {
        let config = self.config.read().await;
        let mut items = Vec::new();

        // CPU usage and temperature together
        if config.show_cpu {
            if let Some(usage) = self.get_cpu_usage().await {
                let top_process = if config.show_top_cpu_process {
                    self.get_top_cpu_process().await
                } else {
                    None
                };
                items.push(self.create_cpu_item(usage, top_process));
            }
        }

        if config.show_temperature {
            if let Some(temp) = self.get_temperature().await {
                items.push(self.create_temperature_item(temp));
            }
        }

        // Memory last
        if config.show_memory {
            if let Some((percent, used_gb, total_gb)) = self.get_memory_usage().await {
                let top_process = if config.show_top_memory_process {
                    self.get_top_memory_process().await
                } else {
                    None
                };
                items.push(self.create_memory_item(percent, used_gb, total_gb, top_process));
            }
        }

        items
    }
}

#[async_trait]
impl Module for SystemModule {
    fn name(&self) -> &str {
        "system"
    }

    fn enabled(&self) -> bool {
        self.config.try_read().map(|c| c.enabled).unwrap_or(true)
    }

    async fn start(&self, ctx: Arc<ModuleContext>) {
        if !self.config.read().await.enabled {
            return;
        }

        // Initial read to populate CPU state
        let _ = self.get_cpu_usage().await;

        // Wait a moment for first delta (cancellable)
        tokio::select! {
            _ = ctx.cancelled() => return,
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }

        // Send initial items
        let items = self.create_items().await;
        ctx.send_items("system", items);

        // Poll at configured interval (re-read each iteration for hot reload)
        loop {
            let interval = {
                let config = self.config.read().await;
                Duration::from_secs(config.interval_seconds)
            };

            tokio::select! {
                _ = ctx.cancelled() => break,
                _ = tokio::time::sleep(interval) => {
                    let items = self.create_items().await;
                    ctx.send_items("system", items);
                }
            }
        }
    }

    async fn stop(&self) {
        tracing::info!("System module stopped");
    }

    async fn invoke_action(&self, _item_id: &str, _action_id: &str, _x: i32, _y: i32) {
        // System module has no actions
    }

    async fn reload_config(&self, config: &crate::config::Config) -> bool {
        if let Some(ref system_config) = config.modules.system {
            let mut current = self.config.write().await;
            *current = system_config.clone();
            tracing::debug!("System module config reloaded");
            true
        } else {
            false
        }
    }
}
