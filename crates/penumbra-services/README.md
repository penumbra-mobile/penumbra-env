# `penumbra-services`

Pure Rust Linux OS & D-Bus service layer for Penumbra Mobile OS (no UI toolkit dependencies).

## Modules

- **`audio`**: Real PipeWire / PulseAudio (`pactl -f json`) backend mirroring GNOME Control Center's Sound panel (output/input devices & ports, card profiles, master volume & L/R balance, mute, per-app streams, and speaker test tones).
- **`telephony`**: `org.freedesktop.ModemManager1` (`Modem.Voice` + `Call`) and `org.mobian_project.CallAudio` (`SelectMode`, `EnableSpeaker`, `MuteMic`) voice call control and persistent call history.
- **`messaging`**: `org.freedesktop.ModemManager1` (`Modem.Messaging` + `Sms`) SMS sync/send and persistent thread storage.
- **`contacts`**: Shared JSON/vCard contact store (`~/.local/share/penumbra/contacts.json`) with E.164/suffix number matching.
- **`network` / `battery` / `sensor` / `backlight` / `display` / `mpris` / `security`**: UPower, NetworkManager, iio-sensor-proxy, sysfs backlight, multi-display convergence, MPRIS media, and lockscreen authentication.
- **`i18n` / `theme`**: Standard Linux `locale.conf` resolution and dynamic Monet wallpaper palette quantization.

