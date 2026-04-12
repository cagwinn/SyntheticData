//! Device fingerprint model for channel-aware transaction enrichment.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Realistic device fingerprint for online/mobile transactions.
///
/// Replaces the simple `DEV-{hash}` pattern with a structured device model
/// that enables device-based anomaly detection (reuse tracking, trust scoring,
/// impossible travel when combined with geolocation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceFingerprint {
    /// Unique device identifier (realistic UUID format)
    pub device_id: String,
    /// Device model (e.g., "iPhone 15 Pro", "Samsung Galaxy S24", "Windows Desktop")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    /// Operating system (e.g., "iOS", "Android", "Windows", "macOS")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    /// OS version (e.g., "17.4", "14", "11")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    /// Screen resolution (e.g., "2556x1179", "1920x1080")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_resolution: Option<String>,
    /// Browser (for web transactions)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser: Option<String>,
    /// Whether this device has been seen before for this customer
    #[serde(default)]
    pub is_known_device: bool,
    /// When this device was first seen for this customer
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_first_seen: Option<DateTime<Utc>>,
    /// Trust score (0.0 = brand new/suspicious, 1.0 = well-established)
    #[serde(default)]
    pub device_trust_score: f64,
}

/// Static device profile tables for realistic generation.
pub struct DeviceProfiles;

impl DeviceProfiles {
    /// Mobile device models with OS.
    pub const MOBILE_DEVICES: &[(&str, &str)] = &[
        ("iPhone 15 Pro", "iOS"),
        ("iPhone 15", "iOS"),
        ("iPhone 14", "iOS"),
        ("iPhone 13", "iOS"),
        ("iPhone SE", "iOS"),
        ("Samsung Galaxy S24", "Android"),
        ("Samsung Galaxy S23", "Android"),
        ("Samsung Galaxy A54", "Android"),
        ("Google Pixel 8", "Android"),
        ("Google Pixel 7", "Android"),
        ("OnePlus 12", "Android"),
        ("Xiaomi 14", "Android"),
    ];

    /// Desktop OS options.
    pub const DESKTOP_OS: &[(&str, &str)] = &[
        ("Windows Desktop", "Windows"),
        ("Windows Desktop", "Windows"),
        ("MacBook Pro", "macOS"),
        ("MacBook Air", "macOS"),
        ("iMac", "macOS"),
        ("Linux Desktop", "Linux"),
    ];

    /// Browser options for web transactions.
    pub const BROWSERS: &[&str] = &[
        "Chrome 123",
        "Chrome 122",
        "Safari 17",
        "Firefox 124",
        "Edge 123",
    ];

    /// Screen resolutions.
    pub const MOBILE_RESOLUTIONS: &[&str] = &[
        "2556x1179",
        "2796x1290",
        "2340x1080",
        "2400x1080",
        "1792x828",
    ];

    pub const DESKTOP_RESOLUTIONS: &[&str] = &[
        "1920x1080",
        "2560x1440",
        "3840x2160",
        "1440x900",
        "1680x1050",
    ];
}
