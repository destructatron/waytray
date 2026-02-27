//! Brightness module - displays and controls display backlight via logind D-Bus

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::RwLock;
use zbus::proxy;
use zbus::zvariant::OwnedObjectPath;
use zbus::Connection;

use crate::config::BrightnessModuleConfig;

use super::{ItemAction, Module, ModuleContext, ModuleItem};

const BACKLIGHT_BASE_PATH: &str = "/sys/class/backlight";

#[derive(Debug, Clone, PartialEq, Eq)]
struct BacklightDevice {
    name: String,
    max_brightness: u32,
    current_brightness_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrightnessState {
    device: BacklightDevice,
    raw_brightness: u32,
    percent: u32,
}

#[proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait LoginManager {
    fn get_session(&self, session_id: &str) -> zbus::Result<OwnedObjectPath>;
    fn get_session_by_pid(&self, pid: u32) -> zbus::Result<OwnedObjectPath>;
}

#[proxy(
    interface = "org.freedesktop.login1.Session",
    default_service = "org.freedesktop.login1"
)]
trait LoginSession {
    fn set_brightness(&self, subsystem: &str, name: &str, brightness: u32) -> zbus::Result<()>;
}

pub struct BrightnessModule {
    config: RwLock<BrightnessModuleConfig>,
    connection: RwLock<Option<Connection>>,
    ctx: RwLock<Option<Arc<ModuleContext>>>,
}

impl BrightnessModule {
    pub fn new(config: BrightnessModuleConfig) -> Self {
        Self {
            config: RwLock::new(config),
            connection: RwLock::new(None),
            ctx: RwLock::new(None),
        }
    }

    fn read_u32(path: &Path) -> Option<u32> {
        fs::read_to_string(path).ok()?.trim().parse().ok()
    }

    fn resolve_current_brightness_path(device_path: &Path) -> PathBuf {
        let actual_brightness = device_path.join("actual_brightness");
        if actual_brightness.is_file() {
            actual_brightness
        } else {
            device_path.join("brightness")
        }
    }

    fn build_backlight_device(device_path: &Path) -> Option<BacklightDevice> {
        let max_brightness = Self::read_u32(&device_path.join("max_brightness"))?;
        if max_brightness == 0 {
            return None;
        }

        let current_brightness_path = Self::resolve_current_brightness_path(device_path);
        if !current_brightness_path.is_file() {
            return None;
        }

        Some(BacklightDevice {
            name: device_path.file_name()?.to_string_lossy().to_string(),
            max_brightness,
            current_brightness_path,
        })
    }

    fn discover_backlight_device_in(
        base_path: &Path,
        configured_device: &str,
    ) -> Option<BacklightDevice> {
        if !configured_device.is_empty() {
            return Self::build_backlight_device(&base_path.join(configured_device));
        }

        let mut devices = Vec::new();
        let entries = fs::read_dir(base_path).ok()?;
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }

            if let Some(device) = Self::build_backlight_device(&entry.path()) {
                devices.push(device);
            }
        }

        devices.sort_by(|a, b| {
            b.max_brightness
                .cmp(&a.max_brightness)
                .then_with(|| a.name.cmp(&b.name))
        });

        devices.into_iter().next()
    }

    fn percent_from_raw(raw_brightness: u32, max_brightness: u32) -> u32 {
        if max_brightness == 0 {
            return 0;
        }

        ((raw_brightness as u64 * 100) / max_brightness as u64).min(100) as u32
    }

    fn raw_from_percent(percent: u32, max_brightness: u32) -> u32 {
        if max_brightness == 0 {
            return 0;
        }

        let clamped_percent = percent.min(100);
        ((clamped_percent as u64 * max_brightness as u64) / 100) as u32
    }

    async fn get_brightness_state(&self) -> Option<BrightnessState> {
        let configured_device = self.config.read().await.device.clone();
        let device =
            Self::discover_backlight_device_in(Path::new(BACKLIGHT_BASE_PATH), &configured_device)?;
        let raw_brightness = Self::read_u32(&device.current_brightness_path)?;
        let percent = Self::percent_from_raw(raw_brightness, device.max_brightness);

        Some(BrightnessState {
            device,
            raw_brightness,
            percent,
        })
    }

    fn icon_name(_percent: u32) -> &'static str {
        "display-brightness-symbolic"
    }

    async fn create_module_item(&self, state: &BrightnessState) -> ModuleItem {
        ModuleItem {
            id: "brightness:display".to_string(),
            module: "brightness".to_string(),
            label: format!("{}%", state.percent),
            icon_name: Some(Self::icon_name(state.percent).to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(format!(
                "Brightness: {}%\nDevice: {}",
                state.percent, state.device.name
            )),
            actions: vec![
                ItemAction::new("brightness_up", "Brightness Up"),
                ItemAction::new("brightness_down", "Brightness Down"),
            ],
        }
    }

    async fn send_update(&self) {
        let ctx = self.ctx.read().await.clone();
        let Some(ctx) = ctx else {
            return;
        };

        if let Some(state) = self.get_brightness_state().await {
            let item = self.create_module_item(&state).await;
            ctx.send_items("brightness", vec![item]);
        } else {
            ctx.send_items("brightness", vec![]);
        }
    }

    async fn set_brightness_percent(
        &self,
        device_name: &str,
        target_percent: u32,
        max_brightness: u32,
    ) {
        let connection = self.connection.read().await.clone();
        let Some(connection) = connection else {
            tracing::warn!("Brightness module has no system D-Bus connection");
            return;
        };

        let session_path = match self.session_path(&connection).await {
            Some(path) => path,
            None => return,
        };

        let builder = match LoginSessionProxy::builder(&connection).path(session_path) {
            Ok(builder) => builder,
            Err(e) => {
                tracing::warn!("Failed to configure logind session brightness proxy: {}", e);
                return;
            }
        };

        let proxy = match builder.build().await {
            Ok(proxy) => proxy,
            Err(e) => {
                tracing::warn!("Failed to create logind session brightness proxy: {}", e);
                return;
            }
        };

        let target_raw = Self::raw_from_percent(target_percent, max_brightness);
        if let Err(e) = proxy
            .set_brightness("backlight", device_name, target_raw)
            .await
        {
            tracing::warn!(
                "Failed to set brightness for {} via session object: {}",
                device_name,
                e
            );
        }
    }

    async fn session_path(&self, connection: &Connection) -> Option<OwnedObjectPath> {
        let proxy = match LoginManagerProxy::new(connection).await {
            Ok(proxy) => proxy,
            Err(e) => {
                tracing::warn!("Failed to create logind manager proxy: {}", e);
                return None;
            }
        };

        let mut session_lookup_error = None;

        if let Ok(session_id) = std::env::var("XDG_SESSION_ID") {
            let session_id = session_id.trim();
            if !session_id.is_empty() {
                match proxy.get_session(session_id).await {
                    Ok(path) => return Some(path),
                    Err(e) => {
                        session_lookup_error = Some(format!(
                            "XDG_SESSION_ID {} lookup failed: {}",
                            session_id, e
                        ));
                        tracing::debug!("{}", session_lookup_error.as_deref().unwrap());
                    }
                }
            } else {
                tracing::debug!("XDG_SESSION_ID was set but empty; falling back to PID lookup");
            }
        } else {
            tracing::debug!("XDG_SESSION_ID not set; falling back to PID lookup");
        }

        match proxy.get_session_by_pid(std::process::id()).await {
            Ok(path) => {
                if let Some(err) = session_lookup_error {
                    tracing::debug!(
                        "Resolved logind session via PID fallback after primary lookup failed: {}",
                        err
                    );
                }
                Some(path)
            }
            Err(e) => {
                if let Some(err) = session_lookup_error {
                    tracing::warn!(
                        "Failed to resolve logind session for brightness control ({}; PID {} lookup also failed: {})",
                        err,
                        std::process::id(),
                        e
                    );
                } else {
                    tracing::warn!(
                        "Failed to resolve logind session for brightness control via PID {}: {}",
                        std::process::id(),
                        e
                    );
                }
                None
            }
        }
    }

    async fn step_brightness(&self, delta_percent: i32) {
        let Some(state) = self.get_brightness_state().await else {
            return;
        };

        let new_percent = state.percent.saturating_add_signed(delta_percent).min(100);
        self.set_brightness_percent(&state.device.name, new_percent, state.device.max_brightness)
            .await;
    }
}

#[async_trait]
impl Module for BrightnessModule {
    fn name(&self) -> &str {
        "brightness"
    }

    fn enabled(&self) -> bool {
        self.config.try_read().map(|c| c.enabled).unwrap_or(true)
    }

    async fn start(&self, ctx: Arc<ModuleContext>) {
        if !self.config.read().await.enabled {
            return;
        }

        let connection = match Connection::system().await {
            Ok(connection) => connection,
            Err(e) => {
                tracing::error!(
                    "Failed to connect to system D-Bus for brightness module: {}",
                    e
                );
                return;
            }
        };

        *self.connection.write().await = Some(connection);
        *self.ctx.write().await = Some(ctx.clone());

        let mut last_state: Option<BrightnessState> = None;
        let mut sent_empty = false;

        loop {
            let current_state = self.get_brightness_state().await;

            match current_state.as_ref() {
                Some(state) if last_state.as_ref() != Some(state) => {
                    let item = self.create_module_item(state).await;
                    ctx.send_items("brightness", vec![item]);
                    last_state = Some(state.clone());
                    sent_empty = false;
                }
                None if !sent_empty || last_state.is_some() => {
                    ctx.send_items("brightness", vec![]);
                    last_state = None;
                    sent_empty = true;
                }
                _ => {}
            }

            let interval_seconds = self.config.read().await.interval_seconds.max(1);
            tokio::select! {
                _ = ctx.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(interval_seconds)) => {}
            }
        }
    }

    async fn stop(&self) {
        *self.ctx.write().await = None;
        *self.connection.write().await = None;
        tracing::info!("Brightness module stopped");
    }

    async fn invoke_action(&self, _item_id: &str, action_id: &str, _x: i32, _y: i32) {
        let step_percent = self.config.read().await.step_percent.max(1) as i32;

        let handled = match action_id {
            "brightness_up" => {
                self.step_brightness(step_percent).await;
                true
            }
            "brightness_down" => {
                self.step_brightness(-step_percent).await;
                true
            }
            _ => {
                tracing::warn!("Unknown brightness action: {}", action_id);
                false
            }
        };

        if handled {
            self.send_update().await;
        }
    }

    async fn reload_config(&self, config: &crate::config::Config) -> bool {
        if let Some(ref brightness_config) = config.modules.brightness {
            let mut current = self.config.write().await;
            *current = brightness_config.clone();
            tracing::debug!("Brightness module config reloaded");
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_backlight_dir() -> PathBuf {
        let unique = format!(
            "waytray-brightness-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_device(base: &Path, name: &str, brightness: u32, max_brightness: u32) {
        let path = base.join(name);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("brightness"), brightness.to_string()).unwrap();
        fs::write(path.join("max_brightness"), max_brightness.to_string()).unwrap();
    }

    #[test]
    fn percent_from_raw_uses_integer_ratio() {
        assert_eq!(BrightnessModule::percent_from_raw(0, 100), 0);
        assert_eq!(BrightnessModule::percent_from_raw(50, 100), 50);
        assert_eq!(BrightnessModule::percent_from_raw(100, 100), 100);
        assert_eq!(BrightnessModule::percent_from_raw(937, 937), 100);
    }

    #[test]
    fn raw_from_percent_is_clamped() {
        assert_eq!(BrightnessModule::raw_from_percent(0, 937), 0);
        assert_eq!(BrightnessModule::raw_from_percent(50, 937), 468);
        assert_eq!(BrightnessModule::raw_from_percent(100, 937), 937);
        assert_eq!(BrightnessModule::raw_from_percent(150, 937), 937);
    }

    #[test]
    fn configured_device_takes_precedence() {
        let base = temp_backlight_dir();
        write_device(&base, "intel_backlight", 500, 1000);
        write_device(&base, "acpi_video0", 50, 100);

        let selected =
            BrightnessModule::discover_backlight_device_in(&base, "acpi_video0").unwrap();
        assert_eq!(selected.name, "acpi_video0");

        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn auto_selects_device_with_highest_max_brightness() {
        let base = temp_backlight_dir();
        write_device(&base, "acpi_video0", 50, 100);
        write_device(&base, "intel_backlight", 500, 1200);
        write_device(&base, "amdgpu_bl1", 100, 400);

        let selected = BrightnessModule::discover_backlight_device_in(&base, "").unwrap();
        assert_eq!(selected.name, "intel_backlight");

        fs::remove_dir_all(base).unwrap();
    }
}
