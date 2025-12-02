//! Desktop notification service
//!
//! Sends desktop notifications using the freedesktop notification standard.

use crate::modules::Urgency;

/// Service for sending desktop notifications
pub struct NotificationService {
    enabled: bool,
    timeout_ms: u32,
}

impl NotificationService {
    /// Create a new notification service
    pub fn new(enabled: bool, timeout_ms: u32) -> Self {
        Self { enabled, timeout_ms }
    }

    /// Send a desktop notification
    pub fn send(&self, title: &str, body: &str, urgency: Urgency) {
        if !self.enabled {
            return;
        }

        let notify_urgency = match urgency {
            Urgency::Low => notify_rust::Urgency::Low,
            Urgency::Normal => notify_rust::Urgency::Normal,
            Urgency::Critical => notify_rust::Urgency::Critical,
        };

        let timeout = if self.timeout_ms == 0 {
            notify_rust::Timeout::Never
        } else {
            notify_rust::Timeout::Milliseconds(self.timeout_ms)
        };

        let result = notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .urgency(notify_urgency)
            .timeout(timeout)
            .show();

        if let Err(e) = result {
            tracing::warn!("Failed to send notification: {}", e);
        }
    }

    /// Send a notification with an icon
    pub fn send_with_icon(&self, title: &str, body: &str, urgency: Urgency, icon: &str) {
        if !self.enabled {
            return;
        }

        let notify_urgency = match urgency {
            Urgency::Low => notify_rust::Urgency::Low,
            Urgency::Normal => notify_rust::Urgency::Normal,
            Urgency::Critical => notify_rust::Urgency::Critical,
        };

        let timeout = if self.timeout_ms == 0 {
            notify_rust::Timeout::Never
        } else {
            notify_rust::Timeout::Milliseconds(self.timeout_ms)
        };

        let result = notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .icon(icon)
            .urgency(notify_urgency)
            .timeout(timeout)
            .show();

        if let Err(e) = result {
            tracing::warn!("Failed to send notification: {}", e);
        }
    }
}

impl Default for NotificationService {
    fn default() -> Self {
        Self::new(true, 5000)
    }
}
