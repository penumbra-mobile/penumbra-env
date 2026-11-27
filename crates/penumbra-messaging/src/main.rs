//! Penumbra SMS Messaging (`penumbra-messaging`)
//! ModemManager1.Modem.Messaging + Shared Contacts & Calls integration.

use std::collections::VecDeque;
use std::io::Write;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use clap::Parser;
use slint::{Color, ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use penumbra_services::{contacts, i18n, messaging, telephony, theme};

slint::include_modules!();

#[derive(Parser, Debug)]
#[command(name = "penumbra-messaging", about = "Penumbra Mobile SMS Messaging")]
struct Args {
    /// Optional recipient phone number to open immediately
    #[arg(long)]
    to: Option<String>,
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

fn apply_palette(ui: &MessagingWindow) {
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

fn format_rel(ts: u64) -> String {
    let diff = telephony::now_epoch_secs().saturating_sub(ts);
    if diff < 60 {
        "Now".to_string()
    } else if diff < 3600 {
        format!("{}m", diff / 60)
    } else if diff < 86400 {
        format!("{}h", diff / 3600)
    } else {
        format!("{}d", diff / 86400)
    }
}

fn contact_initials_for(name: &str) -> String {
    if name.trim().is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = name.split_whitespace().collect();
    match parts.as_slice() {
        [] => String::new(),
        [one] => one.chars().take(2).collect::<String>().to_uppercase(),
        [first, last, ..] => {
            let a = first.chars().next().unwrap_or('?');
            let b = last.chars().next().unwrap_or('?');
            format!("{a}{b}").to_uppercase()
        }
    }
}

fn apply_threads_to_ui(ui: &MessagingWindow, threads: &[messaging::SmsThread]) {
    let q = ui.get_thread_search().trim().to_lowercase();
    let rows: Vec<ThreadRow> = threads
        .iter()
        .filter(|t| {
            q.is_empty()
                || t.contact_name.to_lowercase().contains(&q)
                || t.peer_number.to_lowercase().contains(&q)
                || t.last_message.to_lowercase().contains(&q)
        })
        .map(|t| {
            let initials = contact_initials_for(&t.contact_name);
            ThreadRow {
                peer_number: t.peer_number.clone().into(),
                display_name: t.contact_name.clone().into(),
                initials: initials.into(),
                last_message: t.last_message.clone().into(),
                time_label: format_rel(t.last_timestamp_secs).into(),
                unread_count: t.unread_count as i32,
            }
        })
        .collect();
    ui.set_threads(ModelRc::from(Rc::new(VecModel::from(rows))));
}

fn apply_messages_to_ui(
    ui: &MessagingWindow,
    peer: &str,
    peer_name: &str,
    msgs: Vec<messaging::SmsMessage>,
) {
    ui.set_active_peer(peer.into());
    ui.set_active_peer_name(peer_name.into());
    let bubbles: Vec<ChatBubble> = msgs
        .into_iter()
        .map(|m| ChatBubble {
            id: m.id.into(),
            text: m.text.into(),
            outgoing: m.outgoing,
            time_label: format_rel(m.timestamp_secs).into(),
        })
        .collect();
    ui.set_messages(ModelRc::from(Rc::new(VecModel::from(bubbles))));
}

fn reload_contact_suggestions(ui: &MessagingWindow, query: &str) {
    let q = query.trim().to_lowercase();
    let rows: Vec<ContactPickRow> = contacts::load_contacts()
        .into_iter()
        .filter(|c| {
            q.is_empty()
                || c.name.to_lowercase().contains(&q)
                || c.phone.to_lowercase().contains(&q)
        })
        .map(|c| {
            let initials = c.initials();
            ContactPickRow {
                name: c.name.into(),
                phone: c.phone.into(),
                initials: initials.into(),
            }
        })
        .collect();
    ui.set_contact_suggestions(ModelRc::from(Rc::new(VecModel::from(rows))));
}

enum MsgCmd {
    SyncThreads,
    SyncThread(String),
    SendSms(String, String),
    DeleteMessage(String, String),
    DeleteThread(String),
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();
    let args = Args::parse();

    let lang = i18n::active_lang_code();
    let _ = slint::select_bundled_translation(if lang == "en" { "" } else { &lang });

    let ui = MessagingWindow::new()?;
    apply_palette(&ui);
    let cached_threads = Rc::new(std::cell::RefCell::new(messaging::list_threads()));
    apply_threads_to_ui(&ui, &cached_threads.borrow());
    reload_contact_suggestions(&ui, "");

    if let Some(peer) = args.to {
        messaging::mark_thread_read(&peer);
        let name = contacts::resolve_number(&peer)
            .map(|c| c.name)
            .unwrap_or_default();
        let msgs = messaging::messages_for_peer(&peer);
        apply_messages_to_ui(&ui, &peer, &name, msgs);
    }

    // Background worker for D-Bus SMS send & ModemManager sync (keeps UI responsive).
    let (cmd_tx, cmd_rx) = mpsc::channel::<MsgCmd>();
    {
        let weak = ui.as_weak();
        let threads_cache = Rc::clone(&cached_threads);
        // Send updated thread lists back onto the Slint event loop cleanly.
        let _ = &threads_cache;
        std::thread::spawn(move || {
            let mut queue = VecDeque::new();
            while let Some(cmd) = queue.pop_front().or_else(|| cmd_rx.recv().ok()) {
                while let Ok(more) = cmd_rx.try_recv() {
                    queue.push_back(more);
                }
                match cmd {
                    MsgCmd::SyncThreads => {
                        queue.retain(|c| !matches!(c, MsgCmd::SyncThreads));
                        let threads = messaging::list_threads();
                        let w = weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(win) = w.upgrade() {
                                apply_threads_to_ui(&win, &threads);
                            }
                        });
                    }
                    MsgCmd::SyncThread(peer) => {
                        queue.retain(|c| !matches!(c, MsgCmd::SyncThread(_)));
                        messaging::mark_thread_read(&peer);
                        let name = contacts::resolve_number(&peer)
                            .map(|c| c.name)
                            .unwrap_or_default();
                        let msgs = messaging::messages_for_peer(&peer);
                        let w = weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(win) = w.upgrade() {
                                if win.get_active_peer().as_str() == peer {
                                    apply_messages_to_ui(&win, &peer, &name, msgs);
                                }
                            }
                        });
                    }
                    MsgCmd::SendSms(peer, text) => {
                        let res = messaging::send_sms(&peer, &text);
                        let name = contacts::resolve_number(&peer)
                            .map(|c| c.name)
                            .unwrap_or_default();
                        let msgs = messaging::messages_for_peer(&peer);
                        let threads = messaging::list_threads();
                        let w = weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(win) = w.upgrade() {
                                win.set_sending(false);
                                match res {
                                    Ok(_) => {
                                        win.set_draft_text("".into());
                                        win.set_status_banner("".into());
                                    }
                                    Err(e) => {
                                        win.set_status_banner(e.into());
                                    }
                                }
                                apply_messages_to_ui(&win, &peer, &name, msgs);
                                apply_threads_to_ui(&win, &threads);
                            }
                        });
                    }
                    MsgCmd::DeleteMessage(peer, id) => {
                        messaging::delete_message(&id);
                        let name = contacts::resolve_number(&peer)
                            .map(|c| c.name)
                            .unwrap_or_default();
                        let msgs = messaging::messages_for_peer(&peer);
                        let threads = messaging::list_threads();
                        let w = weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(win) = w.upgrade() {
                                if win.get_active_peer().as_str() == peer {
                                    apply_messages_to_ui(&win, &peer, &name, msgs);
                                }
                                apply_threads_to_ui(&win, &threads);
                            }
                        });
                    }
                    MsgCmd::DeleteThread(peer) => {
                        messaging::delete_thread(&peer);
                        let threads = messaging::list_threads();
                        let w = weak.clone();
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(win) = w.upgrade() {
                                win.set_active_peer("".into());
                                win.set_active_peer_name("".into());
                                win.set_status_banner("".into());
                                apply_threads_to_ui(&win, &threads);
                            }
                        });
                    }
                }
            }
        });
    }

    {
        let weak = ui.as_weak();
        let tx = cmd_tx.clone();
        ui.on_open_thread(move |peer| {
            if let Some(w) = weak.upgrade() {
                w.set_status_banner("".into());
                w.set_active_peer(peer.clone());
                let name = contacts::resolve_number(&peer)
                    .map(|c| c.name)
                    .unwrap_or_default();
                w.set_active_peer_name(name.into());
            }
            let _ = tx.send(MsgCmd::SyncThread(peer.into()));
        });
    }

    {
        let weak = ui.as_weak();
        let tx = cmd_tx.clone();
        ui.on_back_to_threads(move || {
            if let Some(w) = weak.upgrade() {
                w.set_active_peer("".into());
                w.set_active_peer_name("".into());
                w.set_status_banner("".into());
            }
            let _ = tx.send(MsgCmd::SyncThreads);
        });
    }

    {
        let tx = cmd_tx.clone();
        ui.on_thread_search_changed(move |_q| {
            let _ = tx.send(MsgCmd::SyncThreads);
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_recipient_filter_changed(move |q| {
            if let Some(w) = weak.upgrade() {
                reload_contact_suggestions(&w, &q);
            }
        });
    }

    {
        let weak = ui.as_weak();
        let tx = cmd_tx.clone();
        ui.on_send_message(move |peer, text| {
            if text.trim().is_empty() {
                return;
            }
            if let Some(w) = weak.upgrade() {
                w.set_sending(true);
                w.set_status_banner("".into());
            }
            let _ = tx.send(MsgCmd::SendSms(peer.into(), text.into()));
        });
    }

    {
        let tx = cmd_tx.clone();
        ui.on_delete_message(move |peer, id| {
            let _ = tx.send(MsgCmd::DeleteMessage(peer.into(), id.into()));
        });
    }

    {
        let tx = cmd_tx.clone();
        ui.on_delete_thread(move |peer| {
            let _ = tx.send(MsgCmd::DeleteThread(peer.into()));
        });
    }

    ui.on_copy_message(|text| {
        if let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
        }
    });

    ui.on_call_number(|num| {
        let _ = Command::new("penumbra-calls").args(["--dial", &num]).spawn();
    });

    ui.on_add_contact_number(|num| {
        let _ = Command::new("penumbra-contacts")
            .args(["--add-number", &num])
            .spawn();
    });

    ui.on_open_contacts(|| {
        let _ = Command::new("penumbra-contacts").spawn();
    });

    let sync_timer = Timer::default();
    {
        let weak = ui.as_weak();
        let tx = cmd_tx.clone();
        let theme_path = theme::MonetPalette::default_ipc_path();
        let mut last_theme_mtime = std::fs::metadata(&theme_path).and_then(|m| m.modified()).ok();
        sync_timer.start(TimerMode::Repeated, Duration::from_secs(2), move || {
            if let Some(w) = weak.upgrade() {
                let mtime = std::fs::metadata(&theme_path).and_then(|m| m.modified()).ok();
                if mtime != last_theme_mtime {
                    last_theme_mtime = mtime;
                    apply_palette(&w);
                }
                let peer = w.get_active_peer().to_string();
                if peer.is_empty() {
                    let _ = tx.send(MsgCmd::SyncThreads);
                } else {
                    let _ = tx.send(MsgCmd::SyncThread(peer));
                }
            }
        });
    }

    ui.run()?;
    Ok(())
}
