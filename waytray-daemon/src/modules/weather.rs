//! Weather module - displays weather using wttr.in (no API key required)

use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::config::WeatherModuleConfig;
use super::{Module, ModuleContext, ModuleItem};

/// Response from wttr.in JSON API
#[derive(Debug, Deserialize)]
struct WttrResponse {
    current_condition: Vec<CurrentCondition>,
    nearest_area: Vec<NearestArea>,
}

#[derive(Debug, Deserialize)]
struct CurrentCondition {
    #[serde(rename = "temp_C")]
    temp_c: String,
    #[serde(rename = "temp_F")]
    temp_f: String,
    #[serde(rename = "FeelsLikeC")]
    feels_like_c: String,
    #[serde(rename = "FeelsLikeF")]
    feels_like_f: String,
    humidity: String,
    #[serde(rename = "weatherDesc")]
    weather_desc: Vec<WeatherDesc>,
    #[serde(rename = "weatherCode")]
    weather_code: String,
}

#[derive(Debug, Deserialize)]
struct WeatherDesc {
    value: String,
}

#[derive(Debug, Deserialize)]
struct NearestArea {
    #[serde(rename = "areaName")]
    area_name: Vec<AreaValue>,
    country: Vec<AreaValue>,
}

#[derive(Debug, Deserialize)]
struct AreaValue {
    value: String,
}

/// Weather module that displays current weather
pub struct WeatherModule {
    config: RwLock<WeatherModuleConfig>,
    http_client: reqwest::Client,
}

impl WeatherModule {
    pub fn new(config: WeatherModuleConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent("curl/7.68.0") // wttr.in works better with curl user agent
            .build()
            .expect("Failed to create HTTP client");

        Self { config: RwLock::new(config), http_client }
    }

    /// Build the wttr.in URL
    async fn build_url(&self) -> String {
        let config = self.config.read().await;
        let location = if config.location.is_empty() {
            String::new()
        } else {
            // URL-encode the location
            urlencoding::encode(&config.location).into_owned()
        };

        // JSON format includes both temp_C and temp_F, no need for units param
        format!("https://wttr.in/{}?format=j1", location)
    }

    /// Fetch weather data from wttr.in
    async fn fetch_weather(&self) -> Option<WttrResponse> {
        let url = self.build_url().await;
        tracing::debug!("Fetching weather from: {}", url);

        match self.http_client.get(&url).send().await {
            Ok(response) => {
                if !response.status().is_success() {
                    tracing::warn!("Weather API returned status: {}", response.status());
                    return None;
                }

                match response.json::<WttrResponse>().await {
                    Ok(data) => Some(data),
                    Err(e) => {
                        tracing::warn!("Failed to parse weather response: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch weather: {}", e);
                None
            }
        }
    }

    /// Get weather icon name based on weather code
    fn get_weather_icon(weather_code: &str) -> &'static str {
        // wttr.in weather codes: https://www.worldweatheronline.com/developer/api/docs/weather-icons.aspx
        match weather_code {
            // Clear/Sunny
            "113" => "weather-clear",
            // Partly cloudy
            "116" => "weather-few-clouds",
            // Cloudy
            "119" => "weather-overcast",
            // Very cloudy
            "122" => "weather-overcast",
            // Fog/Mist
            "143" | "248" | "260" => "weather-fog",
            // Light rain/drizzle
            "176" | "263" | "266" | "293" | "296" | "353" => "weather-showers-scattered",
            // Rain
            "299" | "302" | "305" | "308" | "356" | "359" => "weather-showers",
            // Snow
            "179" | "182" | "185" | "227" | "230" | "317" | "320" | "323" | "326" | "329" | "332" | "335" | "338" | "368" | "371" | "374" | "377" => "weather-snow",
            // Thunderstorm
            "200" | "386" | "389" | "392" | "395" => "weather-storm",
            // Sleet
            "281" | "284" | "311" | "314" | "350" | "362" | "365" => "weather-snow",
            // Default
            _ => "weather-few-clouds",
        }
    }

    /// Create module item from weather data
    async fn create_module_item(&self, data: &WttrResponse) -> Option<ModuleItem> {
        let condition = data.current_condition.first()?;
        let area = data.nearest_area.first();

        let config = self.config.read().await;
        let use_fahrenheit = config.units.to_lowercase() == "fahrenheit";

        let (temp, feels_like, unit) = if use_fahrenheit {
            (&condition.temp_f, &condition.feels_like_f, "°F")
        } else {
            (&condition.temp_c, &condition.feels_like_c, "°C")
        };

        let label = format!("{}{}", temp, unit);

        let description = condition
            .weather_desc
            .first()
            .map(|d| d.value.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        let location_str = area
            .map(|a| {
                let city = a.area_name.first().map(|n| n.value.as_str()).unwrap_or("Unknown");
                let country = a.country.first().map(|c| c.value.as_str()).unwrap_or("");
                if country.is_empty() {
                    city.to_string()
                } else {
                    format!("{}, {}", city, country)
                }
            })
            .unwrap_or_else(|| "Unknown location".to_string());

        let tooltip = format!(
            "{}\n{}{} (feels like {}{})\nHumidity: {}%\n{}",
            description, temp, unit, feels_like, unit, condition.humidity, location_str
        );

        let icon_name = Self::get_weather_icon(&condition.weather_code);

        Some(ModuleItem {
            id: "weather:current".to_string(),
            module: "weather".to_string(),
            label,
            icon_name: Some(icon_name.to_string()),
            icon_pixmap: None,
            icon_width: 0,
            icon_height: 0,
            tooltip: Some(tooltip),
            actions: Vec::new(),
        })
    }
}

#[async_trait]
impl Module for WeatherModule {
    fn name(&self) -> &str {
        "weather"
    }

    fn enabled(&self) -> bool {
        self.config.try_read().map(|c| c.enabled).unwrap_or(true)
    }

    async fn start(&self, ctx: Arc<ModuleContext>) {
        if !self.config.read().await.enabled {
            return;
        }

        {
            let config = self.config.read().await;
            tracing::info!("Weather module starting, location: {}",
                if config.location.is_empty() { "auto-detect" } else { &config.location });
        }

        // Fetch initial weather
        if let Some(data) = self.fetch_weather().await {
            if let Some(item) = self.create_module_item(&data).await {
                ctx.send_items("weather", vec![item]);
            }
        } else {
            // Send empty on failure, will retry on next interval
            ctx.send_items("weather", vec![]);
        }

        // Poll at configured interval (re-read each iteration for hot reload)
        loop {
            let interval = {
                let config = self.config.read().await;
                Duration::from_secs(config.interval_seconds)
            };

            tokio::select! {
                _ = ctx.cancelled() => break,
                _ = tokio::time::sleep(interval) => {
                    if let Some(data) = self.fetch_weather().await {
                        if let Some(item) = self.create_module_item(&data).await {
                            ctx.send_items("weather", vec![item]);
                        }
                    }
                }
            }
        }
    }

    async fn stop(&self) {
        tracing::info!("Weather module stopped");
    }

    async fn invoke_action(&self, _item_id: &str, _action_id: &str, _x: i32, _y: i32) {
        // Weather module has no actions
    }

    async fn reload_config(&self, config: &crate::config::Config) -> bool {
        if let Some(ref weather_config) = config.modules.weather {
            let mut current = self.config.write().await;
            *current = weather_config.clone();
            tracing::debug!("Weather module config reloaded");
            true
        } else {
            false
        }
    }
}
