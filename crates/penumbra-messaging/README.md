# `penumbra-messaging`

Native Wayland SMS Messaging Application for Penumbra Mobile OS (zero GNOME/GTK runtime dependencies).

## Architecture & Subsystems

- **UI Layer (`ui/messaging_window.slint`)**: Built with Slint + Material 3 Expressive dynamic Monet theming (`MonetPalette`). Includes:
  - **Conversations List**: Grouped by normalized phone number, resolved against `penumbra_services::contacts`.
  - **New Conversation**: Quick recipient entry and thread creation.
  - **Chat Thread View**: Incoming/outgoing message bubbles, SMS composer, one-tap **Call** (`penumbra-calls --dial <number>`) and **Save Contact** (`penumbra-contacts --add-number <number>`).
- **Messaging Backend (`penumbra_services::messaging`)**:
  - Syncs incoming/outgoing SMS from `org.freedesktop.ModemManager1` (`Modem.Messaging` & `Sms`) over system D-Bus.
  - Persists conversation history locally in `~/.local/share/penumbra/messages.json`.

## CLI Usage

- Launch normally: `penumbra-messaging`
- Open a specific thread directly (used by `penumbra-calls` and `penumbra-contacts`): `penumbra-messaging --to <NUMBER>`

