# Penumbra Translations (GNU gettext `.po` + Slint `@tr()`)

Penumbra uses the standard Linux **GNU gettext** `.po` format, bundled directly into Slint UI binaries at compile time.

- **Default fallback:** English (`en`) is the source language. Any missing language or untranslated `msgstr ""` automatically falls back to English.
- **Directory layout:**
  ```text
  po/
    <lang>/
      LC_MESSAGES/
        penumbra-shell.po
        penumbra-settings.po
  ```
- **Adding a new language (e.g. `fr`):**
  1. Copy `po/ro/LC_MESSAGES/penumbra-shell.po` and `po/ro/LC_MESSAGES/penumbra-settings.po` to `po/fr/LC_MESSAGES/`.
  2. Update `"Language: fr\n"` in the header and translate each `msgstr "..."` (you can use Poedit, Lokalize, Weblate, or any text editor).
  3. Rebuild (`cargo build`), and add the language entry to `SUPPORTED_LANGUAGES` in `crates/penumbra-services/src/i18n.rs` + `page_about.slint` if you want it selectable in Settings.
- **Updating templates after adding `@tr(...)` strings:**
  Run `./scripts/extract-translations.sh` (uses `xgettext` / `msgmerge`).

