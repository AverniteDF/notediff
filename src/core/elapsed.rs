//! Text and refresh schedule for the "Last edit" field of the status bar, free
//! of any toolkit types.
//!
//! The field reads `just now` during the first second, then `4 seconds ago`.
//! The elapsed time uses a single
//! unit: seconds below a minute, minutes below an hour, hours beyond. Values
//! are floored, so `119` seconds is `1 minute ago`.

/// `just now`, `4 seconds ago`, `1 minute ago`, `3 hours ago`.
pub fn format_elapsed(secs: u64) -> String {
    if secs == 0 {
        return "just now".to_string();
    }
    let (count, unit) = if secs < 60 {
        (secs, "second")
    } else if secs < 3600 {
        (secs / 60, "minute")
    } else {
        (secs / 3600, "hour")
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

/// Whole seconds to wait before the text for `elapsed_secs` can change. It
/// changes every second for the first minute, then only when the minute count
/// rolls over, and after an hour only when the hour count does.
pub fn next_refresh_secs(elapsed_secs: u64) -> u32 {
    let wait = if elapsed_secs < 60 {
        1
    } else if elapsed_secs < 3600 {
        60 - elapsed_secs % 60
    } else {
        3600 - elapsed_secs % 3600
    };
    wait as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds() {
        assert_eq!(format_elapsed(0), "just now");
        assert_eq!(format_elapsed(1), "1 second ago");
        assert_eq!(format_elapsed(4), "4 seconds ago");
        assert_eq!(format_elapsed(59), "59 seconds ago");
    }

    #[test]
    fn minutes() {
        assert_eq!(format_elapsed(60), "1 minute ago");
        assert_eq!(format_elapsed(119), "1 minute ago");
        assert_eq!(format_elapsed(300), "5 minutes ago");
        assert_eq!(format_elapsed(3599), "59 minutes ago");
    }

    #[test]
    fn hours() {
        assert_eq!(format_elapsed(3600), "1 hour ago");
        assert_eq!(format_elapsed(7199), "1 hour ago");
        assert_eq!(format_elapsed(7200), "2 hours ago");
        assert_eq!(format_elapsed(100 * 3600), "100 hours ago");
    }

    #[test]
    fn summary_format() {
        assert_eq!(format_elapsed(0), "just now");
        assert_eq!(format_elapsed(31), "31 seconds ago");
        assert_eq!(format_elapsed(300), "5 minutes ago");
    }

    #[test]
    fn refresh_waits_until_the_text_can_change() {
        assert_eq!(next_refresh_secs(0), 1);
        assert_eq!(next_refresh_secs(59), 1);
        assert_eq!(next_refresh_secs(60), 60);
        assert_eq!(next_refresh_secs(61), 59);
        assert_eq!(next_refresh_secs(119), 1);
        assert_eq!(next_refresh_secs(3599), 1);
        assert_eq!(next_refresh_secs(3600), 3600);
        assert_eq!(next_refresh_secs(3601), 3599);
    }

    #[test]
    fn refresh_lands_on_a_change_of_text() {
        for start in [0u64, 7, 59, 60, 61, 119, 125, 3599, 3600, 4000, 9000] {
            let next = start + u64::from(next_refresh_secs(start));
            if start >= 60 {
                assert_ne!(format_elapsed(start), format_elapsed(next), "from {start}");
            }
        }
    }
}
