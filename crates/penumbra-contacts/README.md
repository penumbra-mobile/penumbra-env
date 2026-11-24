# `penumbra-contacts`

Native Wayland Contacts Manager for Penumbra Mobile OS (zero GNOME/GTK runtime dependencies).

## Architecture & Subsystems

- **UI Layer (`ui/contacts_window.slint`)**: Built with Slint + Material 3 Expressive dynamic Monet theming (`MonetPalette`). Includes:
  - **Searchable Contacts List**: Live filtering by name or phone number with initials avatars and quick Call/Message pills.
  - **Contact Detail View**: Full contact card with one-tap **Call** (`penumbra-calls --dial <phone>`) and **Message** (`penumbra-messaging --to <phone>`).
  - **Add / Edit View**: Create, update, and delete contacts.
- **Contacts Store (`penumbra_services::contacts`)**:
  - Persists contacts in `~/.local/share/penumbra/contacts.json` (and supports vCard `.vcf` import).
  - Shared by `penumbra-calls`, `penumbra-messaging`, and `penumbra-shell` via E.164/suffix phone matching (`resolve_number`).

## CLI Usage

- Launch normally: `penumbra-contacts`
- Pre-fill a phone number in the Add Contact form (used by `penumbra-calls` and `penumbra-messaging`): `penumbra-contacts --add-number <NUMBER>`

