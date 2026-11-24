//! Penumbra SMS Messaging (`penumbra-messaging`)
//! Real ModemManager1.Modem.Messaging + Shared Contacts & Calls integration.

use std::process::Command;
use std::rc::Rc;
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
    let pal = theme::MonetPalette::load_from_ipc_path(std::path::Path::new("/tmp/penumbra-theme.json"))
        .unwrap_or_else(|_| theme::MonetPalette::default_dark());
    ui.set_bg_color(hex_to_color(&pal.surface_container_lowest));
    ui.set_surface_low(hex_to_color(&pal.surface_container_low));
    ui.set_surface_high(hex_to_color(&pal.surface_container_high));
    ui.set_primary(hex_to_color(&pal.primary));
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

fn reload_threads(ui: &MessagingWindow) {
    let rows: Vec<ThreadRow> = messaging::list_threads()
        .into_iter()
        .map(|t| ThreadRow {
            peer_number: t.peer_number.into(),
            display_name: t.contact_name.into(),
            last_message: t.last_message.into(),
            time_label: format_rel(t.last_timestamp_secs).into(),
        })
        .collect();
    ui.set_threads(ModelRc::from(Rc::new(VecModel::from(rows))));
}

fn reload_thread_messages(ui: &MessagingWindow, peer: &str) {
    let name = contacts::resolve_number(peer)
        .map(|c| c.name)
        .unwrap_or_default();
    ui.set_active_peer(peer.into());
    ui.set_active_peer_name(name.into());
    let bubbles: Vec<ChatBubble> = messaging::messages_for_peer(peer)
        .into_iter()
        .map(|m| ChatBubble {
            text: m.text.into(),
            outgoing: m.outgoing,
            time_label: format_rel(m.timestamp_secs).into(),
        })
        .collect();
    ui.set_messages(ModelRc::from(Rc::new(VecModel::from(bubbles))));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();
    let args = Args::parse();

    let lang = i18n::active_lang_code();
    let _ = slint::select_bundled_translation(if lang == "en" { "" } else { &lang });

    let ui = MessagingWindow::new()?;
    apply_palette(&ui);
    reload_threads(&ui);

    if let Some(peer) = args.to {
        reload_thread_messages(&ui, &peer);
    }

    {
        let weak = ui.as_weak();
        ui.on_open_thread(move |peer| {
            if let Some(w) = weak.upgrade() {
                w.set_status_banner("".into());
                reload_thread_messages(&w, &peer);
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_back_to_threads(move || {
            if let Some(w) = weak.upgrade() {
                w.set_active_peer("".into());
                w.set_active_peer_name("".into());
                w.set_status_banner("".into());
                reload_threads(&w);
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_send_message(move |peer, text| {
            if let Some(w) = weak.upgrade() {
                w.set_status_banner("".into());
                match messaging::send_sms(&peer, &text) {
                    Ok(_) => {
                        w.set_draft_text("".into());
                        reload_thread_messages(&w, &peer);
                        reload_threads(&w);
                    }
                    Err(e) => {
                        w.set_status_banner(e.into());
                    }
                }
            }
        });
    }

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
        sync_timer.start(TimerMode::Repeated, Duration::from_secs(2), move || {
            if let Some(w) = weak.upgrade() {
                let peer = w.get_active_peer().to_string();
                if peer.is_empty() {
                    reload_threads(&w);
                } else {
                    reload_thread_messages(&w, &peer);
                }
            }
        });
    }

    ui.run()?;
    Ok(())
}
