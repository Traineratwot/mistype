# Plasma 6 Widget (Plasmoid) Reference

Official sources (KF6 / Plasma 6):

- Tutorial: https://develop.kde.org/docs/plasma/widget/
- Setup: https://develop.kde.org/docs/plasma/widget/setup/
- Porting to KF6: https://develop.kde.org/docs/plasma/widget/porting_kf6/
- QML API: https://develop.kde.org/docs/plasma/widget/plasma-qml-api/
- Widget properties: https://develop.kde.org/docs/plasma/widget/properties/
- Testing: https://develop.kde.org/docs/plasma/widget/testing/
- API index: https://api.kde.org/plasma-index.html
- libplasma sources: https://invent.kde.org/plasma/libplasma

SwitchFix widget lives in `packaging/plasmoid/org.switchfix.suggestions/`.
Install: `packaging/install.sh` copies it to `~/.local/share/plasma/plasmoids/`.

---

## 1. Package layout

```
org.switchfix.suggestions/
  metadata.json
  contents/
    ui/
      main.qml
    config/          # optional: config.qml + main.xml
```

User widgets: `~/.local/share/plasma/plasmoids/<Id>/`
System widgets: `/usr/share/plasma/plasmoids/<Id>/`

### metadata.json (Plasma 6)

```json
{
    "KPlugin": {
        "Authors": [{ "Name": "…", "Email": "…" }],
        "Category": "System Information",
        "Description": "…",
        "Icon": "preferences-desktop-spellcheck",
        "Id": "org.switchfix.suggestions",
        "Name": "SwitchFix Suggestions",
        "Version": "0.1.0"
    },
    "X-Plasma-API-Minimum-Version": "6.0",
    "KPackageStructure": "Plasma/Applet"
}
```

**Required for Plasma 6:**

| Key | Value |
|-----|--------|
| `KPackageStructure` | `"Plasma/Applet"` |
| `X-Plasma-API-Minimum-Version` | `"6.0"` |

Without `X-Plasma-API-Minimum-Version: 6.0` the plasmoid is treated as Plasma 5-only
and will not appear in the widget list.

---

## 2. Root element: `PlasmoidItem`

In Plasma 6 the root of `main.qml` **must** be `PlasmoidItem` (QML name for
`AppletInterface`). Containments use `ContainmentItem`.

```qml
import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.components as PlasmaComponents
import org.kde.kirigami as Kirigami

PlasmoidItem {
    id: root
    // …
}
```

### Properties on `PlasmoidItem` (NOT on `Plasmoid.*`)

These were on the `Plasmoid` attached object in Plasma 5. In Plasma 6 they
live on the root `PlasmoidItem`:

| Property | Meaning |
|----------|---------|
| `compactRepresentation` | panel / small view |
| `fullRepresentation` | popup / expanded view |
| `preferredRepresentation` | force which view is preferred |
| `switchWidth` / `switchHeight` | min size before switching to full |
| `activationTogglesExpanded` | shortcut toggles popup |
| `toolTipMainText` / `toolTipSubText` / `toolTipTextFormat` / `toolTipItem` | tooltip |
| `hideOnWindowDeactivate` | `false` = pin popup open |

```qml
PlasmoidItem {
    preferredRepresentation: compactRepresentation

    compactRepresentation: Item { /* panel strip */ }
    fullRepresentation: ColumnLayout { /* popup */ }

    toolTipMainText: i18n("My widget")
    toolTipSubText: i18n("Status line")
}
```

### Properties that stay on `Plasmoid` / `plasmoid`

```qml
Plasmoid.icon: "my-icon"
Plasmoid.title: i18n("My Widget")
// or lowercase context property:
plasmoid.expanded
plasmoid.location
plasmoid.formFactor
plasmoid.configuration
```

`plasmoid` (lowercase) is a global context property after
`import org.kde.plasma.plasmoid`.

---

## 3. Landmines (hit by SwitchFix)

| Mistake | Error / symptom | Fix |
|---------|-----------------|-----|
| `Plasmoid.fullRepresentation: …` | `Cannot assign to non-existent property "fullRepresentation"` | Use root: `fullRepresentation: …` |
| `Plasmoid.compactRepresentation: …` | same class of error | Use root: `compactRepresentation: …` |
| `font: Kirigami.Theme.smallFont` then `font.italic: true` | `Property has already been assigned a value` | Set only sub-properties: `font.family`, `font.pixelSize`, `font.italic` |
| `fullRepresentation:` twice (item + `Plasmoid.`) | assigned twice | Assign once on root |
| `ToolButton { tooltip: "…" }` | `Cannot assign to non-existent property "tooltip"` | `import QtQuick.Controls as QQC2` then `QQC2.ToolTip.visible: hovered` + `QQC2.ToolTip.text: …` |
| Versioned imports (`import QtQuick 2.15`) | subtle bugs in Qt6 | Un-version: `import QtQuick` |
| PlasmaComponents 2 (`import org.kde.plasma.components 2.0`) | removed in Plasma 6 | Use `org.kde.plasma.components` (PlasmaComponents 3 = QQC2) |
| Root is `Item` / `MouseArea` | won't load as Plasma 6 applet | Root must be `PlasmoidItem` |

### Tooltip pattern (QQC2)

`PlasmaComponents.ToolButton` inherits Qt Quick Controls 2 **without** a
`tooltip` string property. Use the attached `QQC2.ToolTip`:

```qml
import QtQuick.Controls as QQC2

PlasmaComponents.ToolButton {
    icon.name: "dialog-close"
    QQC2.ToolTip.visible: hovered
    QQC2.ToolTip.text: i18n("Close")
    onClicked: root.dismiss()
}
```

Widget-level tooltip (the hover tip over the panel icon) uses
`toolTipMainText` / `toolTipSubText` on `PlasmoidItem`.

---

## 4. Representations

### compactRepresentation

Shown in the panel. Default draws `plasmoid.icon` and toggles `expanded`.

If you replace it, add a `MouseArea` to open the popup:

```qml
compactRepresentation: Item {
    Layout.preferredHeight: Kirigami.Units.iconSizes.medium
    // content…
    MouseArea {
        anchors.fill: parent
        onClicked: root.expanded = !root.expanded
    }
}
```

### fullRepresentation

Popup (or inline on the desktop if there is room). Size via:

```qml
fullRepresentation: ColumnLayout {
    Layout.minimumWidth: 300
    Layout.preferredWidth: 340
    Layout.minimumHeight: 120
}
```

If the panel is large enough (`switchWidth`/`switchHeight`), full
representation can be shown **instead of** compact. Force compact in the panel:

```qml
preferredRepresentation: compactRepresentation
```

Pin the popup (don't close on focus loss):

```qml
hideOnWindowDeactivate: false
```

---

## 5. PlasmaComponents 3 (QQC2 wrappers)

```qml
import org.kde.plasma.components as PlasmaComponents
```

Same API as [Qt Quick Controls 2](https://doc.qt.io/qt-6/qtquickcontrols-index.html),
styled for Plasma. Common types:

- `PlasmaComponents.Label`
- `PlasmaComponents.Button` / `ToolButton`
- `PlasmaComponents.CheckBox`, `RadioButton`, `ComboBox`
- `PlasmaComponents.TextField`, `TextArea`
- `PlasmaComponents.Slider`, `SpinBox`, `ScrollView`, `BusyIndicator`

Plasma-specific QML sources:
https://invent.kde.org/plasma/libplasma/-/tree/master/src/declarativeimports/plasmacomponents

Extras (Heading, Paragraph):

```qml
import org.kde.plasma.extras as PlasmaExtras
```

Units / theme (prefer Kirigami in new code):

```qml
import org.kde.kirigami as Kirigami
Kirigami.Units.iconSizes.medium
Kirigami.Units.smallSpacing
Kirigami.Theme.textColor
Kirigami.Theme.negativeTextColor / positiveTextColor / neutralTextColor
```

---

## 6. Running shell commands from a widget

`Plasma5Support.DataSource` + `executable` engine (still the supported way in
Plasma 6 for simple polling):

```qml
import org.kde.plasma.plasma5support as P5Support

P5Support.DataSource {
    id: runner
    engine: "executable"
    connectedSources: []
    onNewData: function (source, data) {
        var out = String(data["stdout"] || "").trim();
        disconnectSource(source);
        // parse out…
    }
}

// fire and forget:
runner.connectSource("switchfix suggest-peek");
```

Notes:

- Always `disconnectSource(source)` in `onNewData` or sources pile up.
- Guard with `connectedSources.length === 0` when polling on a timer.
- Prefer absolute path to the binary if it may be outside `PATH`
  (SwitchFix installs to `/usr/local/bin/switchfix`).

---

## 7. i18n

```qml
i18n("Hello")
i18n("Hello %1", name)
i18n("%n suggestion(s)", "", count)   // plural
```

UI strings in SwitchFix widgets are **Russian** (AGENTS.md rule 9).

---

## 8. Install / reload / debug

```bash
# install (from repo)
./packaging/install.sh
# or manually:
cp -r packaging/plasmoid/org.switchfix.suggestions \
      ~/.local/share/plasma/plasmoids/

# reload Plasma shell (pick up QML changes)
systemctl --user restart plasma-plasmashell.service
# or: plasmashell --replace &

# test the widget in a standalone window
plasmawindowed org.switchfix.suggestions

# watch QML errors
journalctl --user -f | grep -i -E 'plasmoid|qml|switchfix'
```

After editing `main.qml`, restart plasmashell **or** remove/re-add the widget.
QML is not always hot-reloaded.

---

## 9. SwitchFix data contract

Widget polls `switchfix suggest-peek` every ~400 ms. Output JSON:

```json
{
  "daemon": true,
  "enabled": true,
  "original": "приветт",
  "slots": [
    { "text": "привет", "kind": "SpellFix", "source_word": "приветт" }
  ],
  "can_add_to_dict": true,
  "timeout_ms": 4000,
  "llm": {
    "configured": true,
    "name": "llm",
    "ok": true,
    "busy": false,
    "calls": 12,
    "errors": 1,
    "last_ms": 340
  },
  "stats": {
    "replaced": 4,
    "undone": 1,
    "hints": 2,
    "vetoes": 0,
    "via": { "layout": 1, "dict": 1, "edit1": 1, "context": 1 },
    "history": [
      { "t_ms": 12000, "action": "replace", "from": "приветт", "to": "привет", "via": "context" }
    ]
  }
}
```

| Field | Meaning |
|-------|---------|
| `daemon` | main `switchfix daemon` is alive (heartbeat + `/proc/<pid>`) |
| `enabled` | not paused |
| `slots` | active suggestions (panel list) |
| `can_add_to_dict` | show «+ в словарь» |
| `llm.busy` | context backend (LLM / LT) is correcting right now → widget spinner |
| `stats.history` | last transforms (`replace` / `undo` / `hint` / `veto`), oldest first |

Commands written by the widget:

| Command | Effect |
|---------|--------|
| `switchfix suggest-accept N` | accept slot N (1-based) |
| `switchfix suggest-learn` | add word to `user.dic` |
| `switchfix suggest-dismiss` | hide tips |
| `switchfix toggle` | pause / resume |

Heartbeat: `/tmp/switchfix.daemon.json` (`{pid, enabled}`), written by the
daemon, removed on clean exit.

---

## 10. Porting checklist (Plasma 5 → 6)

1. Root `Item` → `PlasmoidItem`.
2. Move `compactRepresentation` / `fullRepresentation` /
   `preferredRepresentation` / `toolTip*` / `hideOnWindowDeactivate`
   from `Plasmoid.*` to root item properties.
3. Keep `Plasmoid.icon`, `Plasmoid.title`, `plasmoid.configuration` etc.
4. JSON metadata + `X-Plasma-API-Minimum-Version: 6.0`.
5. Un-version QML imports.
6. PlasmaComponents 2 → 3 (or QQC2 + `import QtQuick.Controls as QQC2`).
7. `PlasmaCore.DataSource` → `Plasma5Support.DataSource`.
8. `PlasmaCore.SortFilterModel` → `KItemModels.KSortFilterProxyModel`.
9. Verify with `plasmawindowed <Id>` and `journalctl --user`.
