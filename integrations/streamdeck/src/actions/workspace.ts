import streamDeck, {
	action,
	type DidReceiveSettingsEvent,
	type KeyAction,
	type SendToPluginEvent,
} from "@elgato/streamdeck";
import type { JsonValue } from "@elgato/utils";
import { NOT_FOUND, seam, SeamError, type SeamState } from "../seam.js";
import { SeamAction } from "./seam-action.js";

type Settings = { id?: string; name?: string };

/** Switch to the workspace chosen in the property inspector. Lit while it's the one in use. */
@action({ UUID: "co.f3nn3x.seam.workspace" })
export class SwitchWorkspace extends SeamAction<Settings> {
	protected async render(key: KeyAction<Settings>, settings: Settings, state: SeamState): Promise<void> {
		await key.setState(settings.id && state.workspace?.id === settings.id ? 1 : 0);
		const name = await this.#currentName(settings, state);
		await key.setTitle(name);
	}

	protected async press(_key: KeyAction<Settings>, settings: Settings): Promise<void> {
		if (!settings.id && !settings.name) {
			throw new Error("no workspace chosen");
		}
		try {
			await seam.call("workspace.switch", settings.id ? { id: settings.id } : { name: settings.name });
		} catch (error) {
			// The id is gone, say after the workspace was deleted and captured again. Try its name.
			if (!(error instanceof SeamError) || error.code !== NOT_FOUND || !settings.id || !settings.name) {
				throw error;
			}
			await seam.call("workspace.switch", { name: settings.name });
		}
	}

	/** The name the key shows: Seam's current one when this desk has the workspace, else the saved one. */
	async #currentName(settings: Settings, state: SeamState): Promise<string | undefined> {
		if (state.workspace?.id === settings.id) {
			return state.workspace?.name;
		}
		const listed = (await seam.workspaces()).find((w) => w.id === settings.id);
		return listed?.name ?? settings.name;
	}

	/** Remember the name of whatever was chosen, so a key still reads right on another desk. */
	override async onDidReceiveSettings(ev: DidReceiveSettingsEvent<Settings>): Promise<void> {
		const { settings } = ev.payload;
		if (settings.id && seam.status === "online") {
			const listed = (await seam.workspaces()).find((w) => w.id === settings.id);
			if (listed && listed.name !== settings.name) {
				settings.name = listed.name;
				await ev.action.setSettings(settings);
				return;
			}
		}
		await super.onDidReceiveSettings(ev);
	}

	/** The property inspector asks for its dropdown items. */
	override async onSendToPlugin(ev: SendToPluginEvent<JsonValue, Settings>): Promise<void> {
		const payload = ev.payload as { event?: string } | null;
		if (payload?.event !== "getWorkspaces") {
			return;
		}
		const saved = await ev.action.getSettings();
		let items: { label: string; value: string; disabled?: boolean }[];
		try {
			const listed = await seam.workspaces();
			items = listed.map((w) => ({ label: w.name, value: w.id }));
			if (saved.id && !listed.some((w) => w.id === saved.id)) {
				items.push({ label: saved.name ?? saved.id, value: saved.id });
			}
			if (items.length === 0) {
				items.push({ label: "No workspaces on this desk", value: "", disabled: true });
			}
		} catch {
			items = [{ label: "Seam isn't running", value: "", disabled: true }];
		}
		await streamDeck.ui.sendToPropertyInspector({ event: "getWorkspaces", items });
	}
}
