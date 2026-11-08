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
