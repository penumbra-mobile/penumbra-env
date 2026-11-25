//! Penumbra Phone / Dialer (`penumbra-calls`)
//! Real ModemManager1 Voice + CallAudioD + Shared Contacts & Messaging integration.

use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

use clap::Parser;
use slint::{Color, ComponentHandle, ModelRc, Timer, TimerMode, VecModel};
use tracing::info;

use penumbra_services::{contacts, i18n, telephony, theme};

slint::include_modules!();

#[derive(Parser, Debug)]
#[command(name = "penumbra-calls", about = "Penumbra Mobile Phone & Dialer")]
struct Args {
    /// Optional phone number to dial immediately on launch
    #[arg(long)]
    dial: Option<String>,
}

fn hex_to_color(hex: &str) -> Color {
    let s = hex.trim_start_matches('#');
    if s.len() >= 6 {
        let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(20);
        let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(20);
        let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(28);
        Color::from_rgb_u8(r, g, b)
    } else {
        Color::from_rgb_u8(20, 20, 28)
    }
}

fn apply_palette(ui: &CallsWindow) {
    let pal = theme::MonetPalette::load_from_ipc_path(&theme::MonetPalette::default_ipc_path())
        .unwrap_or_else(|_| theme::MonetPalette::default_dark());
    ui.set_bg_color(hex_to_color(&pal.surface_container_lowest));
    ui.set_surface_low(hex_to_color(&pal.surface_container_low));
    ui.set_surface_high(hex_to_color(&pal.surface_container_high));
    ui.set_primary(hex_to_color(&pal.primary));
    ui.set_on_primary(hex_to_color(&pal.on_primary));
    ui.set_primary_container(hex_to_color(&pal.primary_container));
    ui.set_on_primary_container(hex_to_color(&pal.on_primary_container));
    ui.set_on_surface(hex_to_color(&pal.on_surface));
    ui.set_on_surface_variant(hex_to_color(&pal.on_surface_variant));
}

fn format_epoch_relative(ts: u64) -> String {
    let now = telephony::now_epoch_secs();
    let diff = now.saturating_sub(ts);
    if diff < 60 {
        "Just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

fn reload_history(ui: &CallsWindow) {
    let rows: Vec<CallHistoryRow> = telephony::load_call_history()
        .into_iter()
        .map(|h| CallHistoryRow {
            id: h.id.into(),
            number: h.number.into(),
            display_name: h.contact_name.into(),
            direction: h.direction.into(),
            time_label: format_epoch_relative(h.timestamp_secs).into(),
        })
        .collect();
    ui.set_history(ModelRc::from(Rc::new(VecModel::from(rows))));
}

fn apply_calls_to_ui(
    ui: &CallsWindow,
    calls: &[telephony::ActiveCall],
    active_start: &mut Option<(String, u64)>,
) {
    let is_ringing_in = calls.iter().any(|c| c.state == telephony::CallState::RingingIn);
    if is_ringing_in {
        penumbra_services::audio::start_ringtone_loop();
    } else {
        penumbra_services::audio::stop_ringtone_loop();
    }

    if let Some(call) = calls.first() {
        if !ui.get_has_call() {
            ui.set_speaker_on(false);
            ui.set_mic_muted(false);
        }
        ui.set_has_call(true);
        ui.set_call_path(call.path.clone().into());
        ui.set_call_number(call.number.clone().into());
        ui.set_call_contact_name(call.contact_name.clone().into());
        ui.set_call_state_label(call.state.label().into());
        ui.set_is_incoming_ringing(call.state == telephony::CallState::RingingIn);

        if call.state == telephony::CallState::Active {
            let start = match active_start {
                Some((p, t)) if p == &call.path => *t,
                _ => {
                    let now = telephony::now_epoch_secs();
                    *active_start = Some((call.path.clone(), now));
                    now
                }
            };
            let elapsed = telephony::now_epoch_secs().saturating_sub(start);
            ui.set_call_duration_text(
                format!("{:02}:{:02}", elapsed / 60, elapsed % 60).into(),
            );
        } else {
            ui.set_call_duration_text("00:00".into());
        }
    } else {
        if ui.get_has_call() {
            *active_start = None;
            ui.set_speaker_on(false);
            ui.set_mic_muted(false);
            reload_history(ui);
        }
        ui.set_has_call(false);
        ui.set_call_path("".into());
        ui.set_is_incoming_ringing(false);
    }
}

enum TelephonyCmd {
    Dial(String),
    Accept(String),
    Hangup(String),
    ToggleSpeaker(bool),
    ToggleMic(bool),
    SetVolume(i32),
    SendDtmf(String, String),
    SyncCalls,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();
    let args = Args::parse();

    let lang = i18n::active_lang_code();
    let _ = slint::select_bundled_translation(if lang == "en" { "" } else { &lang });

    let ui = CallsWindow::new()?;
    apply_palette(&ui);
    reload_history(&ui);

    let active_start: std::sync::Arc<std::sync::Mutex<Option<(String, u64)>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));

    // Sync immediately before showing the window so incoming calls appear on frame 1.
    if let (Ok(calls), Ok(mut guard)) = (telephony::list_active_calls(), active_start.lock()) {
        apply_calls_to_ui(&ui, &calls, &mut guard);
    }

    // Dedicated background worker for all D-Bus, pactl, and amixer operations (0ms UI freeze).
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<TelephonyCmd>();
    {
        let weak = ui.as_weak();
        let active_start_w = std::sync::Arc::clone(&active_start);
        std::thread::spawn(move || {
            let mut was_in_call = false;
            let sync_and_push = |w: &slint::Weak<CallsWindow>, was_in: &mut bool| {
                if let Ok(calls) = telephony::list_active_calls() {
                    if calls.is_empty() && *was_in {
                        telephony::set_call_audio_mode(false);
                    }
                    *was_in = !calls.is_empty();
                    let w2 = w.clone();
                    let st = std::sync::Arc::clone(&active_start_w);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let (Some(win), Ok(mut guard)) = (w2.upgrade(), st.lock()) {
                            apply_calls_to_ui(&win, &calls, &mut guard);
                        }
                    });
                }
            };

            let mut queue = std::collections::VecDeque::new();
            while let Some(cmd) = queue.pop_front().or_else(|| cmd_rx.recv().ok()) {
                while let Ok(more) = cmd_rx.try_recv() {
                    queue.push_back(more);
                }
                match cmd {
                    TelephonyCmd::Dial(num) => {
                        penumbra_services::audio::stop_ringtone_loop();
                        let res = telephony::dial(&num);
                        if let Err(e) = res {
                            let w2 = weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(win) = w2.upgrade() {
                                    win.set_status_banner(e.into());
                                }
                            });
                        }
                        sync_and_push(&weak, &mut was_in_call);
                    }
                    TelephonyCmd::Accept(path) => {
                        penumbra_services::audio::stop_ringtone_loop();
                        if let Err(e) = telephony::accept_call(&path) {
                            let w2 = weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(win) = w2.upgrade() {
                                    win.set_status_banner(e.into());
                                }
                            });
                        }
                        sync_and_push(&weak, &mut was_in_call);
                    }
                    TelephonyCmd::Hangup(path) => {
                        penumbra_services::audio::stop_ringtone_loop();
                        if let Err(e) = telephony::hangup_call(&path) {
                            let w2 = weak.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(win) = w2.upgrade() {
                                    win.set_status_banner(e.into());
                                }
                            });
                        }
                        sync_and_push(&weak, &mut was_in_call);
                    }
                    TelephonyCmd::ToggleSpeaker(enabled) => {
                        telephony::set_speakerphone(enabled);
                    }
                    TelephonyCmd::ToggleMic(muted) => {
                        telephony::set_mic_muted(muted);
                    }
                    TelephonyCmd::SetVolume(mut pct) => {
                        let mut kept = std::collections::VecDeque::new();
                        while let Some(next) = queue.pop_front() {
                            if let TelephonyCmd::SetVolume(p) = next {
                                pct = p;
                            } else {
                                kept.push_back(next);
                            }
                        }
                        queue = kept;
                        telephony::set_call_volume(pct);
                    }
                    TelephonyCmd::SendDtmf(path, digit) => {
                        let _ = telephony::send_dtmf(&path, &digit);
                    }
                    TelephonyCmd::SyncCalls => {
                        queue.retain(|next| !matches!(next, TelephonyCmd::SyncCalls));
                        sync_and_push(&weak, &mut was_in_call);
                    }
                }
            }
        });
    }

    if let Some(num) = args.dial {
        ui.set_dial_number(num.clone().into());
        if let Some(c) = contacts::resolve_number(&num) {
            ui.set_matched_contact_name(c.name.into());
        }
        let _ = cmd_tx.send(TelephonyCmd::Dial(num));
    }

    {
        let weak = ui.as_weak();
        ui.on_dial_number_edited(move |num| {
            if let Some(w) = weak.upgrade() {
                let matched = contacts::resolve_number(&num)
                    .map(|c| c.name)
                    .unwrap_or_default();
                w.set_matched_contact_name(matched.into());
            }
        });
    }

    {
        let weak = ui.as_weak();
        let tx = cmd_tx.clone();
        ui.on_dial_pressed(move |num| {
            info!("Dialing {num}...");
            if let Some(w) = weak.upgrade() {
                w.set_status_banner("".into());
            }
            let _ = tx.send(TelephonyCmd::Dial(num.into()));
        });
    }

    {
        let tx = cmd_tx.clone();
        ui.on_accept_call(move |path| {
            let _ = tx.send(TelephonyCmd::Accept(path.into()));
        });
    }

    {
        let tx = cmd_tx.clone();
        ui.on_hangup_call(move |path| {
            let _ = tx.send(TelephonyCmd::Hangup(path.into()));
        });
    }

    {
        let tx = cmd_tx.clone();
        ui.on_toggle_speaker(move |enabled| {
            let _ = tx.send(TelephonyCmd::ToggleSpeaker(enabled));
        });
    }

    {
        let tx = cmd_tx.clone();
        ui.on_toggle_mic(move |muted| {
            let _ = tx.send(TelephonyCmd::ToggleMic(muted));
        });
    }

    {
        let tx = cmd_tx.clone();
        ui.on_call_volume_changed(move |pct| {
            let _ = tx.send(TelephonyCmd::SetVolume(pct));
        });
    }

    {
        let weak = ui.as_weak();
        let tx = cmd_tx.clone();
        ui.on_send_dtmf(move |path, digit| {
            if path == "__BACKSPACE__" {
                if let Some(w) = weak.upgrade() {
                    let mut s = w.get_dial_number().to_string();
                    s.pop();
                    let matched = contacts::resolve_number(&s)
                        .map(|c| c.name)
                        .unwrap_or_default();
                    w.set_matched_contact_name(matched.into());
                    w.set_dial_number(s.into());
                }
                return;
            }
            let _ = tx.send(TelephonyCmd::SendDtmf(path.into(), digit.into()));
        });
    }

    ui.on_open_contacts(|| {
        let _ = Command::new("penumbra-contacts").spawn();
    });

    ui.on_message_number(|num| {
        let _ = Command::new("penumbra-messaging").args(["--to", &num]).spawn();
    });

    ui.on_add_contact_number(|num| {
        let _ = Command::new("penumbra-contacts")
            .args(["--add-number", &num])
            .spawn();
    });

    // Event-driven ModemManager1 D-Bus listener for instant incoming/state updates.
    {
        let tx = cmd_tx.clone();
        std::thread::spawn(move || {
            telephony::watch_call_events(move || {
                let _ = tx.send(TelephonyCmd::SyncCalls);
            });
        });
    }

    // 1-second timer: updates `00:00` clock + SyncCalls fallback while in call,
    // and live-reloads accent theme if Settings updates `penumbra-theme.json`.
    let duration_timer = Timer::default();
    {
        let weak = ui.as_weak();
        let active_start_tick = std::sync::Arc::clone(&active_start);
        let tx = cmd_tx.clone();
        let theme_path = theme::MonetPalette::default_ipc_path();
        let mut last_theme_mtime = std::fs::metadata(&theme_path).and_then(|m| m.modified()).ok();
        duration_timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
            if let Some(w) = weak.upgrade() {
                let mtime = std::fs::metadata(&theme_path).and_then(|m| m.modified()).ok();
                if mtime != last_theme_mtime {
                    last_theme_mtime = mtime;
                    apply_palette(&w);
                }
                if w.get_has_call() {
                    if let Ok(guard) = active_start_tick.lock() {
                        if let Some((_, start)) = *guard {
                            let elapsed = telephony::now_epoch_secs().saturating_sub(start);
                            w.set_call_duration_text(
                                format!("{:02}:{:02}", elapsed / 60, elapsed % 60).into(),
                            );
                        }
                    }
                    let _ = tx.send(TelephonyCmd::SyncCalls);
                }
            }
        });
    }

    ui.run()?;
    penumbra_services::audio::stop_ringtone_loop();
    Ok(())
}
