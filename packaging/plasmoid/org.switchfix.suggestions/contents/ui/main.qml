import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.components as PlasmaComponents
import org.kde.kirigami as Kirigami
import org.kde.plasma.plasma5support as P5Support

// SwitchFix Suggestions — panel widget (no floating window).
//
// Polls `switchfix suggest-peek` (JSON: daemon, enabled, slots, …).
// compactRepresentation = status LED + live candidate chips.
// fullRepresentation    = popup: status + full suggestion list.

PlasmoidItem {
    id: root

    // Combined status from `switchfix suggest-peek`.
    property var status: ({})
    readonly property bool daemonRunning: !!(status && status.daemon)
    readonly property bool enabled: !!(status && status.enabled)
    readonly property string original: (status && status.original) ? String(status.original) : ""
    readonly property var slots: (status && status.slots) ? status.slots : []
    readonly property int tipCount: slots.length
    readonly property bool hasTips: tipCount > 0
    readonly property bool canAdd: !!(status && status.can_add_to_dict)
    readonly property var llm: (status && status.llm) ? status.llm : ({})
    readonly property var stats: (status && status.stats) ? status.stats : ({})
    readonly property bool llmConfigured: !!(llm && llm.configured)
    readonly property string llmName: (llm && llm.name) ? String(llm.name) : ""
    readonly property var llmOk: (llm && llm.ok !== undefined) ? llm.ok : null
    readonly property int llmCalls: (llm && llm.calls) ? Number(llm.calls) : 0
    readonly property int llmErrors: (llm && llm.errors) ? Number(llm.errors) : 0
    readonly property int stReplaced: (stats && stats.replaced) ? Number(stats.replaced) : 0
    readonly property int stUndone: (stats && stats.undone) ? Number(stats.undone) : 0
    readonly property int stHints: (stats && stats.hints) ? Number(stats.hints) : 0
    readonly property string viaSummary: {
        if (!stats || !stats.via)
            return "";
        var v = stats.via;
        return "layout " + (v.layout || 0) + " · dict " + (v.dict || 0)
             + " · edit1 " + (v.edit1 || 0) + " · llm " + (v.context || 0);
    }

    toolTipMainText: daemonRunning
        ? i18n("SwitchFix — %1", enabled ? i18n("работает") : i18n("пауза"))
        : i18n("SwitchFix — демон не запущен")
    toolTipSubText: {
        var llmLine = llmConfigured
            ? i18n("LLM: %1 (%2 выз., %3 ошиб.)", llmName || "on", llmCalls, llmErrors)
            : i18n("LLM: выключен");
        if (hasTips)
            return i18n("«%1» → %2\n%3", original, slots.map(function (s) { return s.text; }).join(", "), llmLine);
        return llmLine + "\n" + i18n("Замен: %1 · откатов: %2 · подсказок: %3", stReplaced, stUndone, stHints);
    }

    preferredRepresentation: compactRepresentation

    // --- data polling -------------------------------------------------
    P5Support.DataSource {
        id: runner
        engine: "executable"
        connectedSources: []

        onNewData: function (source, data) {
            var out = data["stdout"] !== undefined ? String(data["stdout"]).trim() : "";
            disconnectSource(source);
            try {
                root.status = (out && out !== "{}") ? JSON.parse(out) : {};
            } catch (e) {
                root.status = {};
            }
        }
    }

    Timer {
        interval: 400
        repeat: true
        running: true
        triggeredOnStart: true
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

    // --- panel content (compact): LED + candidate chips ---------------
    compactRepresentation: Item {
        id: compactRoot
        Layout.preferredHeight: Kirigami.Units.iconSizes.medium
        Layout.minimumWidth: Kirigami.Units.iconSizes.medium
        Layout.preferredWidth: root.hasTips
            ? led.width + row.implicitWidth + Kirigami.Units.smallSpacing * 3
            : Kirigami.Units.iconSizes.medium

        readonly property color ledColor: !root.daemonRunning
            ? "#e74c3c"   // red — daemon down
            : (!root.enabled ? "#f0c000"   // yellow — paused
                             : "#2ecc71")  // green — running

        RowLayout {
            id: row
            anchors.fill: parent
            spacing: Kirigami.Units.smallSpacing

            // LED: click opens the full list (popup).
            Rectangle {
                id: led
                width: 12
                height: 12
                radius: 6
                color: compactRoot.ledColor
                border.width: 1
                border.color: Qt.rgba(0, 0, 0, 0.35)
                Layout.alignment: Qt.AlignVCenter
                opacity: root.hasTips ? 1.0 : 0.92
                scale: root.hasTips ? 1.1 : 1.0
                Behavior on scale { NumberAnimation { duration: 120 } }

                MouseArea {
                    anchors.fill: parent
                    anchors.margins: -6 // easier hit target
                    onClicked: root.expanded = !root.expanded
                }
            }

            // LLM backend status chip (tiny).
            Rectangle {
                visible: root.llmConfigured
                width: llmLabel.implicitWidth + 6
                height: 12
                radius: 3
                color: root.llmOk === false ? "#e74c3c" : (root.llmOk === true ? "#2ecc71" : "#888888")
                Layout.alignment: Qt.AlignVCenter
                PlasmaComponents.Label {
                    id: llmLabel
                    anchors.centerIn: parent
                    text: root.llmName === "llm" ? "LLM" : (root.llmName || "ctx")
                    font.pointSize: 7
                    color: "white"
                }
            }

            // Live candidates — click applies immediately (no popup).
            Repeater {
                model: root.hasTips ? root.slots : []
                delegate: PlasmaComponents.Button {
                    required property var modelData
                    required property int index
                    text: (index + 1) + " " + (modelData && modelData.text ? modelData.text : "")
                    flat: true
                    Layout.maximumWidth: 11 * Kirigami.Units.gridUnit
                    onClicked: root.accept(index + 1)
                }
            }
        }

        // Click on empty compact area (no tips) also opens the list.
        MouseArea {
            visible: !root.hasTips
            anchors.fill: parent
            onClicked: root.expanded = !root.expanded
        }
    }

    // --- popup (expanded / click) -------------------------------------
    fullRepresentation: ColumnLayout {
        Layout.minimumWidth: 300
        Layout.preferredWidth: 340
        spacing: Kirigami.Units.smallSpacing

        // Daemon status banner
        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            Rectangle {
                width: 10
                height: 10
                radius: 5
                color: !root.daemonRunning ? Kirigami.Theme.negativeTextColor
                     : (!root.enabled ? Kirigami.Theme.neutralTextColor
                                      : Kirigami.Theme.positiveTextColor)
            }
            PlasmaComponents.Label {
                Layout.fillWidth: true
                text: root.daemonRunning
                    ? (root.enabled ? i18n("SwitchFix работает") : i18n("SwitchFix на паузе"))
                    : i18n("SwitchFix не запущен")
                font.bold: true
                color: root.daemonRunning ? Kirigami.Theme.textColor : Kirigami.Theme.negativeTextColor
            }
            PlasmaComponents.ToolButton {
                visible: !root.daemonRunning
                text: i18n("Запустить")
                icon.name: "system-run"
                onClicked: runner.connectSource("switchfix daemon >/dev/null 2>&1 &")
            }
            PlasmaComponents.ToolButton {
                visible: root.daemonRunning
                text: root.enabled ? i18n("Пауза") : i18n("Включить")
                onClicked: runner.connectSource("switchfix toggle")
            }
        }

        PlasmaComponents.Label {
            visible: !root.daemonRunning
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            opacity: 0.8
            text: i18n("Демон отвечает за хоткеи и подсказки. Запустите его вручную:\nswitchfix daemon")
        }

        Kirigami.Separator {
            Layout.fillWidth: true
        }

        // Active suggestions
        PlasmaComponents.Label {
            Layout.fillWidth: true
            text: root.hasTips
                ? i18n("Исправить: «%1»", root.original)
                : i18n("Активных подсказок нет")
            font.bold: true
            opacity: root.hasTips ? 1.0 : 0.7
        }

        Repeater {
            model: root.hasTips ? root.slots : []
            delegate: PlasmaComponents.Button {
                required property var modelData
                required property int index
                Layout.fillWidth: true
                text: (index + 1) + "   " + (modelData && modelData.text ? modelData.text : "")
                onClicked: {
                    root.accept(index + 1);
                    root.expanded = false;
                }
            }
        }

        PlasmaComponents.Label {
            visible: !root.hasTips
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            opacity: 0.65
            text: i18n("Печатайте с опечаткой или не в той раскладке — список появится здесь.")
        }

        Kirigami.Separator {
            Layout.fillWidth: true
        }

        // LLM / hybrid diagnostics
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            PlasmaComponents.Label {
                Layout.fillWidth: true
                text: root.llmConfigured
                    ? i18n("Движок: %1 — %2", root.llmName || "context",
                           root.llmOk === false ? i18n("ошибка") : i18n("работает"))
                    : i18n("Движок: словарь / edit-1 (LLM выключен)")
                font.bold: true
                opacity: 0.9
            }
            PlasmaComponents.Label {
                visible: root.llmConfigured
                Layout.fillWidth: true
                opacity: 0.7
                text: i18n("LLM вызовов: %1 · ошибок: %2 · %3 мс",
                           root.llmCalls, root.llmErrors, (root.llm && root.llm.last_ms) || 0)
            }
            PlasmaComponents.Label {
                Layout.fillWidth: true
                opacity: 0.7
                text: i18n("Замен: %1 · откатов: %2 · подсказок: %3",
                           root.stReplaced, root.stUndone, root.stHints)
            }
            PlasmaComponents.Label {
                visible: root.viaSummary !== ""
                Layout.fillWidth: true
                opacity: 0.65
                text: root.viaSummary
            }
            PlasmaComponents.Label {
                opacity: 0.55
                font.pointSize: 7
                text: i18n("История: switchfix spell-stats")
            }
        }

        RowLayout {
            Layout.alignment: Qt.AlignRight
            visible: root.hasTips
            PlasmaComponents.Button {
                visible: root.canAdd
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
