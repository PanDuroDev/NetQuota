use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileKind {
    #[default]
    Normal,
    DataSaver,
    StrictDataSaver,
    Gaming,
    Custom,
}

impl ProfileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProfileKind::Normal => "normal",
            ProfileKind::DataSaver => "data_saver",
            ProfileKind::StrictDataSaver => "strict_data_saver",
            ProfileKind::Gaming => "gaming",
            ProfileKind::Custom => "custom",
        }
    }
}

/// Default video target implied by a profile. The extension can still override
/// it per site; `Automatic` in a profile means "leave sites alone".
pub fn video_quality_for(kind: ProfileKind) -> VideoQuality {
    match kind {
        ProfileKind::Normal | ProfileKind::Gaming => VideoQuality::Off,
        ProfileKind::DataSaver => VideoQuality::BalancedSaver,
        ProfileKind::StrictDataSaver => VideoQuality::ExtremeSaver,
        ProfileKind::Custom => VideoQuality::Off,
    }
}

/// Preferred video resolution. `Off` leaves the website alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoQuality {
    Off,
    UltraSaver,
    ExtremeSaver,
    BalancedSaver,
    NormalSaver,
    Custom(u32),
}

impl VideoQuality {
    pub fn for_profile(kind: ProfileKind) -> Self {
        video_quality_for(kind)
    }

    pub fn target_height(self) -> Option<u32> {
        match self {
            VideoQuality::Off => None,
            VideoQuality::UltraSaver => Some(144),
            VideoQuality::ExtremeSaver => Some(240),
            VideoQuality::BalancedSaver => Some(360),
            VideoQuality::NormalSaver => Some(480),
            VideoQuality::Custom(h) => Some(h.clamp(144, 4320)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileSettings {
    pub kind: ProfileKind,
    pub block_background: bool,
    pub foreground_only: bool,
    pub block_on_metered: bool,
    pub block_autoplay: bool,
    pub video_quality: VideoQuality,
    /// Cap for background traffic. Only enforced when a shaping provider is
    /// available; the firewall provider cannot enforce a rate.
    pub background_kbps: Option<u32>,
    /// Soft threshold for automatic saver activation (0.0..=1.0).
    pub auto_saver_ratio: f64,
}

impl ProfileSettings {
    pub fn preset(kind: ProfileKind) -> Self {
        match kind {
            ProfileKind::Normal => Self {
                kind,
                block_background: false,
                foreground_only: false,
                block_on_metered: false,
                block_autoplay: false,
                video_quality: VideoQuality::Off,
                background_kbps: None,
                auto_saver_ratio: 1.0,
            },
            ProfileKind::DataSaver => Self {
                kind,
                block_background: false,
                foreground_only: false,
                block_on_metered: false,
                block_autoplay: true,
                video_quality: VideoQuality::BalancedSaver,
                background_kbps: Some(100),
                auto_saver_ratio: 0.8,
            },
            ProfileKind::StrictDataSaver => Self {
                kind,
                block_background: true,
                foreground_only: false,
                block_on_metered: true,
                block_autoplay: true,
                video_quality: VideoQuality::ExtremeSaver,
                background_kbps: Some(32),
                auto_saver_ratio: 0.6,
            },
            ProfileKind::Gaming => Self {
                kind,
                block_background: true,
                foreground_only: false,
                block_on_metered: false,
                block_autoplay: true,
                video_quality: VideoQuality::NormalSaver,
                background_kbps: Some(64),
                auto_saver_ratio: 0.9,
            },
            ProfileKind::Custom => Self {
                kind,
                block_background: false,
                foreground_only: false,
                block_on_metered: false,
                block_autoplay: false,
                video_quality: VideoQuality::Off,
                background_kbps: None,
                auto_saver_ratio: 0.8,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_saver_blocks_background_and_lowers_video() {
        let p = ProfileSettings::preset(ProfileKind::StrictDataSaver);
        assert!(p.block_background);
        assert_eq!(p.video_quality.target_height(), Some(240));
    }

    #[test]
    fn normal_profile_does_not_change_video() {
        let p = ProfileSettings::preset(ProfileKind::Normal);
        assert_eq!(p.video_quality.target_height(), None);
        assert!(!p.block_autoplay);
    }

    #[test]
    fn custom_quality_is_clamped() {
        assert_eq!(VideoQuality::Custom(10).target_height(), Some(144));
        assert_eq!(VideoQuality::Custom(2160).target_height(), Some(2160));
    }
}
