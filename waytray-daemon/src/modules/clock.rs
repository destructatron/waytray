//! Clock module - displays current time

use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use chrono::{Local, Timelike};
use tokio::sync::RwLock;

use crate::config::ClockModuleConfig;
use super::{Module, ModuleContext, ModuleItem};

/// Clock module that displays the current time
pub struct ClockModule {
    config: RwLock<ClockModuleConfig>,
}

impl ClockModule {
    pub fn new(config: ClockModuleConfig) -> Self {
        Self { config: RwLock::new(config) }
    }

    async fn create_module_item(&self) -> ModuleItem {
        let config = self.config.read().await;
        let now = Local::now();
        let time_str = now.format(&config.format).to_string();
        let date_str = now.format(&config.date_format).to_string();

        ModuleItem {
            id: "clock:time".to_string(),
            module: "clock".to_string(),
            label: time_str,
            icon_name: Some("preferences-system-time".to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(date_str),
            actions: Vec::new(),
        }
    }
}

#[async_trait]
impl Module for ClockModule {
    fn name(&self) -> &str {
        "clock"
    }

    fn enabled(&self) -> bool {
        self.config.try_read().map(|c| c.enabled).unwrap_or(true)
    }

    async fn start(&self, ctx: Arc<ModuleContext>) {
        if !self.config.read().await.enabled {
            return;
        }

        // Send initial time
        let item = self.create_module_item().await;
        ctx.send_items("clock", vec![item]);

        // Update every minute, synchronized to the minute boundary
        loop {
            // Calculate time until next minute
            let now = Local::now();
            let seconds_until_next_minute = 60 - now.second() as u64;
            let nanos = now.nanosecond();

            // Sleep until just after the next minute starts
            let sleep_duration = Duration::from_secs(seconds_until_next_minute)
                - Duration::from_nanos(nanos as u64)
                + Duration::from_millis(100); // Small buffer

            tokio::time::sleep(sleep_duration).await;

            let item = self.create_module_item().await;
            ctx.send_items("clock", vec![item]);
        }
    }

    async fn stop(&self) {
        tracing::info!("Clock module stopped");
    }

    async fn invoke_action(&self, _item_id: &str, _action_id: &str, _x: i32, _y: i32) {
        // Clock module has no actions
    }

    async fn reload_config(&self, config: &crate::config::Config) -> bool {
        if let Some(ref clock_config) = config.modules.clock {
            let mut current = self.config.write().await;
            *current = clock_config.clone();
            tracing::debug!("Clock module config reloaded");
            true
        } else {
            false
        }
    }
}
