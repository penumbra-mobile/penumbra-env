//! Shared Penumbra Contacts service (`~/.local/share/penumbra/contacts.json`)
//! with E.164/suffix phone number normalization and vCard (`.vcf`) import/export.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Contact {
    pub id: String,
    pub name: String,
    pub phone: String,
    pub email: String,
    pub notes: String,
    pub favorite: bool,
}

impl Contact {
    pub fn initials(&self) -> String {
        let mut out = String::new();
        for part in self.name.split_whitespace().take(2) {
            if let Some(ch) = part.chars().next() {
                out.push(ch.to_ascii_uppercase());
            }
        }
        if out.is_empty() {
            "?".to_string()
        } else {
            out
        }
    }
}

pub fn contacts_file_path() -> PathBuf {
    if let Ok(data) = std::env::var("XDG_DATA_HOME") {
        PathBuf::from(data).join("penumbra").join("contacts.json")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("penumbra")
            .join("contacts.json")
    } else {
        PathBuf::from("/tmp/penumbra-contacts.json")
    }
}

/// Strips formatting characters (` `, `-`, `(`, `)`) keeping leading `+` and digits.
pub fn normalize_phone(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for (idx, ch) in raw.trim().chars().enumerate() {
        if ch.is_ascii_digit() || (ch == '+' && idx == 0) {
            out.push(ch);
        }
    }
    out
}

/// Compares two phone numbers tolerating country-code prefixes (matches last 9 digits when >= 9 digits).
pub fn phone_matches(a: &str, b: &str) -> bool {
    let na: String = a.chars().filter(|c| c.is_ascii_digit()).collect();
    let nb: String = b.chars().filter(|c| c.is_ascii_digit()).collect();
    if na.is_empty() || nb.is_empty() {
        return false;
    }
    if na == nb {
        return true;
    }
    if na.len() >= 9 && nb.len() >= 9 {
        return na[na.len() - 9..] == nb[nb.len() - 9..];
    }
    false
}

pub fn load_contacts() -> Vec<Contact> {
    let path = contacts_file_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(mut list) = serde_json::from_str::<Vec<Contact>>(&raw) {
            list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            return list;
        }
    }
    Vec::new()
}

pub fn save_contacts(contacts: &[Contact]) -> std::io::Result<()> {
    let path = contacts_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(contacts)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    fs::write(path, json)
}

pub fn upsert_contact(mut contact: Contact) -> std::io::Result<Vec<Contact>> {
    let mut list = load_contacts();
    if contact.id.trim().is_empty() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        contact.id = format!("c-{ts}");
    }
    if let Some(existing) = list.iter_mut().find(|c| c.id == contact.id) {
        *existing = contact;
    } else {
        list.push(contact);
    }
    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    save_contacts(&list)?;
    Ok(list)
}

pub fn delete_contact(id: &str) -> std::io::Result<Vec<Contact>> {
    let mut list = load_contacts();
    list.retain(|c| c.id != id);
    save_contacts(&list)?;
    Ok(list)
}

/// Resolves a phone number to a matching `Contact`, if one exists.
pub fn resolve_number(number: &str) -> Option<Contact> {
    load_contacts()
        .into_iter()
        .find(|c| phone_matches(&c.phone, number))
}

/// Imports basic vCard 3.0/4.0 (`BEGIN:VCARD` ... `END:VCARD`) entries.
pub fn import_vcard(vcf_text: &str) -> std::io::Result<usize> {
    let mut list = load_contacts();
    let mut cur_name = String::new();
    let mut cur_phone = String::new();
    let mut cur_email = String::new();
    let mut in_card = false;
    let mut imported = 0usize;

    for raw_line in vcf_text.lines() {
        let line = raw_line.trim();
        if line.eq_ignore_ascii_case("BEGIN:VCARD") {
            in_card = true;
            cur_name.clear();
            cur_phone.clear();
            cur_email.clear();
        } else if line.eq_ignore_ascii_case("END:VCARD") && in_card {
            in_card = false;
            if !cur_name.is_empty() || !cur_phone.is_empty() {
                let id = format!(
                    "vcf-{}-{}",
                    imported,
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0)
                );
                list.push(Contact {
                    id,
                    name: if cur_name.is_empty() {
                        cur_phone.clone()
                    } else {
                        cur_name.clone()
                    },
                    phone: cur_phone.clone(),
                    email: cur_email.clone(),
                    notes: String::new(),
                    favorite: false,
                });
                imported += 1;
            }
        } else if in_card {
            if let Some((key, val)) = line.split_once(':') {
                let upper = key.to_ascii_uppercase();
                if upper == "FN" {
                    cur_name = val.trim().to_string();
                } else if upper.starts_with("TEL") && cur_phone.is_empty() {
                    cur_phone = val.trim().to_string();
                } else if upper.starts_with("EMAIL") && cur_email.is_empty() {
                    cur_email = val.trim().to_string();
                }
            }
        }
    }
    if imported > 0 {
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        save_contacts(&list)?;
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phone_matching_e164_suffix() {
        assert!(phone_matches("+40 722 123 456", "0722123456"));
        assert!(phone_matches("0722-123-456", "0722123456"));
        assert!(!phone_matches("0722123456", "0733999888"));
    }
}

