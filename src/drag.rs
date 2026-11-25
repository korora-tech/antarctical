//! The arithmetic behind dragging and resizing events in the time grid: pixels in, snapped
//! wall-clock blocks out. Kept free of UI types so it can be tested without a window.

use jiff::civil::Date;
use jiff::tz::TimeZone;

use crate::model::{self, Event, When};

/// Drags snap to this many minutes.
pub const SNAP_MINUTES: i32 = 15;
const DAY_MINUTES: i32 = 24 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Move the whole block (to another time, or another day).
    Move,
    ResizeStart,
    ResizeEnd,
}

/// A timed event inside one day, in minutes on the wall clock (`end` may be 24:00).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub date: Date,
    pub start: i32,
    pub end: i32,
}

/// Vertical pixels dragged, as a whole number of snap steps in minutes.
pub fn snapped_minutes(dy_px: f32, hour_px: f32) -> i32 {
    ((dy_px / hour_px * 60.0 / SNAP_MINUTES as f32).round() as i32) * SNAP_MINUTES
}

/// Horizontal pixels dragged, as whole columns.
pub fn dragged_days(dx_px: f32, column_px: f32) -> i32 {
    if column_px <= 0.0 {
        return 0;
    }
    (dx_px / column_px).round() as i32
}

/// The block after dragging by `minutes` and `days`. Blocks never leave their day vertically
/// and a resize never shrinks below one snap step.
pub fn apply(block: Block, mode: Mode, minutes: i32, days: i32) -> Block {
    match mode {
        Mode::Move => {
            let length = block.end - block.start;
            let start = (block.start + minutes).clamp(0, DAY_MINUTES - length);
            Block {
                date: model::add_days(block.date, days),
                start,
                end: start + length,
            }
        }
        Mode::ResizeStart => Block {
            start: (block.start + minutes).clamp(0, block.end - SNAP_MINUTES),
            ..block
        },
        Mode::ResizeEnd => Block {
            end: (block.end + minutes).clamp(block.start + SNAP_MINUTES, DAY_MINUTES),
            ..block
        },
    }
}

/// The block made by dragging across empty space: from where the pointer went down (`anchor`,
/// minutes) by `delta` minutes, either way. No movement gives an hour, the length Google uses.
pub fn create_block(date: Date, anchor: i32, delta: i32) -> Block {
    let anchor = anchor.clamp(0, DAY_MINUTES - SNAP_MINUTES);
    if delta == 0 {
        let start = anchor.min(DAY_MINUTES - 60);
        return Block {
            date,
            start,
            end: start + 60,
        };
    }
    let other = (anchor + delta).clamp(0, DAY_MINUTES);
    let (start, end) = (anchor.min(other), anchor.max(other));
    // Dragging up from the very top (or down off the bottom) still yields a real slot.
    let end = end.max(start + SNAP_MINUTES);
    Block {
        date,
        start: start.min(end - SNAP_MINUTES),
        end,
    }
}

/// The block an event occupies, when it is a timed event within a single day (the only kind
/// that is dragged).
pub fn block_of(event: &Event, tz: &TimeZone) -> Option<Block> {
    let When::Timed { start, end } = event.when else {
        return None;
    };
    let (s, e) = (start.to_zoned(tz.clone()), end.to_zoned(tz.clone()));
    if e <= s {
        return None;
    }
    let minute = |z: &jiff::Zoned| z.hour() as i32 * 60 + z.minute() as i32;
    let date = s.date();
    let end = if e.date() == date {
        minute(&e)
    } else if e.date() == model::add_days(date, 1) && minute(&e) == 0 && e.second() == 0 {
        DAY_MINUTES
    } else {
        return None;
    };
    // Only whole minutes can be written back faithfully.
    (s.second() == 0 && s.subsec_nanosecond() == 0 && e.subsec_nanosecond() == 0).then_some(Block {
        date,
        start: minute(&s),
        end,
    })
}

/// The event time for a block.
pub fn when_of(block: Block, tz: &TimeZone) -> Option<When> {
    let at = |date: Date, minutes: i32| {
        let time = jiff::civil::time((minutes / 60) as i8, (minutes % 60) as i8, 0, 0);
        tz.to_ambiguous_zoned(date.to_datetime(time))
            .compatible()
            .ok()
            .map(|z| z.timestamp())
    };
    let end = if block.end >= DAY_MINUTES {
        at(model::add_days(block.date, 1), 0)?
    } else {
        at(block.date, block.end)?
    };
    Some(When::Timed {
        start: at(block.date, block.start)?,
        end,
    })
}

/// The event time moved by `days` whole days, keeping the wall-clock time (so a move across a
/// clock change keeps 09:00 at 09:00). `None` when the time does not exist on the new day.
pub fn shifted_days(when: &When, days: i32, tz: &TimeZone) -> Option<When> {
    match when {
        When::AllDay { start, end } => Some(When::AllDay {
            start: model::add_days(*start, days),
            end: model::add_days(*end, days),
        }),
        When::Timed { start, end } => {
            let shift = |t: &jiff::Timestamp| {
                let z = t.to_zoned(tz.clone());
                let moved = model::add_days(z.date(), days).to_datetime(z.time());
                tz.to_ambiguous_zoned(moved)
                    .compatible()
                    .ok()
                    .map(|z| z.timestamp())
            };
            Some(When::Timed {
                start: shift(start)?,
                end: shift(end)?,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn block(start: i32, end: i32) -> Block {
        Block {
            date: date(2026, 11, 25),
            start,
            end,
        }
    }

    #[test]
    fn pixels_snap_to_quarter_hours() {
        // 48 px per hour: 12 px is a quarter of an hour.
        assert_eq!(snapped_minutes(12.0, 48.0), 15);
        assert_eq!(snapped_minutes(5.0, 48.0), 0);
        assert_eq!(snapped_minutes(7.0, 48.0), 15);
        assert_eq!(snapped_minutes(-24.0, 48.0), -30);
        assert_eq!(dragged_days(70.0, 100.0), 1);
        assert_eq!(dragged_days(-40.0, 100.0), 0);
        assert_eq!(dragged_days(10.0, 0.0), 0);
    }

    #[test]
    fn moving_keeps_the_length_and_stays_inside_the_day() {
        let b = block(9 * 60, 10 * 60);
        let moved = apply(b, Mode::Move, 90, 2);
        assert_eq!(
            moved,
            Block {
                date: date(2026, 11, 27),
                start: 10 * 60 + 30,
                end: 11 * 60 + 30
            }
        );
        assert_eq!(apply(b, Mode::Move, -600, 0).start, 0);
        let late = apply(b, Mode::Move, 2000, 0);
        assert_eq!((late.start, late.end), (23 * 60, 24 * 60));
    }

    #[test]
    fn resizing_changes_one_edge_and_keeps_a_minimum() {
        let b = block(9 * 60, 10 * 60);
        assert_eq!(apply(b, Mode::ResizeEnd, 30, 3), block(540, 630));
        assert_eq!(apply(b, Mode::ResizeEnd, -500, 0), block(540, 555));
        assert_eq!(apply(b, Mode::ResizeEnd, 5000, 0).end, 24 * 60);
        assert_eq!(apply(b, Mode::ResizeStart, -30, 0), block(510, 600));
        assert_eq!(apply(b, Mode::ResizeStart, 500, 0), block(585, 600));
        assert_eq!(apply(b, Mode::ResizeStart, -5000, 0).start, 0);
    }

    #[test]
    fn dragging_across_empty_space_makes_a_slot_either_way() {
        let d = date(2026, 11, 25);
        let at = |h: i32, m: i32| h * 60 + m;
        let b = create_block(d, at(14, 0), 90);
        assert_eq!((b.start, b.end), (at(14, 0), at(15, 30)));
        let b = create_block(d, at(14, 0), -45);
        assert_eq!((b.start, b.end), (at(13, 15), at(14, 0)));
        let b = create_block(d, at(14, 0), 0);
        assert_eq!((b.start, b.end), (at(14, 0), at(15, 0)));
        let b = create_block(d, at(23, 30), 0);
        assert_eq!((b.start, b.end), (at(23, 0), at(24, 0)));
        let b = create_block(d, at(0, 0), -60);
        assert_eq!((b.start, b.end), (0, SNAP_MINUTES));
        let b = create_block(d, at(23, 45), 500);
        assert_eq!((b.start, b.end), (at(23, 45), at(24, 0)));
        assert_eq!(b.date, d);
    }

    fn timed(start: &str, end: &str) -> Event {
        Event {
            id: "e".into(),
            calendar_id: "c".into(),
            title: "t".into(),
            when: When::Timed {
                start: start.parse().unwrap(),
                end: end.parse().unwrap(),
            },
            color: None,
            location: None,
            description: None,
            html_link: None,
            meet_link: None,
            organizer: None,
            attendees: vec![],
            reminders: vec![],
            tentative: false,
            kind: Default::default(),
        }
    }

    #[test]
    fn only_single_day_timed_events_have_a_block() {
        let tz = TimeZone::UTC;
        assert_eq!(
            block_of(&timed("2026-11-25T09:30:00Z", "2026-11-25T10:15:00Z"), &tz),
            Some(block(570, 615))
        );
        // Ending at midnight is the end of that day.
        assert_eq!(
            block_of(&timed("2026-11-25T23:00:00Z", "2026-11-26T00:00:00Z"), &tz),
            Some(block(23 * 60, 24 * 60))
        );
        assert_eq!(
            block_of(&timed("2026-11-25T22:00:00Z", "2026-11-26T01:00:00Z"), &tz),
            None
        );
        assert_eq!(
            block_of(&timed("2026-11-25T09:00:00Z", "2026-11-25T09:00:00Z"), &tz),
            None
        );
        let mut all_day = timed("2026-11-25T09:00:00Z", "2026-11-25T10:00:00Z");
        all_day.when = When::AllDay {
            start: date(2026, 11, 25),
            end: date(2026, 11, 26),
        };
        assert_eq!(block_of(&all_day, &tz), None);
    }

    #[test]
    fn a_block_converts_back_to_the_same_instants() {
        let tz = TimeZone::UTC;
        let e = timed("2026-11-25T09:30:00Z", "2026-11-25T10:15:00Z");
        assert_eq!(when_of(block_of(&e, &tz).unwrap(), &tz), Some(e.when));
        let midnight = timed("2026-11-25T23:00:00Z", "2026-11-26T00:00:00Z");
        assert_eq!(
            when_of(block_of(&midnight, &tz).unwrap(), &tz),
            Some(midnight.when)
        );
    }

    #[test]
    fn shifting_days_keeps_the_wall_clock_time() {
        let tz = TimeZone::UTC;
        let timed = When::Timed {
            start: "2026-11-25T09:30:00Z".parse().unwrap(),
            end: "2026-11-25T10:00:00Z".parse().unwrap(),
        };
        assert_eq!(
            shifted_days(&timed, 3, &tz),
            Some(When::Timed {
                start: "2026-11-28T09:30:00Z".parse().unwrap(),
                end: "2026-11-28T10:00:00Z".parse().unwrap(),
            })
        );
        let all_day = When::AllDay {
            start: date(2026, 11, 30),
            end: date(2026, 12, 2),
        };
        assert_eq!(
            shifted_days(&all_day, -1, &tz),
            Some(When::AllDay {
                start: date(2026, 11, 29),
                end: date(2026, 12, 1),
            })
        );
        // Across a spring-forward in London the clock time holds, so the instant moves an hour.
        if let Ok(london) = TimeZone::get("Europe/London") {
            let morning = When::Timed {
                start: "2026-03-27T09:00:00Z".parse().unwrap(),
                end: "2026-03-27T10:00:00Z".parse().unwrap(),
            };
            assert_eq!(
                shifted_days(&morning, 2, &london),
                Some(When::Timed {
                    start: "2026-03-29T08:00:00Z".parse().unwrap(),
                    end: "2026-03-29T09:00:00Z".parse().unwrap(),
                })
            );
        }
    }
}
