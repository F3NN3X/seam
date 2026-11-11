import { action, type KeyAction } from "@elgato/streamdeck";
import type { JsonObject } from "@elgato/utils";
import { seam, type SeamState } from "../seam.js";
import { SeamAction } from "./seam-action.js";

/** Actions that carry no settings and have nothing of their own to draw. */
abstract class Plain extends SeamAction {
	protected async render(): Promise<void> {}
}

@action({ UUID: "co.f3nn3x.seam.lineup" })
export class LineUp extends Plain {
	protected async press(): Promise<void> {
		await seam.call("lineup.start");
	}
}

@action({ UUID: "co.f3nn3x.seam.zone-next" })
export class NextZone extends Plain {
	protected async press(): Promise<void> {
		await seam.call("zone.move", { dir: "next" });
	}
}

@action({ UUID: "co.f3nn3x.seam.zone-prev" })
export class PreviousZone extends Plain {
	protected async press(): Promise<void> {
		await seam.call("zone.move", { dir: "prev" });
	}
}

/** Pins the focused window on top, or unpins it. State doesn't say which window is pinned, so the key has one look. */
@action({ UUID: "co.f3nn3x.seam.pin-top" })
export class PinOnTop extends Plain {
	protected async press(): Promise<void> {
		await seam.call("window.pin", { kind: "top" });
	}
}

/** Switches to the workspace in use again, so windows that were moved or minimized go back. */
@action({ UUID: "co.f3nn3x.seam.snap-back" })
export class SnapBack extends Plain {
	protected async press(): Promise<void> {
		await seam.call("workspace.snap_back");
	}
}

/** Switches to the workspace that was in use before this one. Pressing it again goes back. */
@action({ UUID: "co.f3nn3x.seam.switch-back" })
export class SwitchBack extends Plain {
	protected async press(): Promise<void> {
		await seam.call("workspace.switch_back");
	}
}

/** Saves the windows into the workspace in use, the way "Update from screen" does, or into a new one. */
@action({ UUID: "co.f3nn3x.seam.quick-save" })
export class SaveWindows extends Plain {
	protected async press(): Promise<void> {
		await seam.call("workspace.quick_save");
	}
}

@action({ UUID: "co.f3nn3x.seam.release" })
export class ReleasePinned extends Plain {
	protected async press(): Promise<void> {
		await seam.call("wm.release");
	}
}

/** Lit while the pointer remap is running. With no lined-up borders it counts as not running. */
@action({ UUID: "co.f3nn3x.seam.remap" })
export class Remap extends SeamAction {
	protected async render(key: KeyAction<JsonObject>, _settings: JsonObject, state: SeamState): Promise<void> {
		await key.setState(state.remap === "on" ? 1 : 0);
	}

	protected async press(_key: KeyAction<JsonObject>, _settings: JsonObject, state: SeamState): Promise<void> {
		await seam.call("remap.set", { on: state.remap !== "on" });
	}
}

@action({ UUID: "co.f3nn3x.seam.zones" })
export class Zones extends SeamAction {
	protected async render(key: KeyAction<JsonObject>, _settings: JsonObject, state: SeamState): Promise<void> {
		await key.setState(state.zones === "on" ? 1 : 0);
	}

	protected async press(_key: KeyAction<JsonObject>, _settings: JsonObject, state: SeamState): Promise<void> {
		await seam.call("zones.set", { on: state.zones !== "on" });
	}
}

/** Minimizes the other windows on the focused window's screen. Pressing it again brings them back. */
@action({ UUID: "co.f3nn3x.seam.focus-only" })
export class FocusOnThisWindow extends Plain {
	protected async press(): Promise<void> {
		await seam.call("window.focus_only");
	}
}

/** Lit while Spotlight dims everything but the focused window. */
@action({ UUID: "co.f3nn3x.seam.spotlight" })
export class Spotlight extends SeamAction {
	protected async render(key: KeyAction<JsonObject>, _settings: JsonObject, state: SeamState): Promise<void> {
		await key.setState(state.spotlight ? 1 : 0);
	}

	protected async press(_key: KeyAction<JsonObject>, _settings: JsonObject, state: SeamState): Promise<void> {
		await seam.call("window.spotlight", { on: !state.spotlight });
	}
}

/** Shows the active desk's name. Pressing it opens the Displays page. */
@action({ UUID: "co.f3nn3x.seam.desk" })
export class Desk extends SeamAction {
	protected async render(key: KeyAction<JsonObject>, _settings: JsonObject, state: SeamState): Promise<void> {
		await key.setTitle(state.desk.name || undefined);
	}

	protected async press(): Promise<void> {
		await seam.call("panel.open", { page: "displays" });
	}
}
