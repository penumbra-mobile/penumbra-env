//! Persistent Alarms, Timers, and World Clock State shared between
//! `penumbra-clock` and `penumbra-shell` (SystemUI top bar & notification cards).

use chrono::{Datelike, FixedOffset, Local, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AlarmEntry {
    pub id: u64,
    pub hour: u8,
    pub minute: u8,
    pub label: String,
    pub enabled: bool,
    /// `[Mon, Tue, Wed, Thu, Fri, Sat, Sun]`; all false means one-shot.
    pub days: [bool; 7],
    pub vibrate: bool,
    pub snoozed_until_epoch: u64,
}

impl Default for AlarmEntry {
    fn default() -> Self {
        Self {
            id: 1,
            hour: 7,
            minute: 0,
            label: "Wake up".to_string(),
            enabled: false,
            days: [false; 7],
            vibrate: true,
            snoozed_until_epoch: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TimerState {
    pub duration_secs: u64,
    pub remaining_secs: u64,
    pub running: bool,
    pub end_epoch_secs: u64,
    pub ringing: bool,
    pub label: String,
}

impl Default for TimerState {
    fn default() -> Self {
        Self {
            duration_secs: 300,
            remaining_secs: 300,
            running: false,
            end_epoch_secs: 0,
            ringing: false,
            label: "Timer".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldCity {
    pub name: String,
    pub tz_offset_mins: i32,
}

impl Default for WorldCity {
    fn default() -> Self {
        Self {
            name: "London".to_string(),
            tz_offset_mins: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ClockState {
    pub alarms: Vec<AlarmEntry>,
    pub timer: Option<TimerState>,
    pub cities: Vec<WorldCity>,
    pub active_alarm_ringing_id: Option<u64>,
    pub last_triggered_minute_key: String,
    pub next_alarm_id: u64,
}

impl Default for ClockState {
    fn default() -> Self {
        Self {
            alarms: vec![
                AlarmEntry {
                    id: 1,
                    hour: 7,
                    minute: 0,
                    label: "Morning".to_string(),
                    enabled: false,
                    days: [true, true, true, true, true, false, false],
                    vibrate: true,
                    snoozed_until_epoch: 0,
                },
                AlarmEntry {
                    id: 2,
                    hour: 8,
                    minute: 30,
                    label: "Weekend".to_string(),
                    enabled: false,
                    days: [false, false, false, false, false, true, true],
                    vibrate: true,
                    snoozed_until_epoch: 0,
                },
            ],
            timer: None,
            cities: vec![
                WorldCity {
                    name: "London".to_string(),
                    tz_offset_mins: 0,
                },
                WorldCity {
                    name: "New York".to_string(),
                    tz_offset_mins: -300,
                },
                WorldCity {
                    name: "Tokyo".to_string(),
                    tz_offset_mins: 540,
                },
            ],
            active_alarm_ringing_id: None,
            last_triggered_minute_key: String::new(),
            next_alarm_id: 3,
        }
    }
}

pub fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn format_hms(total_secs: u64) -> String {
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

pub fn format_alarm_days(days: &[bool; 7]) -> String {
    const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    if days.iter().all(|&d| !d) {
        return "Once".to_string();
    }
    if days.iter().all(|&d| d) {
        return "Every day".to_string();
    }
    if *days == [true, true, true, true, true, false, false] {
        return "Weekdays".to_string();
    }
    if *days == [false, false, false, false, false, true, true] {
        return "Weekends".to_string();
    }
    NAMES
        .iter()
        .zip(days.iter())
        .filter_map(|(n, &on)| if on { Some(*n) } else { None })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Curated list of world cities for the World Clock city picker.
pub fn preset_world_cities() -> Vec<WorldCity> {
    vec![
        WorldCity { name: "Honolulu".into(), tz_offset_mins: -600 },
        WorldCity { name: "Los Angeles".into(), tz_offset_mins: -480 },
        WorldCity { name: "Denver".into(), tz_offset_mins: -420 },
        WorldCity { name: "Chicago".into(), tz_offset_mins: -360 },
        WorldCity { name: "New York".into(), tz_offset_mins: -300 },
        WorldCity { name: "São Paulo".into(), tz_offset_mins: -180 },
        WorldCity { name: "London".into(), tz_offset_mins: 0 },
        WorldCity { name: "Paris".into(), tz_offset_mins: 60 },
        WorldCity { name: "Berlin".into(), tz_offset_mins: 60 },
        WorldCity { name: "Bucharest".into(), tz_offset_mins: 120 },
        WorldCity { name: "Cairo".into(), tz_offset_mins: 120 },
        WorldCity { name: "Istanbul".into(), tz_offset_mins: 180 },
        WorldCity { name: "Dubai".into(), tz_offset_mins: 240 },
        WorldCity { name: "New Delhi".into(), tz_offset_mins: 330 },
        WorldCity { name: "Bangkok".into(), tz_offset_mins: 420 },
        WorldCity { name: "Singapore".into(), tz_offset_mins: 480 },
        WorldCity { name: "Seoul".into(), tz_offset_mins: 540 },
        WorldCity { name: "Tokyo".into(), tz_offset_mins: 540 },
        WorldCity { name: "Sydney".into(), tz_offset_mins: 660 },
        WorldCity { name: "Auckland".into(), tz_offset_mins: 780 },
    ]
}

pub fn format_city_time_and_offset(city: &WorldCity) -> (String, String) {
    let Some(tz) = FixedOffset::east_opt(city.tz_offset_mins * 60) else {
        return ("--:--".into(), String::new());
    };
    let utc_now = Utc::now();
    let city_dt = utc_now.with_timezone(&tz);
    let local_dt = Local::now();
    let local_offset_mins = local_dt.offset().local_minus_utc() / 60;
    let diff_mins = city.tz_offset_mins - local_offset_mins;

    let day_tag = if city_dt.date_naive() > local_dt.date_naive() {
        "Tomorrow"
    } else if city_dt.date_naive() < local_dt.date_naive() {
        "Yesterday"
    } else {
        "Today"
    };

    let diff_str = if diff_mins == 0 {
        format!("{day_tag} • Same time")
    } else {
        let sign = if diff_mins > 0 { "+" } else { "-" };
        let abs_m = diff_mins.abs();
        if abs_m % 60 == 0 {
            format!("{day_tag} • {sign}{} hr", abs_m / 60)
        } else {
            format!("{day_tag} • {sign}{}h {}m", abs_m / 60, abs_m % 60)
        }
    };
    (format!("{:02}:{:02}", city_dt.hour(), city_dt.minute()), diff_str)
}

impl ClockState {
    pub fn config_path() -> PathBuf {
        if let Ok(cfg_dir) = std::env::var("XDG_CONFIG_HOME") {
            PathBuf::from(cfg_dir).join("penumbra").join("clock.json")
        } else if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home).join(".config").join("penumbra").join("clock.json")
        } else {
            PathBuf::from("/etc/penumbra/clock.json")
        }
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        if path.exists() {
            if let Ok(data) = fs::read_to_string(&path) {
                if let Ok(cfg) = serde_json::from_str(&data) {
                    return cfg;
                }
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(tmp, path)
    }

    pub fn has_enabled_alarm(&self) -> bool {
        self.alarms.iter().any(|a| a.enabled)
    }

    /// Returns `(alarm_id, summary_text)` for the next enabled alarm, or ringing alarm if active.
    pub fn next_alarm_summary(&self) -> Option<(u64, String)> {
        if let Some(ring_id) = self.active_alarm_ringing_id {
            if let Some(a) = self.alarms.iter().find(|a| a.id == ring_id) {
                let lbl = if a.label.is_empty() { "Alarm" } else { &a.label };
                return Some((a.id, format!("{:02}:{:02} • {lbl}", a.hour, a.minute)));
            }
        }
        let now = Local::now();
        let cur_wd = now.weekday().num_days_from_monday() as usize;
        let cur_mins = now.hour() * 60 + now.minute();

        let mut best: Option<(u32, &AlarmEntry)> = None;
        for alarm in self.alarms.iter().filter(|a| a.enabled) {
            let alarm_mins = alarm.hour as u32 * 60 + alarm.minute as u32;
            let delta = if alarm.days.iter().all(|&d| !d) {
                if alarm_mins > cur_mins {
                    alarm_mins - cur_mins
                } else {
                    24 * 60 - cur_mins + alarm_mins
                }
            } else {
                let mut min_d = u32::MAX;
                for offset in 0..7u32 {
                    let wd = (cur_wd + offset as usize) % 7;
                    if alarm.days[wd] {
                        let d = offset * 24 * 60 + alarm_mins;
                        if d > cur_mins {
                            min_d = min_d.min(d - cur_mins);
                        } else if offset > 0 {
                            min_d = min_d.min(d + 7 * 24 * 60 - cur_mins);
                        }
                    }
                }
                if min_d == u32::MAX {
                    24 * 60
                } else {
                    min_d
                }
            };
            if best.is_none_or(|(b, _)| delta < b) {
                best = Some((delta, alarm));
            }
        }
        best.map(|(delta, a)| {
            let day_hint = if delta + cur_mins < 24 * 60 { "Today" } else { "Tomorrow" };
            let lbl = if a.label.is_empty() { "Alarm" } else { &a.label };
            (a.id, format!("{day_hint} {:02}:{:02} • {lbl}", a.hour, a.minute))
        })
    }

    pub fn effective_timer_remaining(&self) -> u64 {
        let Some(ref t) = self.timer else {
            return 0;
        };
        if t.running {
            t.end_epoch_secs.saturating_sub(now_epoch_secs())
        } else {
            t.remaining_secs
        }
    }

    /// Evaluates alarms and timers against current wall clock.
    /// Returns `(should_ring_now, changed)`.
    pub fn tick_background(&mut self) -> (bool, bool) {
        let now_epoch = now_epoch_secs();
        let now = Local::now();
        let minute_key = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            now.year(),
            now.month(),
            now.day(),
            now.hour(),
            now.minute()
        );
        let wd = now.weekday().num_days_from_monday() as usize;
        let mut changed = false;
        let mut should_ring = false;

        // 1. Check snoozed or scheduled alarms
        if self.active_alarm_ringing_id.is_none() {
            for alarm in &mut self.alarms {
                if !alarm.enabled {
                    continue;
                }
                if alarm.snoozed_until_epoch > 0 && now_epoch >= alarm.snoozed_until_epoch {
                    alarm.snoozed_until_epoch = 0;
                    self.active_alarm_ringing_id = Some(alarm.id);
                    changed = true;
                    should_ring = true;
                    break;
                }
                if self.last_triggered_minute_key != minute_key
                    && alarm.hour as u32 == now.hour()
                    && alarm.minute as u32 == now.minute()
                {
                    let matches_day = alarm.days.iter().all(|&d| !d) || alarm.days[wd];
                    if matches_day {
                        self.last_triggered_minute_key = minute_key.clone();
                        self.active_alarm_ringing_id = Some(alarm.id);
                        if alarm.days.iter().all(|&d| !d) {
                            // One-shot alarm turns off after firing (dismiss/snooze handles re-enabling if snoozed)
                            alarm.enabled = false;
                        }
                        changed = true;
                        should_ring = true;
                        break;
                    }
                }
            }
        }

        // 2. Check active countdown timer
        if let Some(ref mut t) = self.timer {
            if t.running && now_epoch >= t.end_epoch_secs {
                t.running = false;
                t.remaining_secs = 0;
                t.ringing = true;
                changed = true;
                should_ring = true;
            }
        }

        (should_ring, changed)
    }

    pub fn dismiss_ringing_alarm(&mut self) {
        if let Some(id) = self.active_alarm_ringing_id.take() {
            if let Some(a) = self.alarms.iter_mut().find(|a| a.id == id) {
                a.snoozed_until_epoch = 0;
            }
        }
    }

    pub fn snooze_ringing_alarm(&mut self, mins: u64) {
        if let Some(id) = self.active_alarm_ringing_id.take() {
            let until = now_epoch_secs() + mins * 60;
            if let Some(a) = self.alarms.iter_mut().find(|a| a.id == id) {
                a.enabled = true;
                a.snoozed_until_epoch = until;
            }
        }
    }

    pub fn timer_start_or_resume(&mut self, duration_secs: u64) {
        let now = now_epoch_secs();
        match self.timer {
            Some(ref mut t) => {
                if !t.running {
                    let rem = if t.remaining_secs == 0 {
                        duration_secs.max(1)
                    } else {
                        t.remaining_secs
                    };
                    t.remaining_secs = rem;
                    t.end_epoch_secs = now + rem;
                    t.running = true;
                    t.ringing = false;
                }
            }
            None => {
                let dur = duration_secs.max(1);
                self.timer = Some(TimerState {
                    duration_secs: dur,
                    remaining_secs: dur,
                    running: true,
                    end_epoch_secs: now + dur,
                    ringing: false,
                    label: "Timer".to_string(),
                });
            }
        }
    }

    pub fn timer_toggle_pause(&mut self) {
        let now = now_epoch_secs();
        if let Some(ref mut t) = self.timer {
            if t.ringing {
                t.ringing = false;
                t.running = false;
                t.remaining_secs = t.duration_secs;
            } else if t.running {
                t.remaining_secs = t.end_epoch_secs.saturating_sub(now);
                t.running = false;
            } else if t.remaining_secs > 0 {
                t.end_epoch_secs = now + t.remaining_secs;
                t.running = true;
            }
        }
    }

    pub fn timer_add_minute(&mut self) {
        let now = now_epoch_secs();
        if let Some(ref mut t) = self.timer {
            t.duration_secs += 60;
            if t.ringing {
                t.ringing = false;
                t.remaining_secs = 60;
                t.end_epoch_secs = now + 60;
                t.running = true;
            } else if t.running {
                t.end_epoch_secs += 60;
                t.remaining_secs = t.end_epoch_secs.saturating_sub(now);
            } else {
                t.remaining_secs += 60;
            }
        }
    }

    pub fn timer_cancel(&mut self) {
        self.timer = None;
    }
}

