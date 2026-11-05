import QtQuick
import Quickshell
import Quickshell.Io
import qs.Common
import qs.Widgets
import qs.Modules.Plugins
import qs.DCommon.Common as DCommon

PluginComponent {
    id: root

    property string binaryPath: Quickshell.env("HOME") + "/.local/bin/mokka-clipboard"
    property var entries: []
    property string errorMessage: ""
    property bool favoritesOnly: false

    function refreshHistory() {
        var process = listProcessComponent.createObject(root);

        if (process)
            process.running = true;
    }

    Component.onCompleted: root.refreshHistory()

    DCommon.DSocket {
        path: Quickshell.env("XDG_RUNTIME_DIR")
            + "/mokka-clipboard-events.sock"
        connected: true

        onConnectionStateChanged: {
            if (linkUp)
                root.refreshHistory();
        }

        parser: SplitParser {
            onRead: line => {
                if (line.trim() === "refresh")
                    root.refreshHistory();
            }
        }
    }

    function restoreEntry(entryId) {
        var process = restoreProcessComponent.createObject(root, {
            entryId: entryId
        });

        if (process)
            process.running = true;
    }

    function toggleFavorite(entryId, isFavorite) {
        var process = favoriteProcessComponent.createObject(root, {
            entryId: entryId,
            favorite: !isFavorite
        });

        if (process)
            process.running = true;
    }

    function updateFavorite(entryId, favorite) {
        var updated = root.entries.slice();

        for (var i = 0; i < updated.length; i++) {
            if (updated[i].id === entryId) {
                updated[i].is_favorite = favorite;
                break;
            }
        }

        root.entries = updated;
        root.errorMessage = "";
    }

    Component {
        id: listProcessComponent

        Process {
            command: [root.binaryPath, root.favoritesOnly ? "--favorites" : "--list-json"]

            stdout: StdioCollector {
                onStreamFinished: {
                    try {
                        root.entries = JSON.parse(text);
                        root.errorMessage = "";
                    } catch (error) {
                        root.errorMessage = "Não foi possível ler o histórico: " + error;
                    }
                }
            }

            stderr: StdioCollector {
                onStreamFinished: {
                    if (text.trim())
                        root.errorMessage = text.trim();
                }
            }

            onExited: destroy()
        }
    }

    Component {
        id: restoreProcessComponent

        Process {
            property int entryId: 0

            command: [
                root.binaryPath,
                "--restore",
                String(entryId)
            ]

            onExited: exitCode => {
                if (exitCode !== 0)
                    root.errorMessage = "Falha ao restaurar o item " + entryId;

                destroy();
            }
        }
    }

    Component {
        id: favoriteProcessComponent

        Process {
            property int entryId: 0
            property bool favorite: false

            command: [
                root.binaryPath,
                favorite ? "--favorite" : "--unfavorite",
                String(entryId)
            ]

            onExited: exitCode => {
                if (exitCode === 0)
                    root.updateFavorite(entryId, favorite);
                else
                    root.errorMessage = "Não foi possível atualizar o favorito.";

                destroy();
            }
        }
    }

    horizontalBarPill: Component {
        StyledRect {
            implicitWidth: label.implicitWidth + Theme.spacingM * 2
            implicitHeight: parent.widgetThickness
            width: implicitWidth
            height: implicitHeight
            radius: Theme.cornerRadius
            color: Theme.chipSurface

            StyledText {
                id: label
                anchors.centerIn: parent
                text: "▤ Mokka"
                color: Theme.surfaceText
                font.pixelSize: Theme.fontSizeMedium
            }
        }
    }

    verticalBarPill: Component {
        StyledRect {
            implicitWidth: parent.widgetThickness
            implicitHeight: label.implicitHeight + Theme.spacingM * 2
            width: implicitWidth
            height: implicitHeight
            radius: Theme.cornerRadius
            color: Theme.chipSurface

            StyledText {
                id: label
                anchors.centerIn: parent
                text: "▤"
                color: Theme.surfaceText
                font.pixelSize: Theme.fontSizeLarge
            }
        }
    }

    popoutContent: Component {
        PopoutComponent {
            id: popout

            headerText: "Mokka Clipboard"
            detailsText: "Seu histórico da área de transferência"

            Item {
                width: parent.width
                implicitHeight: Math.max(
                    160,
                    root.popoutHeight
                        - popout.headerHeight
                        - popout.detailsHeight
                        - Theme.spacingXL
                )

                Row {
                    id: filterBar
                    anchors.top: parent.top
                    anchors.horizontalCenter: parent.horizontalCenter
                    height: 34
                    spacing: Theme.spacingM

                    MouseArea {
                        width: 145
                        height: 32
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            root.favoritesOnly = false
                            root.refreshHistory()
                        }

                        StyledRect {
                            anchors.fill: parent
                            radius: Theme.cornerRadius
                            color: !root.favoritesOnly
                                ? Theme.chipSurface
                                : Theme.cardSurface
                        }

                        StyledText {
                            anchors.centerIn: parent
                            text: "Histórico"
                            color: Theme.surfaceText
                            font.pixelSize: Theme.fontSizeMedium
                        }
                    }

                    MouseArea {
                        width: 145
                        height: 32
                        cursorShape: Qt.PointingHandCursor
                        onClicked: {
                            root.favoritesOnly = true
                            root.refreshHistory()
                        }

                        StyledRect {
                            anchors.fill: parent
                            radius: Theme.cornerRadius
                            color: root.favoritesOnly
                                ? Theme.chipSurface
                                : Theme.cardSurface
                        }

                        StyledText {
                            anchors.centerIn: parent
                            text: "★ Favoritos"
                            color: Theme.surfaceText
                            font.pixelSize: Theme.fontSizeMedium
                        }
                    }
                }

                ListView {
                    id: historyList

                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: filterBar.bottom
                    anchors.topMargin: Theme.spacingS
                    anchors.bottom: parent.bottom
                    clip: true
                    spacing: Theme.spacingXS
                    model: root.favoritesOnly
                        ? root.entries.filter(function(entry) {
                            return entry.is_favorite;
                        })
                        : root.entries
                    boundsBehavior: Flickable.StopAtBounds

                    delegate: Item {
                        width: historyList.width
                        height: 76
                        clip: true

                        StyledRect {
                            anchors.fill: parent
                            radius: Theme.cornerRadius
                            color: Theme.cardSurface
                        }

                        Row {
                            id: entryRow
                            anchors.fill: parent
                            anchors.leftMargin: Theme.spacingM
                            anchors.rightMargin: Theme.spacingM
                            anchors.topMargin: Theme.spacingS
                            anchors.bottomMargin: Theme.spacingS
                            spacing: Theme.spacingM

                            Image {
                                id: thumbnail
                                width: 76
                                height: 58
                                visible: modelData.kind === "image"
                                    && !!modelData.thumbnail_path
                                source: visible
                                    ? modelData.thumbnail_path
                                    : ""
                                fillMode: Image.PreserveAspectFit
                                asynchronous: true
                                cache: false
                            }

                            Column {
                                anchors.verticalCenter: parent.verticalCenter
                                width: parent.width
                                    - (thumbnail.visible
                                        ? thumbnail.width + entryRow.spacing
                                        : 0)
                                    - 40
                                spacing: Theme.spacingXS

                                StyledText {
                                    width: parent.width
                                    text: modelData.kind === "image"
                                        ? "Imagem #" + modelData.id
                                        : String(modelData.preview || "")
                                            .replace(/\s+/g, " ").trim()
                                    color: Theme.surfaceText
                                    font.pixelSize: Theme.fontSizeMedium
                                    wrapMode: Text.NoWrap
                                    elide: Text.ElideRight
                                }

                                StyledText {
                                    width: parent.width
                                    text: (modelData.is_favorite ? "★ Favorito" : "Histórico")
                                        + "  ·  #" + modelData.id
                                    color: Theme.outline
                                    font.pixelSize: Theme.fontSizeSmall
                                    wrapMode: Text.NoWrap
                                    elide: Text.ElideRight
                                }
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            anchors.rightMargin: 42
                            cursorShape: Qt.PointingHandCursor
                            onClicked: root.restoreEntry(modelData.id)
                        }

                        MouseArea {
                            anchors.right: parent.right
                            anchors.rightMargin: Theme.spacingS
                            anchors.verticalCenter: parent.verticalCenter
                            width: 36
                            height: 36
                            cursorShape: Qt.PointingHandCursor
                            onClicked: root.toggleFavorite(
                                modelData.id,
                                modelData.is_favorite
                            )

                            StyledText {
                                anchors.centerIn: parent
                                text: modelData.is_favorite ? "★" : "☆"
                                color: Theme.surfaceText
                                font.pixelSize: 25
                            }
                        }
                    }
                }

                StyledText {
                    anchors.centerIn: historyList
                    width: parent.width - Theme.spacingL * 2
                    visible: root.errorMessage.length > 0
                        || historyList.count === 0
                    text: root.errorMessage.length > 0
                        ? root.errorMessage
                        : root.favoritesOnly
                            ? "Nenhum favorito ainda."
                            : "Nenhum item no histórico."
                    color: root.errorMessage.length > 0
                        ? Theme.error
                        : Theme.surfaceText
                    font.pixelSize: Theme.fontSizeMedium
                    wrapMode: Text.WordWrap
                    horizontalAlignment: Text.AlignHCenter
                }
            }
        }
    }

    popoutWidth: 560
    popoutHeight: 620
}
