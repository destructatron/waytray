//! GPU module - displays GPU usage, temperature, and top process

use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::config::GpuModuleConfig;
use super::{Module, ModuleContext, ModuleItem};

/// Information about a GPU process
struct GpuProcessInfo {
    name: String,
    memory_mb: u64,
}

/// Detected GPU type
#[derive(Debug, Clone, Copy, PartialEq)]
enum GpuType {
    Nvidia,
    Amd,
    Intel,
    Unknown,
}

/// GPU module that displays GPU usage and temperature
pub struct GpuModule {
    config: RwLock<GpuModuleConfig>,
    gpu_type: RwLock<Option<GpuType>>,
    /// AMD card path (e.g., /sys/class/drm/card0/device)
    amd_device_path: RwLock<Option<String>>,
}

impl GpuModule {
    pub fn new(config: GpuModuleConfig) -> Self {
        Self {
            config: RwLock::new(config),
            gpu_type: RwLock::new(None),
            amd_device_path: RwLock::new(None),
        }
    }

    /// Detect what type of GPU is available
    async fn detect_gpu_type(&self) -> GpuType {
        // Check for cached result
        if let Some(gpu_type) = *self.gpu_type.read().await {
            return gpu_type;
        }

        // Try NVIDIA first (nvidia-smi)
        if let Ok(output) = tokio::process::Command::new("nvidia-smi")
            .arg("--query-gpu=name")
            .arg("--format=csv,noheader")
            .output()
            .await
        {
            if output.status.success() {
                let mut gpu_type = self.gpu_type.write().await;
                *gpu_type = Some(GpuType::Nvidia);
                tracing::info!("Detected NVIDIA GPU");
                return GpuType::Nvidia;
            }
        }

        // Try AMD via sysfs
        if let Some(device_path) = Self::find_amd_gpu().await {
            let mut amd_path = self.amd_device_path.write().await;
            *amd_path = Some(device_path);
            let mut gpu_type = self.gpu_type.write().await;
            *gpu_type = Some(GpuType::Amd);
            tracing::info!("Detected AMD GPU");
            return GpuType::Amd;
        }

        // Try Intel via sysfs
        if Self::find_intel_gpu().await.is_some() {
            let mut gpu_type = self.gpu_type.write().await;
            *gpu_type = Some(GpuType::Intel);
            tracing::info!("Detected Intel GPU");
            return GpuType::Intel;
        }

        let mut gpu_type = self.gpu_type.write().await;
        *gpu_type = Some(GpuType::Unknown);
        tracing::warn!("No supported GPU detected");
        GpuType::Unknown
    }

    /// Find AMD GPU device path in sysfs
    async fn find_amd_gpu() -> Option<String> {
        let mut entries = tokio::fs::read_dir("/sys/class/drm").await.ok()?;

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Look for card directories (card0, card1, etc.)
            if name_str.starts_with("card") && !name_str.contains('-') {
                let device_path = entry.path().join("device");

                // Check if this is an AMD GPU by looking for amdgpu driver
                let driver_path = device_path.join("driver");
                if let Ok(driver_link) = tokio::fs::read_link(&driver_path).await {
                    if driver_link.to_string_lossy().contains("amdgpu") {
                        // Verify gpu_busy_percent exists
                        let busy_path = device_path.join("gpu_busy_percent");
                        if tokio::fs::metadata(&busy_path).await.is_ok() {
                            return Some(device_path.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }

        None
    }

    /// Find Intel GPU device path in sysfs
    async fn find_intel_gpu() -> Option<String> {
        let mut entries = tokio::fs::read_dir("/sys/class/drm").await.ok()?;

        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            if name_str.starts_with("card") && !name_str.contains('-') {
                let device_path = entry.path().join("device");
                let driver_path = device_path.join("driver");

                if let Ok(driver_link) = tokio::fs::read_link(&driver_path).await {
                    if driver_link.to_string_lossy().contains("i915") {
                        return Some(device_path.to_string_lossy().to_string());
                    }
                }
            }
        }

        None
    }

    /// Get GPU usage from NVIDIA
    async fn get_nvidia_usage(&self) -> Option<u8> {
        let output = tokio::process::Command::new("nvidia-smi")
            .arg("--query-gpu=utilization.gpu")
            .arg("--format=csv,noheader,nounits")
            .output()
            .await
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.trim().parse().ok()
    }

    /// Get GPU temperature from NVIDIA
    async fn get_nvidia_temperature(&self) -> Option<f32> {
        let output = tokio::process::Command::new("nvidia-smi")
            .arg("--query-gpu=temperature.gpu")
            .arg("--format=csv,noheader,nounits")
            .output()
            .await
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.trim().parse().ok()
    }

    /// Get top GPU process from NVIDIA
    async fn get_nvidia_top_process(&self) -> Option<GpuProcessInfo> {
        let output = tokio::process::Command::new("nvidia-smi")
            .arg("--query-compute-apps=pid,used_memory")
            .arg("--format=csv,noheader,nounits")
            .output()
            .await
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut top_process: Option<GpuProcessInfo> = None;
        let mut max_memory: u64 = 0;

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
            if parts.len() >= 2 {
                if let (Ok(pid), Ok(memory)) = (parts[0].parse::<u32>(), parts[1].parse::<u64>()) {
                    if memory > max_memory {
                        max_memory = memory;
                        // Get process name from /proc
                        if let Some(name) = Self::get_process_name(pid).await {
                            top_process = Some(GpuProcessInfo {
                                name,
                                memory_mb: memory,
                            });
                        }
                    }
                }
            }
        }

        top_process
    }

    /// Get process name from PID
    async fn get_process_name(pid: u32) -> Option<String> {
        let comm_path = format!("/proc/{}/comm", pid);
        tokio::fs::read_to_string(&comm_path)
            .await
            .ok()
            .map(|s| s.trim().to_string())
    }

    /// Get GPU usage from AMD via sysfs
    async fn get_amd_usage(&self) -> Option<u8> {
        let device_path = self.amd_device_path.read().await;
        let device_path = device_path.as_ref()?;

        let busy_path = format!("{}/gpu_busy_percent", device_path);
        let content = tokio::fs::read_to_string(&busy_path).await.ok()?;
        content.trim().parse().ok()
    }

    /// Get GPU temperature from AMD via sysfs (hwmon)
    async fn get_amd_temperature(&self) -> Option<f32> {
        let device_path = self.amd_device_path.read().await;
        let device_path = device_path.as_ref()?;

        let hwmon_path = format!("{}/hwmon", device_path);
        let mut entries = tokio::fs::read_dir(&hwmon_path).await.ok()?;

        while let Ok(Some(entry)) = entries.next_entry().await {
            let temp_path = entry.path().join("temp1_input");
            if let Ok(content) = tokio::fs::read_to_string(&temp_path).await {
                if let Ok(millidegrees) = content.trim().parse::<i32>() {
                    return Some(millidegrees as f32 / 1000.0);
                }
            }
        }

        None
    }

    /// Get GPU usage (dispatches to correct implementation)
    async fn get_usage(&self) -> Option<u8> {
        match self.detect_gpu_type().await {
            GpuType::Nvidia => self.get_nvidia_usage().await,
            GpuType::Amd => self.get_amd_usage().await,
            GpuType::Intel => None, // Intel GPU usage requires i915 perf counters, not easily accessible
            GpuType::Unknown => None,
        }
    }

    /// Get GPU temperature (dispatches to correct implementation)
    async fn get_temperature(&self) -> Option<f32> {
        match self.detect_gpu_type().await {
            GpuType::Nvidia => self.get_nvidia_temperature().await,
            GpuType::Amd => self.get_amd_temperature().await,
            GpuType::Intel => None,
            GpuType::Unknown => None,
        }
    }

    /// Get top GPU process (currently NVIDIA only)
    async fn get_top_process(&self) -> Option<GpuProcessInfo> {
        match self.detect_gpu_type().await {
            GpuType::Nvidia => self.get_nvidia_top_process().await,
            _ => None, // AMD/Intel don't have easy process tracking
        }
    }

    fn create_gpu_item(&self, usage: u8, temperature: Option<f32>, top_process: Option<GpuProcessInfo>) -> ModuleItem {
        let icon_name = if temperature.map(|t| t >= 80.0).unwrap_or(false) {
            "dialog-warning"
        } else {
            "video-display" // Generic display/GPU icon
        };

        let mut tooltip_parts = vec![format!("GPU Usage: {}%", usage)];

        if let Some(temp) = temperature {
            tooltip_parts.push(format!("Temperature: {:.0}°C", temp));
        }

        if let Some(proc) = top_process {
            tooltip_parts.push(format!("Top: {} ({} MB)", proc.name, proc.memory_mb));
        }

        let tooltip = tooltip_parts.join("\n");

        ModuleItem {
            id: "gpu:usage".to_string(),
            module: "gpu".to_string(),
            label: format!("GPU {}%", usage),
            icon_name: Some(icon_name.to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(tooltip),
            actions: Vec::new(),
        }
    }

    async fn create_items(&self) -> Vec<ModuleItem> {
        let config = self.config.read().await;
        let mut items = Vec::new();

        if let Some(usage) = self.get_usage().await {
            let temperature = if config.show_temperature {
                self.get_temperature().await
            } else {
                None
            };

            let top_process = if config.show_top_process {
                self.get_top_process().await
            } else {
                None
            };

            items.push(self.create_gpu_item(usage, temperature, top_process));
        }

        items
    }
}

#[async_trait]
impl Module for GpuModule {
    fn name(&self) -> &str {
        "gpu"
    }

    fn enabled(&self) -> bool {
        self.config.try_read().map(|c| c.enabled).unwrap_or(true)
    }

    async fn start(&self, ctx: Arc<ModuleContext>) {
        if !self.config.read().await.enabled {
            return;
        }

        // Detect GPU type on startup
        let gpu_type = self.detect_gpu_type().await;
        if gpu_type == GpuType::Unknown {
            tracing::warn!("No supported GPU found, GPU module will not display anything");
            return;
        }

        // Send initial items
        let items = self.create_items().await;
        ctx.send_items("gpu", items);

        // Poll at configured interval
        loop {
            let interval = {
                let config = self.config.read().await;
                Duration::from_secs(config.interval_seconds)
            };

            tokio::select! {
                _ = ctx.cancelled() => break,
                _ = tokio::time::sleep(interval) => {
                    let items = self.create_items().await;
                    ctx.send_items("gpu", items);
                }
            }
        }
    }

    async fn stop(&self) {
        tracing::info!("GPU module stopped");
    }

    async fn invoke_action(&self, _item_id: &str, _action_id: &str, _x: i32, _y: i32) {
        // GPU module has no actions
    }

    async fn reload_config(&self, config: &crate::config::Config) -> bool {
        if let Some(ref gpu_config) = config.modules.gpu {
            let mut current = self.config.write().await;
            *current = gpu_config.clone();
            tracing::debug!("GPU module config reloaded");
            true
        } else {
            false
        }
    }
}
