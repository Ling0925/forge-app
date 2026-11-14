//! VoiceOver announcement bridge (§8). `forge-state` decides *what* to say
//! (merging, rate limits); this module only hands the text to the platform.
//!
//! macOS posts `NSAccessibilityAnnouncementRequestedNotification` on the
//! application object with `NSAccessibilityAnnouncementKey` and
//! `NSAccessibilityPriorityKey`. Other platforms are a no-op.

use forge_state::AnnouncementPriority;
use std::cell::RefCell;
use std::rc::Rc;

pub trait Announcer {
    fn announce(&self, text: &str, priority: AnnouncementPriority);
}

/// Discards announcements (Linux/Windows builds, headless runs).
#[derive(Default)]
pub struct NoopAnnouncer;

impl Announcer for NoopAnnouncer {
    fn announce(&self, _text: &str, _priority: AnnouncementPriority) {}
}

/// Records what would have been spoken (tests).
#[derive(Default)]
pub struct RecordingAnnouncer {
    pub spoken: RefCell<Vec<(String, AnnouncementPriority)>>,
}

impl Announcer for RecordingAnnouncer {
    fn announce(&self, text: &str, priority: AnnouncementPriority) {
        self.spoken.borrow_mut().push((text.to_owned(), priority));
    }
}

/// `NSAccessibilityPriorityLevel` raw values.
pub fn ns_priority(priority: AnnouncementPriority) -> i64 {
    match priority {
        AnnouncementPriority::Low => 10,
        AnnouncementPriority::Medium => 50,
        AnnouncementPriority::High => 90,
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use objc2::MainThreadMarker;
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{
        NSAccessibilityAnnouncementKey, NSAccessibilityAnnouncementRequestedNotification,
        NSAccessibilityPostNotificationWithUserInfo, NSAccessibilityPriorityKey, NSApplication,
        NSWorkspace,
    };
    use objc2_foundation::{NSDictionary, NSNumber, NSString};

    pub struct MacAnnouncer;

    impl Announcer for MacAnnouncer {
        fn announce(&self, text: &str, priority: AnnouncementPriority) {
            // gpui runs views on the main thread; never post from elsewhere.
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let app = NSApplication::sharedApplication(mtm);
            let message = NSString::from_str(text);
            let level = NSNumber::numberWithInteger(ns_priority(priority) as isize);
            let objects: [&AnyObject; 2] = [&message, &level];
            // SAFETY: the keys/notification name are AppKit's exported
            // NSString constants; `app` is a live NSApplication (an
            // NSAccessibilityElement) and every object outlives the call.
            unsafe {
                let keys = [NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey];
                let info = NSDictionary::from_slices(&keys, &objects);
                NSAccessibilityPostNotificationWithUserInfo(
                    &app,
                    NSAccessibilityAnnouncementRequestedNotification,
                    Some(&info),
                );
            }
        }
    }

    pub fn increase_contrast() -> bool {
        NSWorkspace::sharedWorkspace().accessibilityDisplayShouldIncreaseContrast()
    }
}

/// The announcer for this platform.
pub fn platform() -> Rc<dyn Announcer> {
    #[cfg(target_os = "macos")]
    {
        Rc::new(mac::MacAnnouncer)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Rc::new(NoopAnnouncer)
    }
}

/// System "Increase contrast" (macOS); `FORGE_UI_CONTRAST=high` forces it.
pub fn increase_contrast() -> bool {
    let forced = std::env::var("FORGE_UI_CONTRAST").is_ok_and(|v| v == "high");
    #[cfg(target_os = "macos")]
    {
        forced || mac::increase_contrast()
    }
    #[cfg(not(target_os = "macos"))]
    {
        forced
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ns_priority_maps_to_appkit_levels() {
        assert_eq!(ns_priority(AnnouncementPriority::Low), 10);
        assert_eq!(ns_priority(AnnouncementPriority::Medium), 50);
        assert_eq!(ns_priority(AnnouncementPriority::High), 90);
    }

    #[test]
    fn recording_announcer_captures_text_and_priority() {
        let announcer = RecordingAnnouncer::default();
        announcer.announce("已停止", AnnouncementPriority::High);
        announcer.announce("开始运行", AnnouncementPriority::Medium);
        let spoken = announcer.spoken.borrow();
        assert_eq!(
            spoken.as_slice(),
            [
                ("已停止".to_owned(), AnnouncementPriority::High),
                ("开始运行".to_owned(), AnnouncementPriority::Medium),
            ]
        );
    }

    #[test]
    fn noop_announcer_accepts_everything() {
        NoopAnnouncer.announce("任意文本", AnnouncementPriority::Low);
    }

    #[test]
    fn platform_announcer_is_constructible() {
        let _announcer = platform();
    }
}
