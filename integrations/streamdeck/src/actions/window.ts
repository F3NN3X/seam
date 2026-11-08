import { action, type KeyAction } from "@elgato/streamdeck";
import { seam } from "../seam.js";
import { SeamAction } from "./seam-action.js";

type Direction = "left" | "right" | "up" | "down";
type Settings = { dir?: Direction };

const DIRECTIONS: Direction[] = ["left", "right", "up", "down"];

/** The direction chosen in the property inspector, or none yet. */
function directionOf(settings: Settings): Direction | undefined {
	return DIRECTIONS.find((d) => d === settings.dir);
}

/** Acts on the focused window in the direction chosen in the property inspector. */
abstract class Directional extends SeamAction<Settings> {
	protected abstract readonly method: string;
	protected abstract readonly verb: string;

	protected async render(key: KeyAction<Settings>, settings: Settings): Promise<void> {
		const dir = directionOf(settings);
		await key.setTitle(dir ? `${this.verb}\n${dir}` : undefined);
	}

	protected async press(_key: KeyAction<Settings>, settings: Settings): Promise<void> {
		const dir = directionOf(settings);
		if (!dir) {
			throw new Error("no direction chosen");
		}
		await seam.call(this.method, { dir });
	}
}

@action({ UUID: "co.f3nn3x.seam.window-move" })
export class MoveWindow extends Directional {
	protected readonly method = "window.move";
	protected readonly verb = "Move";
}

@action({ UUID: "co.f3nn3x.seam.window-focus" })
export class FocusWindow extends Directional {
	protected readonly method = "window.focus";
	protected readonly verb = "Focus";
}
