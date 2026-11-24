//! Penumbra Contacts (`penumbra-contacts`)
//! Shared Contact Book interconnected with `penumbra-calls` and `penumbra-messaging`.

use std::process::Command;
use std::rc::Rc;

use clap::Parser;
use slint::{Color, ComponentHandle, ModelRc, VecModel};

use penumbra_services::{contacts, i18n, theme};

slint::include_modules!();

#[derive(Parser, Debug)]
#[command(name = "penumbra-contacts", about = "Penumbra Mobile Contacts")]
struct Args {
    /// Optional phone number to pre-fill in New Contact screen
    #[arg(long)]
    add_number: Option<String>,
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

fn apply_palette(ui: &ContactsWindow) {
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

fn reload_contacts(ui: &ContactsWindow, query: &str) {
    let q = query.trim().to_lowercase();
    let mut list = contacts::load_contacts();
    list.sort_by(|a, b| {
        b.favorite
            .cmp(&a.favorite)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let rows: Vec<ContactRow> = list
        .into_iter()
        .filter(|c| {
            q.is_empty()
                || c.name.to_lowercase().contains(&q)
                || c.phone.to_lowercase().contains(&q)
                || c.email.to_lowercase().contains(&q)
        })
        .map(|c| {
            let initials = c.initials();
            ContactRow {
                id: c.id.into(),
                name: c.name.into(),
                phone: c.phone.into(),
                email: c.email.into(),
                notes: c.notes.into(),
                initials: initials.into(),
                favorite: c.favorite,
            }
        })
        .collect();
    ui.set_contacts(ModelRc::from(Rc::new(VecModel::from(rows))));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).init();
    let args = Args::parse();

    let lang = i18n::active_lang_code();
    let _ = slint::select_bundled_translation(if lang == "en" { "" } else { &lang });

    let ui = ContactsWindow::new()?;
    apply_palette(&ui);
    reload_contacts(&ui, "");

    if let Some(num) = args.add_number {
        ui.set_sel_id("".into());
        ui.set_sel_name("".into());
        ui.set_sel_phone(num.into());
        ui.set_sel_email("".into());
        ui.set_sel_notes("".into());
        ui.set_mode("edit".into());
    }

    {
        let weak = ui.as_weak();
        ui.on_search_changed(move |q| {
            if let Some(w) = weak.upgrade() {
                reload_contacts(&w, &q);
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_save_contact(move |id, name, phone, email, notes| {
            let id_s: String = id.into();
            let existing_fav = if !id_s.is_empty() {
                contacts::load_contacts()
                    .into_iter()
                    .find(|c| c.id == id_s)
                    .map(|c| c.favorite)
                    .unwrap_or(false)
            } else {
                false
            };
            let contact = contacts::Contact {
                id: id_s,
                name: if name.trim().is_empty() {
                    phone.to_string()
                } else {
                    name.trim().to_string()
                },
                phone: phone.trim().into(),
                email: email.trim().into(),
                notes: notes.trim().into(),
                favorite: existing_fav,
            };
            let saved_initials = contact.initials();
            let saved_name = contact.name.clone();
            let saved_phone = contact.phone.clone();
            let saved_email = contact.email.clone();
            let saved_notes = contact.notes.clone();
            let _ = contacts::upsert_contact(contact);
            if let Some(w) = weak.upgrade() {
                w.set_sel_name(saved_name.into());
                w.set_sel_phone(saved_phone.into());
                w.set_sel_email(saved_email.into());
                w.set_sel_notes(saved_notes.into());
                w.set_sel_initials(saved_initials.into());
                w.set_sel_favorite(existing_fav);
                let q = w.get_search_query().to_string();
                reload_contacts(&w, &q);
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_toggle_favorite(move |id| {
            let id_s: String = id.into();
            let mut new_fav = false;
            if let Some(mut c) = contacts::load_contacts().into_iter().find(|c| c.id == id_s) {
                c.favorite = !c.favorite;
                new_fav = c.favorite;
                let _ = contacts::upsert_contact(c);
            }
            if let Some(w) = weak.upgrade() {
                w.set_sel_favorite(new_fav);
                let q = w.get_search_query().to_string();
                reload_contacts(&w, &q);
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_delete_contact(move |id| {
            let _ = contacts::delete_contact(&id);
            if let Some(w) = weak.upgrade() {
                let q = w.get_search_query().to_string();
                reload_contacts(&w, &q);
            }
        });
    }

    ui.on_call_number(|num| {
        let _ = Command::new("penumbra-calls").args(["--dial", &num]).spawn();
    });

    ui.on_message_number(|num| {
        let _ = Command::new("penumbra-messaging").args(["--to", &num]).spawn();
    });

    ui.run()?;
    Ok(())
}
