//! Penumbra Clock (`penumbra-clock`)
//! Material 3 Expressive Alarms, World Clock, Timer & Stopwatch.

use chrono::Local;
use penumbra_services::audio;
use penumbra_services::clock::{
    format_alarm_days, format_city_time_and_offset, format_hms, preset_world_cities, AlarmEntry,
    ClockState, WorldCity,
};
use penumbra_services::theme::{self, MonetPalette};
use slint::{Color, ModelRc, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

slint::include_modules!();

fn hex_to_color(hex: &str) -> Color {
    let clean = hex.trim_start_matches('#');
    if clean.len() == 6 {
        if let (Ok(r), Ok(g), Ok(b)) = (
            u8::from_str_radix(&clean[0..2], 16),
            u8::from_str_radix(&clean[2..4], 16),
            u8::from_str_radix(&clean[4..6], 16),
        ) {
            return Color::from_rgb_u8(r, g, b);
        }
    }
    Color::from_rgb_u8(173, 198, 255)
}

fn apply_palette(ui: &ClockWindow) {
    let p = MonetPalette::load_from_ipc_path(&MonetPalette::default_ipc_path())
        .unwrap_or_else(|_| MonetPalette::default_dark());
    ui.set_surface(hex_to_color(&p.surface_container_lowest));
    ui.set_surface_low(hex_to_color(&p.surface_container_low));
    ui.set_surface_high(hex_to_color(&p.surface_container_high));
    ui.set_primary(hex_to_color(&p.primary));
    ui.set_on_primary(hex_to_color(&p.on_primary));
    ui.set_primary_container(hex_to_color(&p.primary_container));
    ui.set_on_primary_container(hex_to_color(&p.on_primary_container));
    ui.set_on_surface(hex_to_color(&p.on_surface));
    ui.set_on_surface_variant(hex_to_color(&p.on_surface_variant));
}

fn sync_state_to_ui(ui: &ClockWindow, state: &ClockState) {
    let alarm_rows: Vec<AlarmItemData> = state
        .alarms
        .iter()
        .map(|a| AlarmItemData {
            id: a.id as i32,
            time_text: format!("{:02}:{:02}", a.hour, a.minute).into(),
            label: a.label.clone().into(),
            days_text: format_alarm_days(&a.days).into(),
            enabled: a.enabled,
        })
        .collect();
    ui.set_alarms(ModelRc::from(Rc::new(VecModel::from(alarm_rows))));

    if let Some(ring_id) = state.active_alarm_ringing_id {
        ui.set_alarm_ringing(true);
        let text = state
            .alarms
            .iter()
            .find(|a| a.id == ring_id)
            .map(|a| format!("{:02}:{:02} {}", a.hour, a.minute, a.label))
            .unwrap_or_else(|| "Alarm".into());
        ui.set_ringing_alarm_text(text.into());
    } else {
        ui.set_alarm_ringing(false);
    }

    let next_txt = state
        .next_alarm_summary()
        .map(|(_, s)| s)
        .unwrap_or_default();
    ui.set_next_alarm_text(next_txt.into());

    // World Clock
    let now = Local::now();
    ui.set_local_time_text(now.format("%H:%M").to_string().into());
    ui.set_local_secs_text(now.format("%S").to_string().into());
    ui.set_local_date_text(now.format("%A, %b %e").to_string().into());

    let city_rows: Vec<WorldCityData> = state
        .cities
        .iter()
        .map(|c| {
            let (time_text, offset_text) = format_city_time_and_offset(c);
            WorldCityData {
                name: c.name.clone().into(),
                time_text: time_text.into(),
                offset_text: offset_text.into(),
            }
        })
        .collect();
    ui.set_world_cities(ModelRc::from(Rc::new(VecModel::from(city_rows))));

    let preset_rows: Vec<PresetCityData> = preset_world_cities()
        .into_iter()
        .map(|c| PresetCityData {
            added: state.cities.iter().any(|existing| existing.name == c.name),
            name: c.name.into(),
            tz_offset_mins: c.tz_offset_mins,
        })
        .collect();
    ui.set_preset_cities(ModelRc::from(Rc::new(VecModel::from(preset_rows))));

    // Timer
    if let Some(ref t) = state.timer {
        let rem = state.effective_timer_remaining();
        ui.set_timer_has_active(true);
        ui.set_timer_running(t.running);
        ui.set_timer_ringing(t.ringing);
        ui.set_timer_display(format_hms(rem).into());
        let prog = if t.duration_secs > 0 {
            (rem as f32 / t.duration_secs as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ui.set_timer_progress(prog);
    } else {
        ui.set_timer_has_active(false);
        ui.set_timer_running(false);
        ui.set_timer_ringing(false);
    }
}

fn format_sw(ms: u128) -> (String, String) {
    let total_cs = ms / 10;
    let cs = total_cs % 100;
    let total_s = total_cs / 100;
    let s = total_s % 60;
    let m = (total_s / 60) % 60;
    let h = total_s / 3600;
    let main = if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    };
    (main, format!(".{cs:02}"))
}

struct StopwatchTracker {
    running: bool,
    started_at: Option<Instant>,
    accumulated_ms: u128,
    last_lap_total_ms: u128,
    laps: Vec<LapItemData>,
}

impl StopwatchTracker {
    fn total_ms(&self) -> u128 {
        if self.running {
            if let Some(st) = self.started_at {
                return self.accumulated_ms + st.elapsed().as_millis();
            }
        }
        self.accumulated_ms
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();

    let ui = ClockWindow::new()?;
    apply_palette(&ui);

    let state = Rc::new(RefCell::new(ClockState::load()));
    sync_state_to_ui(&ui, &state.borrow());

    let sw = Rc::new(RefCell::new(StopwatchTracker {
        running: false,
        started_at: None,
        accumulated_ms: 0,
        last_lap_total_ms: 0,
        laps: Vec::new(),
    }));

    // Alarms callbacks
    {
        let weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_toggle_alarm(move |id, enabled| {
            let mut s = st.borrow_mut();
            if let Some(a) = s.alarms.iter_mut().find(|a| a.id == id as u64) {
                a.enabled = enabled;
                a.snoozed_until_epoch = 0;
            }
            let _ = s.save();
            if let Some(w) = weak.upgrade() {
                sync_state_to_ui(&w, &s);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_save_alarm(move |id, h, m, label, mon, tue, wed, thu, fri, sat, sun| {
            let mut s = st.borrow_mut();
            let days = [mon, tue, wed, thu, fri, sat, sun];
            if id >= 0 {
                if let Some(a) = s.alarms.iter_mut().find(|a| a.id == id as u64) {
                    a.hour = h.clamp(0, 23) as u8;
                    a.minute = m.clamp(0, 59) as u8;
                    a.label = label.to_string();
                    a.days = days;
                    a.enabled = true;
                    a.snoozed_until_epoch = 0;
                }
            } else {
                let new_id = s.next_alarm_id;
                s.next_alarm_id += 1;
                s.alarms.push(AlarmEntry {
                    id: new_id,
                    hour: h.clamp(0, 23) as u8,
                    minute: m.clamp(0, 59) as u8,
                    label: label.to_string(),
                    enabled: true,
                    days,
                    vibrate: true,
                    snoozed_until_epoch: 0,
                });
            }
            let _ = s.save();
            if let Some(w) = weak.upgrade() {
                sync_state_to_ui(&w, &s);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_delete_alarm(move |id| {
            let mut s = st.borrow_mut();
            s.alarms.retain(|a| a.id != id as u64);
            if s.active_alarm_ringing_id == Some(id as u64) {
                s.active_alarm_ringing_id = None;
                audio::stop_ringtone_loop();
            }
            let _ = s.save();
            if let Some(w) = weak.upgrade() {
                sync_state_to_ui(&w, &s);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_dismiss_alarm(move || {
            let mut s = st.borrow_mut();
            s.dismiss_ringing_alarm();
            audio::stop_ringtone_loop();
            let _ = s.save();
            if let Some(w) = weak.upgrade() {
                sync_state_to_ui(&w, &s);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_snooze_alarm(move || {
            let mut s = st.borrow_mut();
            s.snooze_ringing_alarm(10);
            audio::stop_ringtone_loop();
            let _ = s.save();
            if let Some(w) = weak.upgrade() {
                sync_state_to_ui(&w, &s);
            }
        });
    }

    // World Clock callbacks
    {
        let weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_toggle_city(move |name, offset_mins| {
            let mut s = st.borrow_mut();
            if let Some(idx) = s.cities.iter().position(|c| c.name == name.as_str()) {
                s.cities.remove(idx);
            } else {
                s.cities.push(WorldCity {
                    name: name.to_string(),
                    tz_offset_mins: offset_mins,
                });
            }
            let _ = s.save();
            if let Some(w) = weak.upgrade() {
                sync_state_to_ui(&w, &s);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_remove_city(move |name| {
            let mut s = st.borrow_mut();
            s.cities.retain(|c| c.name != name.as_str());
            let _ = s.save();
            if let Some(w) = weak.upgrade() {
                sync_state_to_ui(&w, &s);
            }
        });
    }

    // Timer callbacks
    {
        let weak = ui.as_weak();
        ui.on_timer_set_preset(move |secs| {
            if let Some(w) = weak.upgrade() {
                let s = secs.max(10);
                w.set_timer_setup_secs(s);
                w.set_timer_setup_display(format_hms(s as u64).into());
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_timer_adjust_setup(move |delta| {
            if let Some(w) = weak.upgrade() {
                let s = (w.get_timer_setup_secs() + delta).clamp(10, 35999);
                w.set_timer_setup_secs(s);
                w.set_timer_setup_display(format_hms(s as u64).into());
            }
        });
    }

    {
        let weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_timer_start(move || {
            let dur = weak
                .upgrade()
                .map(|w| w.get_timer_setup_secs().max(1) as u64)
                .unwrap_or(300);
            let mut s = st.borrow_mut();
            s.timer = None;
            s.timer_start_or_resume(dur);
            let _ = s.save();
            if let Some(w) = weak.upgrade() {
                sync_state_to_ui(&w, &s);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_timer_toggle_pause(move || {
            let mut s = st.borrow_mut();
            s.timer_toggle_pause();
            if s.timer.as_ref().is_none_or(|t| !t.ringing) && s.active_alarm_ringing_id.is_none() {
                audio::stop_ringtone_loop();
            }
            let _ = s.save();
            if let Some(w) = weak.upgrade() {
                sync_state_to_ui(&w, &s);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_timer_add_minute(move || {
            let mut s = st.borrow_mut();
            s.timer_add_minute();
            if s.active_alarm_ringing_id.is_none() {
                audio::stop_ringtone_loop();
            }
            let _ = s.save();
            if let Some(w) = weak.upgrade() {
                sync_state_to_ui(&w, &s);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let st = Rc::clone(&state);
        ui.on_timer_cancel(move || {
            let mut s = st.borrow_mut();
            s.timer_cancel();
            if s.active_alarm_ringing_id.is_none() {
                audio::stop_ringtone_loop();
            }
            let _ = s.save();
            if let Some(w) = weak.upgrade() {
                sync_state_to_ui(&w, &s);
            }
        });
    }

    // Stopwatch callbacks
    {
        let weak = ui.as_weak();
        let sw_c = Rc::clone(&sw);
        ui.on_sw_toggle(move || {
            let mut tracker = sw_c.borrow_mut();
            if tracker.running {
                if let Some(st) = tracker.started_at.take() {
                    tracker.accumulated_ms += st.elapsed().as_millis();
                }
                tracker.running = false;
            } else {
                tracker.started_at = Some(Instant::now());
                tracker.running = true;
            }
            if let Some(w) = weak.upgrade() {
                w.set_sw_running(tracker.running);
                w.set_sw_has_started(tracker.running || tracker.accumulated_ms > 0);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let sw_c = Rc::clone(&sw);
        ui.on_sw_lap(move || {
            let mut tracker = sw_c.borrow_mut();
            let total = tracker.total_ms();
            let split = total.saturating_sub(tracker.last_lap_total_ms);
            tracker.last_lap_total_ms = total;
            let idx = tracker.laps.len() + 1;
            let (sm, scs) = format_sw(split);
            let (tm, tcs) = format_sw(total);
            tracker.laps.insert(
                0,
                LapItemData {
                    num_text: format!("#{idx}").into(),
                    split_text: format!("+{sm}{scs}").into(),
                    total_text: format!("{tm}{tcs}").into(),
                },
            );
            if let Some(w) = weak.upgrade() {
                w.set_sw_laps(ModelRc::from(Rc::new(VecModel::from(tracker.laps.clone()))));
            }
        });
    }

    {
        let weak = ui.as_weak();
        let sw_c = Rc::clone(&sw);
        ui.on_sw_reset(move || {
            let mut tracker = sw_c.borrow_mut();
            tracker.running = false;
            tracker.started_at = None;
            tracker.accumulated_ms = 0;
            tracker.last_lap_total_ms = 0;
            tracker.laps.clear();
            if let Some(w) = weak.upgrade() {
                w.set_sw_running(false);
                w.set_sw_has_started(false);
                w.set_sw_main_text("00:00".into());
                w.set_sw_cs_text(".00".into());
                w.set_sw_laps(ModelRc::from(Rc::new(VecModel::default())));
            }
        });
    }

    // 50ms UI tick for stopwatch centiseconds + 500ms clock/theme sync
    let ui_timer = Timer::default();
    {
        let weak = ui.as_weak();
        let st = Rc::clone(&state);
        let sw_c = Rc::clone(&sw);
        let theme_path = theme::MonetPalette::default_ipc_path();
        let clock_path = ClockState::config_path();
        let mut last_theme_mtime = std::fs::metadata(&theme_path).and_then(|m| m.modified()).ok();
        let mut last_clock_mtime = std::fs::metadata(&clock_path).and_then(|m| m.modified()).ok();
        let mut tick_count: u32 = 0;

        ui_timer.start(TimerMode::Repeated, Duration::from_millis(50), move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            {
                let tracker = sw_c.borrow();
                if tracker.running {
                    let (main, cs) = format_sw(tracker.total_ms());
                    w.set_sw_main_text(main.into());
                    w.set_sw_cs_text(cs.into());
                }
            }
            tick_count = tick_count.wrapping_add(1);
            if tick_count % 10 == 0 {
                let mtime = std::fs::metadata(&theme_path).and_then(|m| m.modified()).ok();
                if mtime != last_theme_mtime {
                    last_theme_mtime = mtime;
                    apply_palette(&w);
                }
                let cmtime = std::fs::metadata(&clock_path).and_then(|m| m.modified()).ok();
                if cmtime != last_clock_mtime {
                    last_clock_mtime = cmtime;
                    *st.borrow_mut() = ClockState::load();
                }
                sync_state_to_ui(&w, &st.borrow());
            }
        });
    }

    ui.run()?;
    Ok(())
}
