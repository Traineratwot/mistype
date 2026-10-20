import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.components as PlasmaComponents
import org.kde.kirigami as Kirigami
import org.kde.plasma.plasma5support as P5Support

// SwitchFix LLM — panel widget: request history for «LLM-fix selection».
//
// Polls `switchfix llm-history` (JSON array, newest first).
// compactRepresentation = status LED + busy spinner.
// fullRepresentation    = popup: status + history with copy buttons.

PlasmoidItem {
    id: root

    property var history: []
    readonly property int count: history.length

    function shortText(s, n) {
        s = String(s === undefined || s === null ? "" : s).replace(/\s+/g, " ").trim();
        if (s.length <= n)
            return s;
        return s.slice(0, Math.max(1, n - 1)) + "…";
    }

    toolTipMainText: i18n("SwitchFix — история LLM-запросов")
    toolTipSubText: count > 0
        ? i18n("%1 запрос(ов). Клик — открыть.", count)
        : i18n("Пока пусто. Выделите текст и нажмите Alt+R.")

    preferredRepresentation: compactRepresentation

    // --- data polling -------------------------------------------------
    P5Support.DataSource {
        id: runner
        engine: "executable"
        connectedSources: []

        onNewData: function (source, data) {
            var out = data["stdout"] !== undefined ? String(data["stdout"]).trim() : "";
            disconnectSource(source);
            if (source.indexOf("llm-history") === 0) {
                try {
                    root.history = (out && out !== "[]") ? JSON.parse(out) : [];
                } catch (e) {
                    root.history = [];
                }
            }
        }
    }

    Timer {
        interval: 1500
        repeat: true
        running: true
        triggeredOnStart: true
        onTriggered: {
            if (runner.connectedSources.length === 0) {
                runner.connectSource("switchfix llm-history");
            }
        }
    }

    function copy(index, field) {
        runner.connectSource("switchfix llm-copy --index " + index + " --field " + field);
    }

    // --- panel content (compact): LED ---------------------------------
    compactRepresentation: Item {
        id: compactRoot
        Layout.preferredHeight: Kirigami.Units.iconSizes.medium
        Layout.minimumWidth: Kirigami.Units.iconSizes.medium
        Layout.preferredWidth: Kirigami.Units.iconSizes.medium

        Rectangle {
            anchors.centerIn: parent
            width: 12
            height: 12
            radius: 6
            color: root.count > 0 ? "#2ecc71" : "#888888"
            border.width: 1
            border.color: Qt.rgba(0, 0, 0, 0.35)
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
            PlasmaComponents.Label {
                Layout.fillWidth: true
                text: i18n("История LLM-запросов")
                font.bold: true
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
        ScrollView {
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
