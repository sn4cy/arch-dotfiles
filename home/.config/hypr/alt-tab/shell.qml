import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import Quickshell.Hyprland
import Quickshell.Widgets

ShellRoot {
    id: root
    property var clients: []
    property var choices: []
    property int selected: 0
    property bool active: false
    property bool shown: false
    property bool refreshPending: false

    function refresh() {
        if (snapshot.running) refreshPending = true;
        else snapshot.running = true;
    }
    function step(reverse) {
        if (!active) {
            const monitor = Hyprland.focusedMonitor;
            if (!monitor || !monitor.activeWorkspace) return;
            const workspace = monitor.activeWorkspace.id;
            choices = clients.filter(c => c.mapped && !c.hidden && c.workspace.id === workspace)
                .sort((a, b) => a.focusHistoryID - b.focusHistoryID);
            if (choices.length < 2) return;
            selected = reverse ? choices.length - 1 : 1;
            active = true;
            reveal.restart();
        } else {
            selected = (selected + (reverse ? -1 : 1) + choices.length) % choices.length;
        }
    }
    function finish(cancel) {
        if (!active) return;
        const address = choices[selected].address;
        active = false;
        shown = false;
        reveal.stop();
        choices = [];
        if (!cancel && /^0x[0-9a-f]+$/i.test(address))
            Hyprland.dispatch('hl.dsp.focus({ window = "address:' + address + '" })');
    }
    function toplevel(address) {
        return ToplevelManager.toplevels.values.find(t => {
            const raw = String(t.HyprlandToplevel.address);
            return (raw.startsWith('0x') ? raw : '0x' + raw) === address;
        }) || null;
    }
    function appName(client) {
        const entry = DesktopEntries.heuristicLookup(client.class || '');
        if (entry) return entry.name;
        const name = (client.class || 'Window').split('.').pop();
        return name.charAt(0).toUpperCase() + name.slice(1);
    }
    function appIcon(client) {
        const official = {
            'com.mitchellh.ghostty': 'file:///usr/share/icons/hicolor/128x128/apps/com.mitchellh.ghostty.png',
            'zen': 'file:///opt/zen-browser-bin/browser/chrome/icons/default/default128.png',
            'firefox': 'file:///usr/share/icons/hicolor/128x128/apps/firefox.png'
        };
        if (official[client.class]) return official[client.class];
        const entry = DesktopEntries.heuristicLookup(client.class || '');
        if (!entry || !entry.icon) return Quickshell.iconPath('application-x-executable');
        return entry.icon.startsWith('/') ? 'file://' + entry.icon : Quickshell.iconPath(entry.icon);
    }
    Component.onCompleted: refresh()
    Connections {
        target: Hyprland
        function onRawEvent(event) {
            if (['activewindowv2', 'openwindow', 'closewindow', 'movewindowv2',
                 'workspacev2', 'focusedmon'].indexOf(event.name) >= 0) update.restart();
        }
    }
    Timer { id: update; interval: 15; onTriggered: root.refresh() }
    Timer { id: reveal; interval: 100; onTriggered: root.shown = root.active }
    Process {
        id: snapshot
        command: ['/usr/bin/hyprctl', '-j', 'clients']
        stdout: StdioCollector {
            onStreamFinished: {
                try { root.clients = JSON.parse(text); }
                catch (error) { console.warn('Window snapshot:', error); }
            }
        }
        onExited: {
            if (root.refreshPending) {
                root.refreshPending = false;
                update.restart();
            }
        }
    }
    SocketServer {
        active: true
        path: Quickshell.env('XDG_RUNTIME_DIR') + '/fast-alt-tab.sock'
        handler: Socket {
            id: connection
            parser: SplitParser {
                onRead: data => {
                    if (data === 'next') root.step(false);
                    else if (data === 'reverse') root.step(true);
                    else if (data === 'commit') root.finish(false);
                    else if (data === 'cancel') root.finish(true);
                    connection.write(JSON.stringify({active: root.active, selected: root.selected, choices: root.choices.map(c => c.address)}) + '\n');
                    connection.flush();
                    connection.connected = false;
                }
            }
        }
    }
    IpcHandler {
        target: 'switcher'
        function next(): void { root.step(false); }
        function reverse(): void { root.step(true); }
        function commit(): void { root.finish(false); }
        function cancel(): void { root.finish(true); }
        function status(): string { return JSON.stringify({ active: root.active, selected: root.selected, choices: root.choices.map(c => c.address), clients: root.clients.length }); }
    }
    PanelWindow {
        id: panel
        visible: root.shown
        screen: Quickshell.screens.find(s => Hyprland.focusedMonitor && s.name === Hyprland.focusedMonitor.name) || Quickshell.screens[0]
        implicitWidth: Math.min(screen.width - 64, Math.max(420, root.choices.length * 256 + 44))
        implicitHeight: 260
        color: 'transparent'
        exclusionMode: ExclusionMode.Ignore
        WlrLayershell.namespace: 'fast-alt-tab'
        WlrLayershell.layer: WlrLayer.Overlay
        WlrLayershell.keyboardFocus: WlrKeyboardFocus.None
        Rectangle {
            anchors.fill: parent
            anchors.margins: 6
            radius: 22
            border.color: '#354152'
            gradient: Gradient {
                GradientStop { position: 0; color: '#f51d2533' }
                GradientStop { position: 1; color: '#f5111722' }
            }
            Flickable {
                id: scroll
                x: 22; y: 22; width: parent.width - 44; height: 204
                clip: true
                contentWidth: cards.width
                contentHeight: height
                boundsBehavior: Flickable.StopAtBounds
                function track() { contentX = Math.max(0, Math.min(contentWidth - width, root.selected * 256 - (width - 244) / 2)); }
                Connections { target: root; function onSelectedChanged() { scroll.track(); } }
                onVisibleChanged: if (visible) Qt.callLater(track)
                Row {
                    id: cards
                    spacing: 12
                    Repeater {
                        model: root.choices
                        Rectangle {
                            required property var modelData
                            required property int index
                            property bool chosen: index === root.selected
                            width: 244
                            height: 204
                            radius: 14
                            color: chosen ? '#283b54' : '#1c2533'
                            border.width: chosen ? 2 : 1
                            border.color: chosen ? '#8bbcff' : '#344052'
                            Behavior on color { ColorAnimation { duration: 90 } }
                            Behavior on border.color { ColorAnimation { duration: 90 } }
                            Rectangle {
                                x: 10; y: 10; width: 224; height: 136
                                radius: 8; color: '#101620'
                                ScreencopyView {
                                    anchors.fill: parent; anchors.margins: 4
                                    captureSource: root.shown ? root.toplevel(modelData.address) : null
                                    constraintSize: Qt.size(448, 256)
                                    live: false
                                }
                            }
                            Image {
                                x: 12; y: 159; width: 28; height: 28
                                source: root.appIcon(modelData)
                                sourceSize: Qt.size(56, 56)
                                fillMode: Image.PreserveAspectFit
                            }
                            Text {
                                x: 49; y: 154; width: 181
                                text: modelData.title || modelData.class
                                color: '#eff5ff'; font.pixelSize: 12; font.weight: Font.DemiBold
                                elide: Text.ElideRight
                            }
                            Text {
                                x: 49; y: 174; width: 181
                                text: root.appName(modelData)
                                color: chosen ? '#b8cee9' : '#91a0b6'; font.pixelSize: 11
                                elide: Text.ElideRight
                            }
                            MouseArea {
                                anchors.fill: parent
                                onClicked: { root.selected = parent.index; root.finish(false); }
                            }
                        }
                    }
                }
            }
        }
    }
}
