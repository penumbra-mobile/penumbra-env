#!/usr/bin/env bash
set -euo pipefail

# Extracts @tr("...") strings from Slint UI files into GNU gettext .pot templates
# and merges them into existing po/<lang>/LC_MESSAGES/<domain>.po files.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PO_DIR="$ROOT_DIR/po"
mkdir -p "$PO_DIR"

extract_domain() {
    local domain="$1"
    local ui_dir="$2"
    local pot_file="$PO_DIR/$domain.pot"

    python3 - "$ui_dir" "$pot_file" "$domain" <<'PY'
import os, re, sys

ui_dir, pot_file, domain = sys.argv[1], sys.argv[2], sys.argv[3]
pattern = re.compile(r'@tr\(\s*"((?:\\.|[^"\\])*)"\s*(?:,|\))')
strings = []
seen = set()

for root, _, files in os.walk(ui_dir):
    for f in sorted(files):
        if f.endswith(".slint"):
            path = os.path.join(root, f)
            with open(path, "r", encoding="utf-8") as fh:
                content = fh.read()
            for m in pattern.finditer(content):
                s = m.group(1)
                if s not in seen:
                    seen.add(s)
                    strings.append(s)

with open(pot_file, "w", encoding="utf-8") as out:
    out.write(f'''msgid ""
msgstr ""
"Project-Id-Version: {domain} 0.1.0\\n"
"MIME-Version: 1.0\\n"
"Content-Type: text/plain; charset=UTF-8\\n"
"Content-Transfer-Encoding: 8bit\\n"

''')
    for s in strings:
        out.write(f'msgid "{s}"\nmsgstr ""\n\n')
print(f"Extracted {len(strings)} strings -> {pot_file}")
PY

    for po in "$PO_DIR"/*/LC_MESSAGES/"$domain.po"; do
        if [[ -f "$po" ]] && command -v msgmerge >/dev/null 2>&1; then
            msgmerge --update --backup=none "$po" "$pot_file"
            echo "Merged $po"
        fi
    done
}

extract_domain "penumbra-shell" "$ROOT_DIR/crates/penumbra-shell/ui"
extract_domain "penumbra-desktop-shell" "$ROOT_DIR/crates/penumbra-desktop-shell/ui"
extract_domain "penumbra-settings" "$ROOT_DIR/crates/penumbra-settings/ui"

