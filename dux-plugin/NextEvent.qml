import QtQuick
import Quickshell
import qs.Common
import qs.Services
import qs.DCommon.Widgets
import qs.Modules.Plugins

// The current or next timed event today, from the events Antarctical serves Dux over the
// dankcal socket (CalendarService); this widget never talks to the socket itself.
PluginComponent {
	id: root

	// The Nix package puts Antarctical's store path here; elsewhere it is found on PATH.
	readonly property string antarcticalBin: "@antarctical@".startsWith("@") ? "antarctical" : "@antarctical@"

	// Join is offered from this long before an event starts until it ends.
	readonly property int joinLeadMs: 5 * 60 * 1000

	property var event: null
	property string label: ""
	// Only the video call: `url` is the event's Google Calendar page, which every event has.
	readonly property string joinUrl: event ? (event.meetingUrl || "") : ""
	property bool joinable: false

	function eligible(ev) {
		if (!ev || ev.allDay || !ev.start || !ev.end)
			return false;
		const id = ev.id || "";
		if (id.startsWith("task_") || id.startsWith("vtodo_"))
			return false;
		// Antarctical lists declined events under "<calendar>#declined" when they are shown.
		return !(ev.calendarId || "").endsWith("#declined");
	}

	// An event about to start wins (so it can be joined), then the latest one in progress,
	// then the next one.
	function pick(events, now) {
		let soon = null, ongoing = null, next = null;
		for (const ev of events) {
			if (!eligible(ev) || ev.end.getTime() <= now)
				continue;
			const start = ev.start.getTime();
			if (start <= now) {
				if (!ongoing || start > ongoing.start.getTime())
					ongoing = ev;
			} else {
				if (start - now <= root.joinLeadMs && (!soon || start < soon.start.getTime()))
					soon = ev;
				if (!next || start < next.start.getTime())
					next = ev;
			}
		}
		return soon || ongoing || next;
	}

	function refresh() {
		const now = new Date();
		const ev = pick(CalendarService.getEventsForDate(now) || [], now.getTime());
		root.event = ev;
		if (ev) {
			const until = ev.start.getTime() - now.getTime();
			if (until <= 0)
				root.label = "now";
			else if (until < 60 * 60 * 1000)
				root.label = "in " + Math.ceil(until / 60000) + "m";
			else
				root.label = Qt.formatTime(ev.start, SettingsData.use24HourClock ? "HH:mm" : "h:mm AP");
			root.joinable = until <= root.joinLeadMs && root.joinUrl !== "";
		} else {
			root.label = "";
			root.joinable = false;
		}
		setVisibilityOverride(ev !== null);
	}

	function join() {
		if (root.joinUrl)
			Quickshell.execDetached(["xdg-open", root.joinUrl]);
	}

	pillClickAction: () => Quickshell.execDetached([root.antarcticalBin])

	Component.onCompleted: refresh()

	Connections {
		target: CalendarService
		function onEventsByDateChanged() {
			root.refresh();
		}
	}

	Timer {
		interval: 30000
		running: true
		repeat: true
		onTriggered: root.refresh()
	}

	horizontalBarPill: Component {
		Row {
			spacing: Theme.spacingXS

			Rectangle {
				width: Math.round(root.textSize * 0.6)
				height: width
				radius: width / 2
				color: root.event ? root.event.color : Theme.primary
				anchors.verticalCenter: parent.verticalCenter
			}

			StyledText {
				text: root.label
				font.pixelSize: root.textSize
				font.weight: Font.DemiBold
				color: Theme.surfaceText
				anchors.verticalCenter: parent.verticalCenter
			}

			StyledText {
				text: root.event ? root.event.title : ""
				width: Math.min(implicitWidth, 220)
				elide: Text.ElideRight
				font.pixelSize: root.textSize
				color: Theme.surfaceText
				anchors.verticalCenter: parent.verticalCenter
			}

			DIcon {
				visible: root.joinable
				name: "videocam"
				size: root.iconSize
				color: Theme.primary
				anchors.verticalCenter: parent.verticalCenter

				// Above the pill's own click handler, which opens Antarctical.
				MouseArea {
					anchors.fill: parent
					anchors.margins: -Theme.spacingXS
					cursorShape: Qt.PointingHandCursor
					onClicked: root.join()
				}
			}
		}
	}

	verticalBarPill: Component {
		Column {
			spacing: Theme.spacingXS

			Rectangle {
				width: Math.round(root.textSize * 0.6)
				height: width
				radius: width / 2
				color: root.event ? root.event.color : Theme.primary
				anchors.horizontalCenter: parent.horizontalCenter
			}

			StyledText {
				text: root.label
				font.pixelSize: Math.round(root.textSize * 0.8)
				color: Theme.surfaceText
				anchors.horizontalCenter: parent.horizontalCenter
			}

			DIcon {
				visible: root.joinable
				name: "videocam"
				size: root.iconSize
				color: Theme.primary
				anchors.horizontalCenter: parent.horizontalCenter

				MouseArea {
					anchors.fill: parent
					cursorShape: Qt.PointingHandCursor
					onClicked: root.join()
				}
			}
		}
	}
}
