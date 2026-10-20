import QtQuick
import QtQuick.Layouts
import QtQuick.Controls as QQC2
import org.kde.plasma.plasmoid
import org.kde.plasma.components as PlasmaComponents
import org.kde.kirigami as Kirigami
import org.kde.plasma.plasma5support as P5Support

// SwitchFix LLM — panel widget: request history for «LLM-fix selection».
//
// Polls `/tmp/switchfix.llm-history.json` (status + history) via `cat`
// so PATH / binary version cannot hide a running daemon.
// compactRepresentation = status LED + busy spinner.
// fullRepresentation    = popup: status + history with copy buttons.

PlasmoidItem {
    id: root

    property var history: []
    property bool daemonRunning: false
    property bool enabled: false
    property bool llmBusy: false
    readonly property int count: history.length

    function shortText(s, n) {
        s = String(s === undefined || s === null ? "" : s).replace(/\s+/g, " ").trim();
        if (s.length <= n)
            return s;
        return s.slice(0, Math.max(1, n - 1)) + "…";
    }

    function nowMs() {
        return Date.now();
    }

    toolTipMainText: !daemonRunning
        ? i18n("SwitchFix — демон не запущен")
        : (llmBusy ? i18n("SwitchFix — LLM думает…") : i18n("SwitchFix — история LLM-запросов"))
    toolTipSubText: !daemonRunning
        ? i18n("Запустите: switchfix daemon")
        : (llmBusy ? i18n("Запрос к API…")
                   : i18n("%1 запрос(ов). Клик — открыть.", count))

    preferredRepresentation: compactRepresentation

    // --- data polling -------------------------------------------------
    P5Support.DataSource {
        id: runner
        engine: "executable"
        connectedSources: []

        onNewData: function (source, data) {
            var out = data["stdout"] !== undefined ? String(data["stdout"]).trim() : "";
            disconnectSource(source);
            if (source.indexOf("llm-history.json") >= 0 || source.indexOf("llm-history") === 0) {
                try {
                    var obj = out ? JSON.parse(out) : {};
                    if (Array.isArray(obj)) {
                        root.history = obj;
                        root.daemonRunning = true;
                        root.llmBusy = false;
                        root.enabled = true;
                    } else {
                        root.history = obj.history || [];
                        root.enabled = !!obj.enabled;
                        root.llmBusy = !!obj.busy;
                        // Alive if a daemon wrote the file in the last 15 s.
                        var updated = Number(obj.updated || 0);
                        root.daemonRunning = updated > 0 && (root.nowMs() - updated) < 15000;
                    }
                } catch (e) {
                    root.history = [];
                    root.daemonRunning = false;
                    root.llmBusy = false;
                }
            }
        }
    }

    Timer {
        interval: 800
        repeat: true
        running: true
        triggeredOnStart: true
        onTriggered: {
            if (runner.connectedSources.length === 0) {
                // Direct file read — works even if `switchfix` is not on the
                // plasmashell PATH or is an older install.
                runner.connectSource("sh -c 'cat /tmp/switchfix.llm-history.json 2>/dev/null || echo {}'");
            }
        }
    }

    function copy(index, field) {
        // Prefer the CLI (uses the same clipboard path as the daemon).
        runner.connectSource("sh -c 'switchfix llm-copy --index " + index + " --field " + field + " 2>/dev/null || wl-copy < /tmp/switchfix.llm-history.json'");
    }

    // --- panel content (compact): LED + spinner -----------------------
    compactRepresentation: Item {
        id: compactRoot
        Layout.preferredHeight: Kirigami.Units.iconSizes.medium
        Layout.minimumWidth: Kirigami.Units.iconSizes.medium
        Layout.preferredWidth: Kirigami.Units.iconSizes.medium

        // Green = daemon running, blue = LLM busy, red = daemon down, gray = paused.
        Rectangle {
            anchors.centerIn: parent
            width: 12
            height: 12
            radius: 6
            color: !root.daemonRunning ? "#e74c3c"
                 : (root.llmBusy ? "#3498db"
                 : (root.enabled ? "#2ecc71" : "#f0c000"))
            border.width: 1
            border.color: Qt.rgba(0, 0, 0, 0.35)
            visible: !root.llmBusy
        }

        PlasmaComponents.BusyIndicator {
            anchors.centerIn: parent
            visible: root.llmBusy
            running: root.llmBusy
            width: 16
            height: 16
        }

        MouseArea {
            anchors.fill: parent
            onClicked: root.expanded = !root.expanded
        }
    }

    // --- popup (expanded / click) -------------------------------------
    fullRepresentation: ColumnLayout {
        Layout.minimumWidth: 320
        Layout.preferredWidth: 360
        spacing: Kirigami.Units.smallSpacing

        RowLayout {
            Layout.fillWidth: true
            spacing: Kirigami.Units.smallSpacing

            Rectangle {
                width: 10
                height: 10
                radius: 5
                color: !root.daemonRunning ? "#e74c3c"
                     : (root.llmBusy ? "#3498db"
                     : (root.enabled ? "#2ecc71" : "#f0c000"))
            }
            PlasmaComponents.Label {
                Layout.fillWidth: true
                font.bold: true
                text: !root.daemonRunning
                    ? i18n("Демон не запущен")
                    : (root.llmBusy ? i18n("LLM: запрос…")
                       : (root.enabled ? i18n("SwitchFix работает") : i18n("SwitchFix на паузе")))
            }
            PlasmaComponents.BusyIndicator {
                visible: root.llmBusy
                running: root.llmBusy
                width: Kirigami.Units.iconSizes.small
                height: Kirigami.Units.iconSizes.small
            }
            PlasmaComponents.Label {
                opacity: 0.6
                text: root.count > 0 ? i18n("%1", root.count) : ""
            }
        }

        PlasmaComponents.Label {
            visible: root.count === 0
            Layout.fillWidth: true
            wrapMode: Text.WordWrap
            opacity: 0.6
            text: i18n("Пока пусто. Выделите текст и нажмите Alt+R — результат появится здесь.")
        }

        // Newest first; short texts so rows fit the popup width.
        QQC2.ScrollView {
            Layout.fillWidth: true
            Layout.maximumHeight: 280
            visible: root.count > 0

            ColumnLayout {
                width: parent.width
                spacing: Kirigami.Units.smallSpacing

                Repeater {
                    model: root.history.slice(0, 12)
                    delegate: Rectangle {
                        required property var modelData
                        required property int index

                        Layout.fillWidth: true
                        implicitHeight: col.implicitHeight + Kirigami.Units.smallSpacing * 2
                        radius: 4
                        color: Kirigami.Theme.backgroundColor
                        border.color: Kirigami.Theme.disabledTextColor
                        border.width: 1
                        opacity: 0.95

                        property string fromText: modelData && modelData.from ? String(modelData.from) : ""
                        property string toText: modelData && modelData.to ? String(modelData.to) : ""
                        property bool ok: !!(modelData && modelData.ok)

                        ColumnLayout {
                            id: col
                            anchors.fill: parent
                            anchors.margins: Kirigami.Units.smallSpacing
                            spacing: 2

                            RowLayout {
                                Layout.fillWidth: true
                                PlasmaComponents.Label {
                                    text: ok ? i18n("ok") : i18n("ошибка")
                                    font.pointSize: 8
                                    color: ok ? Kirigami.Theme.positiveTextColor : Kirigami.Theme.negativeTextColor
                                    Layout.preferredWidth: Kirigami.Units.gridUnit * 2.2
                                }
                                PlasmaComponents.Label {
                                    Layout.fillWidth: true
                                    opacity: 0.75
                                    elide: Text.ElideRight
                                    font.pointSize: 8
                                    text: i18n("было: %1", root.shortText(fromText, 36))
                                }
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                PlasmaComponents.Label {
                                    Layout.preferredWidth: Kirigami.Units.gridUnit * 2.2
                                    text: "→"
                                    opacity: 0.5
                                }
                                PlasmaComponents.Label {
                                    Layout.fillWidth: true
                                    opacity: 0.9
                                    elide: Text.ElideRight
                                    font.pointSize: 8
                                    text: i18n("стало: %1", root.shortText(toText, 36))
                                }
                            }

                            RowLayout {
                                Layout.alignment: Qt.AlignRight
                                spacing: Kirigami.Units.smallSpacing

                                PlasmaComponents.Button {
                                    text: i18n("Копировать было")
                                    flat: true
                                    font.pointSize: 8
                                    onClicked: root.copy(index, "from")
                                }
                                PlasmaComponents.Button {
                                    text: i18n("Копировать стало")
                                    flat: true
                                    font.pointSize: 8
                                    onClicked: root.copy(index, "to")
                                }
                            }
                        }
                    }
                }
            }
        }

        PlasmaComponents.Label {
            visible: root.count > 12
            opacity: 0.45
            font.pointSize: 7
            text: i18n("Ещё %1 — switchfix llm-history", root.count - 12)
        }
    }
}
