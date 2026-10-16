import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.components as PlasmaComponents
import org.kde.plasma.core as PlasmaCore
import org.kde.kirigami as Kirigami
import org.kde.plasma.plasma5support as P5Support

// SwitchFix Suggestions — panel widget (no floating window).
//
// compactRepresentation = what sits in the panel (icon or candidate strip).
// fullRepresentation    = popup opened on click (full list).
// Polls `switchfix suggest-peek` and dispatches `switchfix suggest-*`.

PlasmoidItem {
    id: root

    property var suggestions: ({})
    readonly property bool hasTips: !!(suggestions && suggestions.slots && suggestions.slots.length > 0)
    readonly property var slots: hasTips ? suggestions.slots : []
    readonly property int tipCount: slots.length

    toolTipMainText: hasTips
        ? i18n("SwitchFix: %1", suggestions.original || "")
        : i18n("SwitchFix")
    toolTipSubText: hasTips
        ? i18n("%n suggestion(s) — click to accept", "", tipCount)
        : i18n("No suggestions")

    // Panel shows our compact representation (strip / icon), not a separate
    // full view — the popup is only for the expanded state.
    Plasmoid.preferredRepresentation: Plasmoid.compactRepresentation

    // --- data polling -------------------------------------------------
    P5Support.DataSource {
        id: runner
        engine: "executable"
        connectedSources: []

        onNewData: function (source, data) {
            var out = data["stdout"] !== undefined ? String(data["stdout"]).trim() : "";
            disconnectSource(source);
            try {
                if (!out || out === "{}") {
                    root.suggestions = {};
                } else {
                    root.suggestions = JSON.parse(out);
                }
            } catch (e) {
                root.suggestions = {};
            }
        }
    }

    Timer {
        interval: 400
        repeat: true
        running: true
        onTriggered: {
            if (runner.connectedSources.length === 0) {
                runner.connectSource("switchfix suggest-peek");
            }
        }
    }

    function accept(n) {
        runner.connectSource("switchfix suggest-accept " + n);
    }
    function learn() {
        runner.connectSource("switchfix suggest-learn");
    }
    function dismiss() {
        runner.connectSource("switchfix suggest-dismiss");
    }

    // --- panel content (compact) --------------------------------------
    Plasmoid.compactRepresentation: Item {
        id: compactRoot
        Layout.preferredHeight: Kirigami.Units.iconSizes.medium
        Layout.preferredWidth: root.hasTips
            ? strip.implicitWidth
            : Kirigami.Units.iconSizes.medium

        // Sleep-state icon
        PlasmaComponents.ToolButton {
            id: icon
            visible: !root.hasTips
            anchors.fill: parent
            icon.name: "preferences-desktop-spellcheck"
            opacity: 0.55
            onClicked: runner.connectSource("switchfix toggle")
            PlasmaCore.ToolTipArea {
                anchors.fill: parent
                mainText: root.toolTipMainText
                subText: root.toolTipSubText
            }
        }

        // Candidate strip
        RowLayout {
            id: strip
            visible: root.hasTips
            anchors.fill: parent
            spacing: Kirigami.Units.smallSpacing

            PlasmaComponents.Label {
                visible: strip.width > 280
                text: "«" + (suggestions.original || "") + "»"
                font.family: Kirigami.Theme.smallFont.family
                font.pixelSize: Kirigami.Theme.smallFont.pixelSize
                font.italic: true
                color: Kirigami.Theme.disabledTextColor
                elide: Text.ElideRight
                Layout.maximumWidth: 12 * Kirigami.Units.gridUnit
            }

            Repeater {
                model: root.tipCount
                delegate: PlasmaComponents.Button {
                    required property int index
                    text: (index + 1) + " " + (root.slots[index] ? root.slots[index].text : "")
                    flat: index > 0
                    Layout.maximumWidth: 14 * Kirigami.Units.gridUnit
                    onClicked: root.accept(index + 1)
                }
            }

            PlasmaComponents.ToolButton {
                visible: suggestions.can_add_to_dict === true && strip.width > 160
                text: strip.width > 280 ? i18n("+словарь") : ""
                icon.name: "bookmark-new"
                onClicked: root.learn()
            }

            PlasmaComponents.ToolButton {
                icon.name: "dialog-close"
                onClicked: root.dismiss()
            }
        }

        // Clicking the icon area when tips are visible opens the popup.
        MouseArea {
            visible: root.hasTips
            anchors.fill: parent
            z: -1
            onClicked: root.expanded = !root.expanded
        }
    }

    // --- popup (expanded / click) -------------------------------------
    Plasmoid.fullRepresentation: ColumnLayout {
        Layout.minimumWidth: 280
        Layout.preferredWidth: 320
        spacing: Kirigami.Units.smallSpacing

        PlasmaComponents.Label {
            text: root.hasTips
                ? i18n("Исправить: «%1»", suggestions.original || "")
                : i18n("Подсказок нет")
            font.bold: true
        }

        Repeater {
            model: root.tipCount
            delegate: PlasmaComponents.Button {
                required property int index
                Layout.fillWidth: true
                text: (index + 1) + "   " + (root.slots[index] ? root.slots[index].text : "")
                onClicked: {
                    root.accept(index + 1);
                    root.expanded = false;
                }
            }
        }

        RowLayout {
            Layout.alignment: Qt.AlignRight
            PlasmaComponents.Button {
                visible: suggestions.can_add_to_dict === true
                text: i18n("+ в словарь")
                onClicked: {
                    root.learn();
                    root.expanded = false;
                }
            }
            PlasmaComponents.Button {
                text: i18n("Пропустить")
                onClicked: {
                    root.dismiss();
                    root.expanded = false;
                }
            }
        }
    }
}
