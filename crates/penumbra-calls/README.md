# `penumbra-calls`

Native Wayland Phone & Voice Dialer for Penumbra Mobile OS (zero GNOME/GTK runtime dependencies).

## Architecture & Subsystems

- **UI Layer (`ui/calls_window.slint`)**: Built with Slint + Material 3 Expressive dynamic Monet theming (`MonetPalette`). Includes:
  - **Keypad**: Touch dialpad with quick actions (`Message`, `Save contact`).
  - **Recents**: Persistent call history (`~/.local/share/penumbra/call_history.json`) with contact name resolution.
  - **Active / Incoming Call Overlay**: Full-screen call controls (`Answer`, `Decline`, `End call`, `Speaker`, `Mute`, in-call `DTMF` keypad, live duration timer).
- **Telephony Backend (`penumbra_services::telephony`)**:
  - Connects directly to `org.freedesktop.ModemManager1` (`Modem.Voice` & `Call`) over system D-Bus (`CreateCall`, `Start`, `Accept`, `Hangup`, `SendDtmf`, `ListCalls`).
  - Routes voice call audio via `org.mobian_project.CallAudio` (`SelectMode`, `EnableSpeaker`, `MuteMic`) over session D-Bus.
- **Incoming Call Auto-Launch**: `penumbra-shell` monitors `ModemManager1` for `RingingIn` calls and launches `penumbra-calls` automatically.

## CLI Usage

- Launch normally: `penumbra-calls`
- Dial a number directly (used by `penumbra-contacts` and `penumbra-messaging`): `penumbra-calls --dial <NUMBER>`

