//! Clock module - displays current time

use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use chrono::{Local, Timelike};

use crate::config::ClockModuleConfig;
use super::{Module, ModuleContext, ModuleItem};

/// Clock module that displays the current time
pub struct ClockModule {
    config: ClockModuleConfig,
}

impl ClockModule {
    pub fn new(config: ClockModuleConfig) -> Self {
        Self { config }
    }

    fn create_module_item(&self) -> ModuleItem {
        let now = Local::now();
        let time_str = now.format(&self.config.format).to_string();
        let date_str = now.format(&self.config.date_format).to_string();

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
        self.config.enabled
    }

    async fn start(&self, ctx: Arc<ModuleContext>) {
        if !self.enabled() {
            return;
        }

        // Send initial time
        let item = self.create_module_item();
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

            let item = self.create_module_item();
            ctx.send_items("clock", vec![item]);
        }
    }

    async fn stop(&self) {
        tracing::info!("Clock module stopped");
    }

    async fn invoke_action(&self, _item_id: &str, _action_id: &str, _x: i32, _y: i32) {
        // Clock module has no actions
    }
}
